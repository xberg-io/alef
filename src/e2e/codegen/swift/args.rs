use crate::core::config::ResolvedCrateConfig;
use crate::e2e::fixture::Fixture;
use heck::ToLowerCamelCase;

use super::values::{escape_swift, from_json_helper_for_arg, is_scalar_element_type, json_to_swift};

#[allow(clippy::too_many_arguments)]
/// Build setup lines and the argument list for the function call.
///
/// Swift-bridge wrappers require strongly-typed values that don't have implicit
/// Swift literal conversions:
///
/// - `bytes` args become `RustVec<UInt8>` — fixture supplies a relative file path
///   string which is read at test time and pushed into a `RustVec<UInt8>` setup
///   variable. A literal byte array is base64-decoded or UTF-8 encoded inline.
/// - `json_object` args become opaque config/request instances — a JSON string is
///   decoded via the matching `{Type}FromJson(...)` helper in a setup line.
/// - Optional args missing from the fixture must still appear at the call site
///   as `nil` whenever a later positional arg is present, otherwise Swift slots
///   subsequent values into the wrong parameter.
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    has_host_root_route: bool,
    function_name: &str,
    options_via: Option<&str>,
    options_type: Option<&str>,
    handle_config_fn: Option<&str>,
    visitor_handle_expr: Option<&str>,
    is_method_call: bool,
    module_name: &str,
    unnamed_arg_indices: &[usize],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
    arg_name_map: Option<&std::collections::HashMap<String, String>>,
    streaming_request_type: Option<&str>,
    enums: &[crate::core::ir::EnumDef],
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<(usize, String)> = Vec::new();

    // Pre-compute, for each arg index, whether any later arg has a fixture-provided
    // value (or is required and will emit a default). When an optional arg is empty
    // but a later arg WILL emit, we must keep the slot with `nil` so positional
    // alignment is preserved.
    let later_emits: Vec<bool> = (0..args.len())
        .map(|i| {
            args.iter().skip(i + 1).any(|a| {
                let f = a.field.strip_prefix("input.").unwrap_or(&a.field);
                let v = input.get(f);
                let has_value = matches!(v, Some(x) if !x.is_null());
                has_value || !a.optional || (a.arg_type == "json_object" && a.name == "config")
            })
        })
        .collect();

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_ascii_uppercase().replace('-', "_"));
            let url_expr = if has_host_root_route {
                format!(
                    "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\"")
            };
            setup_lines.push(format!("let {} = {url_expr}", arg.name));

            // Streaming adapters with request metadata take a request DTO instead
            // of a bare mock URL. Use the adapter's request_type and this arg's
            // configured name as the DTO field label.
            if let Some(request_type) = streaming_request_type.filter(|_| idx > 0) {
                let request_label = arg_name_map
                    .and_then(|map| map.get(&arg.name).map(String::as_str))
                    .map(str::to_owned)
                    .unwrap_or_else(|| arg.name.to_lower_camel_case());
                let request_var = format!("{}Request", arg.name.to_lower_camel_case());
                setup_lines.push(format!(
                    "let {request_var} = {request_type}({request_label}: {})",
                    arg.name
                ));
                parts.push((idx, request_var));
            } else {
                parts.push((idx, arg.name.clone()));
            }
            continue;
        }

        if arg.arg_type == "handle" {
            let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_val = input.get(field);
            let has_config = config_val
                .is_some_and(|v| !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty())));
            // Swift binding's engine factory declares `createEngine(config: ConfigType?)`,
            // so calls require the `config:` argument label even when passing `nil`.
            if has_config {
                if let Some(from_json_fn) = handle_config_fn {
                    let json_str = serde_json::to_string(config_val.unwrap()).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    let config_var = format!("{}Config", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {config_var} = try {from_json_fn}(\"{escaped}\")"));
                    setup_lines.push(format!("let {var_name} = try createEngine(config: {config_var})"));
                } else {
                    setup_lines.push(format!("let {var_name} = try createEngine(config: nil)"));
                }
            } else {
                setup_lines.push(format!("let {var_name} = try createEngine(config: nil)"));
            }
            parts.push((idx, var_name));
            continue;
        }

        // bytes args: behavior depends on whether this is an e2e async wrapper (e.g. extractBytes
        // with unnamed_arg_indices) or a regular binding function. Swift's extractBytes/extractBytesSync
        // e2e wrappers take [UInt8] bytes (not path strings). When the fixture provides a path string,
        // read the file to bytes. Regular bindings also emit [UInt8] arrays from path strings.
        if arg.arg_type == "bytes" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);

            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    // Empty byte array
                    parts.push((idx, "[UInt8]()".to_string()));
                }
                Some(serde_json::Value::String(s)) => {
                    let escaped = escape_swift(s);
                    // Both unnamed and named bytes args: read file to bytes
                    let var_name = format!("{}Bytes", arg.name.to_lower_camel_case());
                    let data_var = format!("{}Data", arg.name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {data_var} = try Data(contentsOf: URL(fileURLWithPath: \"{escaped}\"))"
                    ));
                    setup_lines.push(format!("let {var_name} = Array({data_var})"));
                    parts.push((idx, var_name));
                }
                Some(serde_json::Value::Array(arr)) => {
                    // Inline byte array literal
                    let bytes: Vec<String> = arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                    parts.push((idx, format!("[UInt8]({})", bytes.join(", "))));
                }
                Some(other) => {
                    // Fallback: encode the JSON serialisation as UTF-8 bytes.
                    let json_str = serde_json::to_string(other).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    let var_name = format!("{}Bytes", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {var_name} = Array(\"{escaped}\".utf8)"));
                    parts.push((idx, var_name));
                }
            }
            continue;
        }

        // file_path args: pass path strings directly (for extract_file, extract_file_sync, etc.)
        if arg.arg_type == "file_path" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);

            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    parts.push((idx, "\"\"".to_string()));
                }
                Some(serde_json::Value::String(s)) => {
                    let escaped = escape_swift(s);
                    parts.push((idx, format!("\"{}\"", escaped)));
                }
                Some(other) => {
                    // Fallback: convert to JSON string
                    let json_str = serde_json::to_string(other).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    parts.push((idx, format!("\"{}\"", escaped)));
                }
            }
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let emission = super::stubs::emit_test_backend(trait_bridge, &methods, fixture, enums);
                    setup_lines.push(emission.setup_block);
                    parts.push((idx, emission.arg_expr));
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("swift");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push((idx, "nil".to_string()));
            continue;
        }

        // json_object "config" args: behavior depends on whether this is an e2e wrapper or regular binding.
        // E2e wrappers (all args in unnamed_arg_indices) take JSON strings and deserialize internally.
        // Regular bindings (config arg not unnamed) expect deserialized objects (via options_via or default helper).
        let is_config_arg = arg.name == "config" && arg.arg_type == "json_object";
        if is_config_arg {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            let json_str = match val {
                None | Some(serde_json::Value::Null) => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            let escaped = escape_swift(&json_str);

            // Detect if config arg is unnamed (index `idx` in unnamed_arg_indices).
            // E2e wrappers keep config unnamed and receive JSON strings.
            let config_is_unnamed = unnamed_arg_indices.contains(&idx);

            if config_is_unnamed {
                // E2e wrapper: pass JSON string directly (positional, no label).
                parts.push((idx, format!("\"{}\"", escaped)));
            } else {
                // Regular binding: deserialize to an opaque object.
                let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
                let from_json_fn = from_json_helper_for_arg(arg, options_type);
                // Qualify with module name to avoid ambiguity when both SampleCrate and RustBridge are imported.
                setup_lines.push(format!(
                    "let {var_name} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                ));
                parts.push((idx, var_name));
            }
            continue;
        }

        // json_object non-config args with array values: construct Swift data-enum objects
        // from the JSON array using the {TypeName}FromJson helper. This handles cases like
        // interact(actions: [PageAction]) where we deserialize JSON into enum instances.
        if arg.arg_type == "json_object"
            && arg.element_type.is_some()
            && !is_scalar_element_type(arg.element_type.as_deref())
        {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            let elem_type = arg.element_type.as_deref().unwrap_or("Unknown");
            // Convert element type to camelCase for the from-json helper name
            let from_json_fn = format!("{}FromJson", elem_type.to_lower_camel_case());

            match val {
                Some(serde_json::Value::Array(arr)) => {
                    let var_name = format!("{}Array", arg.name.to_lower_camel_case());

                    if arr.is_empty() {
                        // Empty array literal
                        parts.push((idx, "[]".to_string()));
                    } else {
                        // For each JSON item in the array, call the helper to deserialize it
                        let json_strs: Vec<String> =
                            arr.iter().filter_map(|item| serde_json::to_string(item).ok()).collect();

                        let mut item_vars = Vec::new();
                        for (i, json_str) in json_strs.iter().enumerate() {
                            let escaped = escape_swift(json_str);
                            let item_var = format!("_item_{var_name}_{i}");
                            // Call the wrapper-module's `{type}FromJson` helper rather than the
                            // raw `RustBridge` one so the resulting element is the
                            // wrapper-module's `PageAction` (etc.), matching the type the
                            // function signature expects. The wrapper internally delegates to
                            // `RustBridge.{type}FromJson` which understands the
                            // serde(tag = "type") format.
                            setup_lines.push(format!(
                                "let {item_var} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                            ));
                            item_vars.push(item_var);
                        }

                        // Construct the final array from all item variables
                        setup_lines.push(format!("let {var_name} = [{}]", item_vars.join(", ")));
                        parts.push((idx, var_name));
                    }
                }
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    // Required but missing — emit empty array
                    parts.push((idx, "[]".to_string()));
                }
                Some(_other) => {
                    // Non-array value — emit empty array (shouldn't happen)
                    parts.push((idx, "[]".to_string()));
                }
            }
            continue;
        }

        // json_object non-config args with options_via = "from_json":
        // Use the generated `{typeCamelCase}FromJson(_:)` helper so the fixture JSON is
        // deserialised into the opaque swift-bridge type rather than passed as a raw string.
        // When arg.field == "input", the entire fixture input IS the request object.
        // When a visitor handle is present, use `{typeCamelCase}FromJsonWithVisitor(json, handle)`
        // instead to attach the visitor to the options in one step.
        if arg.arg_type == "json_object" && options_via == Some("from_json") {
            if let Some(type_name) = options_type {
                let resolved_val = super::super::resolve_field(input, &arg.field);
                let json_str = match resolved_val {
                    serde_json::Value::Null => "{}".to_string(),
                    v => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
                };
                let escaped = escape_swift(&json_str);
                let var_name = format!("_{}", arg.name.to_lower_camel_case());
                if let Some(handle_expr) = visitor_handle_expr {
                    // Use the visitor-aware helper: `{typeCamelCase}FromJsonWithVisitor(json, handle)`.
                    // The handle expression builds a VisitorHandle from the local class instance.
                    // The function name mirrors emit_options_field_options_helper: camelCase of
                    // `{options_snake}_from_json_with_visitor`.
                    let with_visitor_fn = format!("{}FromJsonWithVisitor", type_name.to_lower_camel_case());
                    let handle_var = format!("_visitorHandle_{}", var_name.trim_start_matches('_'));
                    setup_lines.push(format!("let {handle_var} = {handle_expr}"));
                    setup_lines.push(format!(
                        "let {var_name} = try {module_name}.{with_visitor_fn}(\"{escaped}\", {handle_var})"
                    ));
                } else {
                    let from_json_fn = format!("{}FromJson", type_name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {var_name} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                    ));
                }
                parts.push((idx, var_name));
                continue;
            }
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: keep the slot with `nil`
                // when a later arg will emit, so positional alignment matches
                // the swift-bridge wrapper signature.
                if later_emits[idx] {
                    parts.push((idx, "nil".to_string()));
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push((idx, default_val));
            }
            Some(v) => {
                parts.push((idx, json_to_swift(v)));
            }
        }
    }

    // Method calls on the configured client handle (e.g. `_client.chat(req)`) use
    // anonymous Swift argument labels (`func chat(_ req:)`), so omit `name:` prefixes.
    // Free-function calls (e.g. `process(source:, config:)`) keep labelled args.
    // Registration functions also use positional args.
    // Swift argument labels must be camelCase, so convert from snake_case.
    // Some APIs like detectMimeTypeFromBytes take unnamed first parameters —
    // omit labels for indices listed in unnamed_arg_indices.
    let is_register_call = function_name.starts_with("register") || function_name.starts_with("Register");
    let args_str = parts
        .into_iter()
        .map(|(idx, val)| {
            if is_method_call || is_register_call || unnamed_arg_indices.contains(&idx) {
                val
            } else {
                // Apply per-language argument renames before emitting the call.
                let arg_name: &str = arg_name_map
                    .and_then(|m| m.get(&args[idx].name).map(String::as_str))
                    .unwrap_or(&args[idx].name);
                let label = arg_name.to_lower_camel_case();
                format!("{label}: {val}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    (setup_lines, args_str)
}
