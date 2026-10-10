use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeRef};
use crate::e2e::codegen::call_ir::TargetParams;
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
    target_params: TargetParams<'_>,
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
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = input.get(field).unwrap_or(&serde_json::Value::Null);
            let url_expr = if let Some(url) =
                crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value)
            {
                format!("\"{}\"", escape_swift(url))
            } else if has_host_root_route {
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

        if arg.arg_type == "mock_url_list" {
            let value = crate::e2e::codegen::resolve_urls_field(input, &arg.field);
            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, value) {
                let literals = urls
                    .into_iter()
                    .map(|url| format!("\"{}\"", escape_swift(url)))
                    .collect::<Vec<_>>()
                    .join(", ");
                setup_lines.push(format!("let {} = [{literals}]", arg.name));
                parts.push((idx, arg.name.clone()));
                continue;
            }
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
            if let Some(trait_name) = &arg.trait_name
                && let Some(trait_bridge) = config
                    .trait_bridges_for(crate::core::config::Language::Swift)
                    .find(|tb| tb.trait_name == *trait_name)
            {
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
            // A `test_backend` arg fills a required Swift stub parameter — there is
            // no compilable value to fall back to when the trait isn't configured.
            // Fail generation loudly instead of silently splicing a `nil` argument
            // with a comment where the real stub belongs. ~keep
            panic!(
                "Swift e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Swift stub without a resolvable trait bridge",
                fixture.id, arg.name, arg.trait_name
            );
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
            let docs_files = fixture.docs_files_for_arg(&arg.field);
            let json_expr = docs_json_expression(&json_str, &arg.name, &docs_files, &mut setup_lines);

            // Detect if config arg is unnamed (index `idx` in unnamed_arg_indices).
            // E2e wrappers keep config unnamed and receive JSON strings.
            let config_is_unnamed = unnamed_arg_indices.contains(&idx);

            if config_is_unnamed {
                // E2e wrapper: pass JSON string directly (positional, no label).
                parts.push((idx, json_expr));
            } else {
                // Regular binding: deserialize to an opaque object.
                let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
                let from_json_fn = from_json_helper_for_arg(arg, options_type);
                // Qualify with module name to avoid ambiguity when both SampleCrate and RustBridge are imported.
                setup_lines.push(format!(
                    "let {var_name} = try {module_name}.{from_json_fn}({json_expr})"
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
            let val = super::super::resolve_field(input, &arg.field);
            let elem_type = arg.element_type.as_deref().unwrap_or("Unknown");
            // Convert element type to camelCase for the from-json helper name
            let from_json_fn = format!("{}FromJson", elem_type.to_lower_camel_case());

            match val {
                serde_json::Value::Object(_) => {
                    let json_str = serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string());
                    let escaped = escape_swift(&json_str);
                    let json_expr = if crate::e2e::codegen::value_contains_mock_url_placeholder(val) {
                        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                        let base_var = format!("{}MockBaseUrl", arg.name.to_lower_camel_case());
                        let json_var = format!("{}Json", arg.name.to_lower_camel_case());
                        setup_lines.push(format!(
                            "let {base_var} = ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\")"
                        ));
                        setup_lines.push(format!(
                            "let {json_var} = \"{escaped}\".replacingOccurrences(of: \"{}\", with: {base_var})",
                            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                        ));
                        json_var
                    } else {
                        format!("\"{escaped}\"")
                    };

                    if unnamed_arg_indices.contains(&idx) {
                        parts.push((idx, json_expr));
                    } else {
                        let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
                        setup_lines.push(format!(
                            "let {var_name} = try {module_name}.{from_json_fn}({json_expr})"
                        ));
                        parts.push((idx, var_name));
                    }
                }
                serde_json::Value::Array(arr) => {
                    let var_name = format!("{}Array", arg.name.to_lower_camel_case());

                    if arr.is_empty() {
                        // Empty array literal
                        parts.push((idx, "[]".to_string()));
                    } else {
                        // For each JSON item in the array, call the helper to deserialize it
                        let json_strs: Vec<String> =
                            arr.iter().filter_map(|item| serde_json::to_string(item).ok()).collect();
                        let mock_base_var = if arr.iter().any(crate::e2e::codegen::value_contains_mock_url_placeholder)
                        {
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            let base_var = format!("{}MockBaseUrl", arg.name.to_lower_camel_case());
                            setup_lines.push(format!(
                                "let {base_var} = ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\")"
                            ));
                            Some(base_var)
                        } else {
                            None
                        };

                        let mut item_vars = Vec::new();
                        for (i, json_str) in json_strs.iter().enumerate() {
                            let escaped = escape_swift(json_str);
                            let item_var = format!("_item_{var_name}_{i}");
                            let json_expr = if let Some(base_var) = mock_base_var.as_deref()
                                && json_str.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                            {
                                format!(
                                    "\"{escaped}\".replacingOccurrences(of: \"{}\", with: {base_var})",
                                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                )
                            } else {
                                format!("\"{escaped}\"")
                            };
                            // Call the wrapper-module's `{type}FromJson` helper rather than the
                            // raw `RustBridge` one so the resulting element is the
                            // wrapper-module's `PageAction` (etc.), matching the type the
                            // function signature expects. The wrapper internally delegates to
                            // `RustBridge.{type}FromJson` which understands the
                            // serde(tag = "type") format.
                            setup_lines.push(format!(
                                "let {item_var} = try {module_name}.{from_json_fn}({json_expr})"
                            ));
                            item_vars.push(item_var);
                        }

                        // Construct the final array from all item variables
                        setup_lines.push(format!("let {var_name} = [{}]", item_vars.join(", ")));
                        parts.push((idx, var_name));
                    }
                }
                serde_json::Value::Null if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                serde_json::Value::Null => {
                    // Required but missing — emit empty array
                    parts.push((idx, "[]".to_string()));
                }
                _other => {
                    // Non-array value — emit empty array (shouldn't happen)
                    parts.push((idx, "[]".to_string()));
                }
            }
            continue;
        }

        if arg.arg_type == "json_object" && arg.element_type.as_deref() == Some("String") {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            if let Some(serde_json::Value::Array(arr)) = input.get(field) {
                let mock_base_var = if arr.iter().any(crate::e2e::codegen::value_contains_mock_url_placeholder) {
                    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                    let base_var = format!("{}MockBaseUrl", arg.name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {base_var} = ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\")"
                    ));
                    Some(base_var)
                } else {
                    None
                };
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|raw| {
                        let escaped = escape_swift(raw);
                        if let Some(base_var) = mock_base_var.as_deref()
                            && raw.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                        {
                            format!(
                                "\"{escaped}\".replacingOccurrences(of: \"{}\", with: {base_var})",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            )
                        } else {
                            format!("\"{escaped}\"")
                        }
                    })
                    .collect();
                parts.push((idx, format!("[{}]", items.join(", "))));
                continue;
            }
        }

        // json_object non-config args with options_via = "from_json":
        // Use the generated `{typeCamelCase}FromJson(_:)` helper so the fixture JSON is
        // deserialised into the opaque swift-bridge type rather than passed as a raw string.
        // When arg.field == "input", the entire fixture input IS the request object.
        // When a visitor handle is present, use `{typeCamelCase}FromJsonWithVisitor(json, handle)`
        // instead to attach the visitor to the options in one step.
        if arg.arg_type == "json_object"
            && options_via == Some("from_json")
            && let Some(type_name) = options_type
        {
            let resolved_val = super::super::resolve_field(input, &arg.field);
            let json_str = match resolved_val {
                serde_json::Value::Null => "{}".to_string(),
                v => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            let docs_files = fixture.docs_files_for_arg(&arg.field);
            let json_expr = docs_json_expression(&json_str, &arg.name, &docs_files, &mut setup_lines);
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
                    "let {var_name} = try {module_name}.{with_visitor_fn}({json_expr}, {handle_var})"
                ));
            } else {
                let from_json_fn = format!("{}FromJson", type_name.to_lower_camel_case());
                setup_lines.push(format!(
                    "let {var_name} = try {module_name}.{from_json_fn}({json_expr})"
                ));
            }
            parts.push((idx, var_name));
            continue;
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
                if let Some(typed) = ir_typed_swift_expression(arg, idx, v, target_params, enums) {
                    parts.push((idx, typed));
                    continue;
                }
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

/// The Swift expression for an argument whose *declared* parameter type the core IR resolved, or
/// `None` to keep the existing `arg_type`-only lowering.
///
/// This is the Swift answer to the shared question [`TargetParams`] poses, not a shared verdict.
/// `ArgMapping::arg_type` defaults to `"string"`, so before the seam a fixture string bound for an
/// enum-typed parameter reached [`json_to_swift`] and came out as a *quoted string literal*, which
/// `swiftc` rejects against a `public enum` parameter -- Swift has no implicit `String` -> enum
/// conversion, not even for a `RawRepresentable` one.
///
/// The replacement names the case, and both halves are read off
/// `backends::swift::gen_bindings::enums::emit_enum`: it spells the case
/// `swift_source_ident(variant.name.to_lower_camel_case())` (so a case colliding with a Swift
/// keyword keeps its backticks) and gives it the raw value `unit_enum_wire_value`, i.e. the
/// variant's `serde(rename)` if it has one and the enum's `rename_all` spelling otherwise. Naming
/// the case rather than emitting `T(rawValue:)!` keeps the failure at compile time, where an
/// unrecognised value is visible, instead of trapping at test run time. ~keep
///
/// Deliberately narrow. Only a bare `TypeRef::Named` qualifies: an `Optional<T>`/`Vec<T>` parameter
/// wants a wrapper this expression does not build. Only a *unit-only enum with serde* qualifies --
/// `emit_enum` renders `has_serde: false` as a `typealias` onto the opaque `RustBridge` type and a
/// data-carrying enum as a `Codable` enum with associated values, and neither takes a bare case
/// reference built from a string. And a wire value naming no variant keeps its literal: a fixture
/// may be feeding a deliberately invalid value to exercise the binding's own validation. ~keep
///
/// **Not** extended to DTO-typed parameters. The Swift emitter renders a struct either as a
/// first-class `public struct T: Codable` or as a `typealias T = RustBridge.T` decoded through a
/// generated `tFromJson` helper, and which one it picks is a fixed point over the *whole*
/// `ApiSurface` (`gen_bindings::dto::compute_first_class_dto_names`) that this seam is not handed.
/// Guessing between `JSONDecoder().decode(T.self, ...)` and `try Module.tFromJson(...)` would trade
/// one compile error for another, so the DTO arm stays on today's lowering. ~keep
fn ir_typed_swift_expression(
    arg: &crate::e2e::config::ArgMapping,
    index: usize,
    value: &serde_json::Value,
    target_params: TargetParams<'_>,
    enums: &[EnumDef],
) -> Option<String> {
    let TypeRef::Named(declared) = &target_params.param_for(&arg.name, index)?.ty else {
        return None;
    };
    let text = value.as_str()?;
    let enum_def = enums.iter().find(|enum_def| {
        &enum_def.name == declared && enum_def.has_serde && enum_def.variants.iter().all(|v| v.fields.is_empty())
    })?;
    let variant = enum_def.variants.iter().find(|variant| {
        crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        ) == text
    })?;
    let case_name = crate::backends::swift::naming::swift_source_ident(&variant.name.to_lower_camel_case());
    Some(format!("{declared}.{case_name}"))
}

fn docs_json_expression(
    json: &str,
    variable: &str,
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
    setup_lines: &mut Vec<String>,
) -> String {
    if files.is_empty() {
        return format!("\"{}\"", escape_swift(json));
    }
    let mut value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let mut replacements = String::new();
    for (index, file) in files.iter().enumerate() {
        let marker = format!("__ALEF_DOC_FILE_{index}__");
        let target = if file.field.is_empty() {
            Some(&mut value)
        } else {
            value.pointer_mut(&file.field)
        };
        let Some(target) = target else { continue };
        *target = serde_json::Value::String(marker.clone());
        setup_lines.push(
            crate::e2e::template_env::render(
                "swift/docs_file_read.jinja",
                crate::alef_context! { variable => variable, index => index, path => escape_swift(&file.path) },
            )
            .trim_end()
            .to_string(),
        );
        setup_lines.push(
            crate::e2e::template_env::render(
                "swift/docs_file_json.jinja",
                crate::alef_context! { variable => variable, index => index },
            )
            .trim_end()
            .to_string(),
        );
        replacements.push_str(&format!(
            ".replacingOccurrences(of: \"\\\\\\\"{marker}\\\\\\\"\", with: {variable}File{index}Json)"
        ));
    }
    format!(
        "\"{}\"{replacements}",
        escape_swift(&serde_json::to_string(&value).unwrap_or_default())
    )
}

#[cfg(test)]
mod tests {
    use super::docs_json_expression;
    use crate::e2e::fixture::FixtureDocsFileInput;

    #[test]
    fn nested_typed_dto_files_become_runtime_byte_arrays() {
        let mut setup = Vec::new();
        let expression = docs_json_expression(
            r#"{"content":"ignored"}"#,
            "request",
            &[FixtureDocsFileInput {
                field: "/content".into(),
                path: "document.pdf".into(),
            }],
            &mut setup,
        );

        assert_eq!(
            setup[0],
            "let requestFile0 = try Data(contentsOf: URL(fileURLWithPath: \"document.pdf\"))"
        );
        assert!(setup[1].contains("requestFile0.map(String.init)"));
        assert!(expression.contains("replacingOccurrences"), "{expression}");
        assert!(expression.contains("requestFile0Json"), "{expression}");
    }

    // --- typed-argument lowering (alef #227) --------------------------------------------------

    use super::build_args_and_setup;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::{EnumDef, EnumVariant, ParamDef, TypeRef};
    use crate::e2e::codegen::call_ir::TargetParams;
    use crate::e2e::config::ArgMapping;
    use crate::e2e::fixture::Fixture;

    /// An `args` entry with the default `arg_type` (`"string"`) -- the shape that used to send
    /// every value through `json_to_swift` regardless of the declared parameter type. ~keep
    fn default_typed_arg(name: &str) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "string".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn mode_enum() -> EnumDef {
        EnumDef {
            name: "FilePurpose".to_string(),
            has_serde: true,
            serde_rename_all: Some("kebab-case".to_string()),
            variants: vec![EnumVariant {
                name: "FineTune".to_string(),
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }
    }

    fn swift_args_for(enums: &[EnumDef], target_params: TargetParams<'_>) -> String {
        let args = vec![default_typed_arg("purpose")];
        let fixture = Fixture {
            id: "typed".to_string(),
            input: serde_json::json!({ "purpose": "fine-tune" }),
            ..Fixture::default()
        };
        let (_setup, args_str) = build_args_and_setup(
            &fixture.input,
            &args,
            &fixture.id,
            false,
            "convert",
            None,
            None,
            None,
            None,
            true,
            "Sample",
            &[],
            &ResolvedCrateConfig::default(),
            &[],
            &fixture,
            None,
            None,
            enums,
            target_params,
        );
        args_str
    }

    /// The defect: a fixture string bound for an enum-typed parameter used to stay a *string
    /// literal*, which `swiftc` rejects against a `public enum FilePurpose: String`. The variant
    /// here is `rename_all = "kebab-case"`, the case a naive camel-casing of the wire value would
    /// get wrong. ~keep
    #[test]
    fn a_string_value_for_an_ir_enum_parameter_names_the_generated_case() {
        let params = [ParamDef {
            name: "purpose".to_string(),
            ty: TypeRef::Named("FilePurpose".to_string()),
            ..ParamDef::default()
        }];
        assert_eq!(
            swift_args_for(&[mode_enum()], TargetParams::Known(&params)),
            "FilePurpose.fineTune"
        );
    }

    /// The other half of the three-state trade. Identical arg and fixture value, `IrAbsent`
    /// instead of `Known`: the pre-seam lowering must survive verbatim, or every IR-less caller
    /// regresses silently. ~keep
    #[test]
    fn the_same_string_value_still_renders_as_a_literal_when_the_ir_is_absent() {
        assert_eq!(swift_args_for(&[mode_enum()], TargetParams::IrAbsent), "\"fine-tune\"");
    }

    /// A wire value naming no variant is very often a deliberately invalid value driving the
    /// binding's own validation, so it keeps its literal rather than gaining an invented case. ~keep
    #[test]
    fn an_unmatched_enum_wire_value_keeps_its_string_literal() {
        let params = [ParamDef {
            name: "purpose".to_string(),
            ty: TypeRef::Named("FilePurpose".to_string()),
            ..ParamDef::default()
        }];
        let mut unmatched = mode_enum();
        unmatched.variants[0].name = "Assistants".to_string();
        assert_eq!(
            swift_args_for(&[unmatched], TargetParams::Known(&params)),
            "\"fine-tune\""
        );
    }

    /// `emit_enum` renders a `has_serde: false` enum as a `typealias` onto the opaque bridge type,
    /// which has no such case to name, so the seam must decline. ~keep
    #[test]
    fn a_non_serde_enum_keeps_the_existing_lowering() {
        let params = [ParamDef {
            name: "purpose".to_string(),
            ty: TypeRef::Named("FilePurpose".to_string()),
            ..ParamDef::default()
        }];
        let mut non_serde = mode_enum();
        non_serde.has_serde = false;
        assert_eq!(
            swift_args_for(&[non_serde], TargetParams::Known(&params)),
            "\"fine-tune\""
        );
    }

    /// An `Optional<T>` parameter wants a wrapper this expression does not build, so the seam must
    /// decline rather than unwrap to `T` and trade one compile error for another. ~keep
    #[test]
    fn a_wrapped_named_parameter_is_left_to_the_existing_lowering() {
        let params = [ParamDef {
            name: "purpose".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("FilePurpose".to_string()))),
            ..ParamDef::default()
        }];
        assert_eq!(
            swift_args_for(&[mode_enum()], TargetParams::Known(&params)),
            "\"fine-tune\""
        );
    }
}
