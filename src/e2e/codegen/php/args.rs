//! PHP e2e argument/rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_php;
use heck::ToUpperCamelCase;
use std::collections::HashMap;

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// `options_via` controls how `json_object` args are passed:
/// - `"array"` (default): PHP array literal `["key" => value, ...]`
/// - `"json"`: JSON string via `json_encode([...])` — use when the Rust method accepts `Option<String>`
///
/// `options_type` is the PHP class name (e.g. `"ProcessConfig"`) used when constructing options
/// via `ClassName::from_json(json_encode([...]))`. Required when `options_via` is not `"json"` and
/// the binding accepts a typed config object.
///
/// Returns `(setup_lines, args_string, teardown_block)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    class_name: &str,
    _enum_fields: &HashMap<String, String>,
    fixture: &crate::e2e::fixture::Fixture,
    options_via: &str,
    options_type: Option<&str>,
    adapter_request_type: Option<&str>,
    namespace: &str,
    owner_handle_is_receiver: bool,
    type_defs: &[crate::core::ir::TypeDef],
    php_lang_rename_all: &str,
    config: &ResolvedCrateConfig,
    _trait_bridge_imports: &mut Vec<String>,
) -> (Vec<String>, String, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args configuration: pass the whole input only if it's non-empty.
        // Functions with no parameters (e.g. list_models) have empty input and get no args.
        let is_empty_input = match input {
            serde_json::Value::Null => true,
            serde_json::Value::Object(m) => m.is_empty(),
            _ => false,
        };
        if is_empty_input {
            return (Vec::new(), String::new(), String::new());
        }
        return (Vec::new(), super::values::json_to_php(input), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut teardown_block = String::new();

    // True when any arg after `from_idx` has a fixture value (or has no fixture
    // value but is required — i.e. would emit *something*). Used to decide
    // whether a missing optional middle arg must emit `null` to preserve the
    // positional argument layout, or can be safely dropped.
    let arg_has_emission = |arg: &crate::e2e::config::ArgMapping| -> bool {
        let val = if arg.field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) => {
                // A `json_object` arg named `config` always emits a value (a default
                // `Type::from_json('{}')`) regardless of `optional`, mirroring the
                // unconditional special case in the per-arg loop below. Treating it as
                // "no emission" would let an earlier optional arg (e.g. `mime_type`) be
                // dropped, shifting `config` into the wrong positional slot.
                if arg.arg_type == "json_object" && arg.name == "config" {
                    return true;
                }
                !arg.optional
            }
            Some(_) => true,
        }
    };
    let any_later_has_emission = |from_idx: usize| -> bool { args[from_idx..].iter().any(arg_has_emission) };

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "${} = getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "${} = getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("${}_req", arg.name);
                setup_lines.push(format!("{req_var} = new {req_type}(${});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(format!("${}", arg.name));
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // array of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let val = if let Some(v) = input.get(field).filter(|v| !v.is_null()) {
                v.clone()
            } else {
                crate::e2e::codegen::resolve_urls_field(input, &arg.field).clone()
            };
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_php(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "${name}_base = getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';"
            ));
            setup_lines.push(format!(
                "${name} = array_map(fn($p) => str_starts_with($p, 'http') ? $p : ${name}_base . $p, [{paths_literal}]);"
            ));
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("${name}_req");
                setup_lines.push(format!("{req_var} = new {req_type}(${name});"));
                parts.push(req_var);
            } else {
                parts.push(format!("${name}"));
            }
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a createEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let config_value = if arg.field == "input" {
                input
            } else {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                input.get(field).unwrap_or(&serde_json::Value::Null)
            };
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("${} = {class_name}::{constructor_name}(null);", arg.name,));
            } else {
                let name = &arg.name;
                // Use <ConfigType>::from_json() instead of direct property assignment.
                // ext-php-rs doesn't support writable #[php(prop)] fields for complex types,
                // so serialize the config to JSON and use from_json() to construct it.
                // Filter out empty string enum values before passing to from_json().
                let filtered_config = super::values::filter_empty_enum_strings(config_value);
                // The PHP binding's `from_json` deserializes into the binding struct, which is
                // always emitted with `#[serde(rename_all = "{php_lang_rename_all}")]` by the
                // PHP backend (camelCase by default, or whatever `[crates.php] serde_rename_all`
                // overrides). The core IR's `serde_rename_all` may be None, but that has nothing
                // to do with what the binding deserializer expects.
                let config_rename_all = Some(php_lang_rename_all);
                let config_type = options_type.unwrap_or_else(|| {
                    panic!(
                        "e2e fixture {fixture_id}: handle arg `{}` requires an `options_type` on the call config (or per-language override). Set `[e2e.call] options_type = \"...\"` to the PHP class name of the handle's config struct.",
                        arg.name
                    )
                });
                setup_lines.push(format!(
                    "${name}_config = {config_type}::from_json(json_encode({}));",
                    super::values::json_to_php_camel_keys_with_types(
                        &filtered_config,
                        Some(config_type),
                        config_rename_all,
                        type_defs
                    )
                ));
                setup_lines.push(format!(
                    "${} = {class_name}::{constructor_name}(${name}_config);",
                    arg.name,
                ));
            }
            // For streaming owner_type adapters the handle is the instance-method
            // receiver, not a positional argument — emit its construction but omit
            // it from the call's argument list.
            if owner_handle_is_receiver {
                continue;
            }
            parts.push(format!("${}", arg.name));
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    // Collect methods from both the main trait and its super-trait (if present).
                    // The super-trait methods are needed so stubs implement the full interface.
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();

                    // If there's a super-trait, also collect its methods.
                    // Compare against rust_path (full module path), not just the simple name.
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                // Only add if not already present (avoid duplicates).
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    let emission =
                        super::stubs::emit_test_backend_with_ns(trait_bridge, &methods, fixture, namespace, class_name);
                    // Split multi-line setup_block into individual lines so the
                    // Jinja template can indent each line uniformly with `        {{ line }}`.
                    for line in emission.setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(emission.arg_expr);
                    teardown_block.push_str(&emission.teardown_block);
                    // Collect any function imports needed for trait-bridge teardown
                    _trait_bridge_imports.extend(emission.type_imports);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("php");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        let val = if arg.field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };

        // Bytes args: fixture stores either a fixture-relative path string (load
        // with file_get_contents at runtime, mirroring the go/python convention)
        // or an inline byte array (encode as a "\xNN" escape string).
        if arg.arg_type == "bytes" {
            match val {
                None | Some(serde_json::Value::Null) => {
                    if arg.optional {
                        parts.push("null".to_string());
                    } else {
                        parts.push("\"\"".to_string());
                    }
                }
                Some(serde_json::Value::String(s)) => {
                    let var_name = format!("{}Bytes", arg.name);
                    setup_lines.push(format!(
                        "${var_name} = file_get_contents(\"{path}\");\n        if (${var_name} === false) {{ $this->fail(\"failed to read fixture: {path}\"); }}",
                        path = s.replace('"', "\\\"")
                    ));
                    parts.push(format!("${var_name}"));
                }
                Some(serde_json::Value::Array(arr)) => {
                    let bytes: String = arr
                        .iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| format!("\\x{:02x}", n))
                        .collect();
                    parts.push(format!("\"{bytes}\""));
                }
                Some(other) => {
                    parts.push(super::values::json_to_php(other));
                }
            }
            continue;
        }

        match val {
            None | Some(serde_json::Value::Null) if arg.arg_type == "json_object" && arg.name == "config" => {
                if let Some(type_name) = options_type {
                    parts.push(format!("\\{namespace}\\{type_name}::from_json('{{}}')"));
                } else {
                    parts.push("null".to_string());
                }
                continue;
            }
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value. If a later arg WILL emit
                // something, we must keep this slot in place by passing `null`
                // so the positional argument layout matches the PHP signature.
                // Otherwise drop the trailing optional argument entirely.
                if any_later_has_emission(idx + 1) {
                    parts.push("null".to_string());
                }
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" if options_via == "json" => "null".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" && !v.is_null() {
                    let json_object_type =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v);
                    // Check for typed object arrays first.
                    if let Some(elem_type) = &arg.element_type {
                        if v.is_array() {
                            // When element_type is a scalar/primitive and value is an array,
                            // pass it directly as a PHP array (e.g. ["python"]) rather than
                            // wrapping in a typed config constructor.
                            if super::values::is_php_reserved_type(elem_type) {
                                parts.push(super::values::json_to_php(v));
                                continue;
                            }
                            // Typed object arrays wrap each element in `{ElemType}::from_json('{...}')`.
                            // PHP's #[php_class] FromZval only
                            // accepts class instances; raw assoc arrays produce "Failed to convert
                            // array element to {Type}". Every alef-emitted has_default/has_serde
                            // struct gets a `from_json` static method, so this is the portable path.
                            if v.as_array().is_some() {
                                if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                                    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                    let base_var = format!("{}MockBaseUrl", arg.name);
                                    setup_lines.push(format!(
                                        "${base_var} = getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';"
                                    ));
                                    parts.push(super::values::emit_php_object_array_with_mock_base(
                                        v,
                                        elem_type,
                                        Some(&base_var),
                                    ));
                                } else {
                                    parts.push(super::values::emit_php_object_array(v, elem_type));
                                }
                                continue;
                            }
                        }
                    }
                    match options_via {
                        "json" => {
                            // Pass as JSON string via json_encode(); the Rust method accepts Option<String>.
                            // Filter out empty string enum values.
                            let filtered_v = super::values::filter_empty_enum_strings(v);

                            // If the config is empty after filtering, pass null instead.
                            if let serde_json::Value::Object(obj) = &filtered_v {
                                if obj.is_empty() {
                                    parts.push("null".to_string());
                                    continue;
                                }
                            }

                            // The PHP binding deserializes into the binding struct, which is
                            // always emitted with the language-effective rename strategy
                            // (camelCase by default). The core IR's serde_rename_all is not
                            // what serde_json::from_str::<Self> reads.
                            let rename_all = Some(php_lang_rename_all);
                            parts.push(format!(
                                "json_encode({})",
                                super::values::json_to_php_camel_keys_with_types(
                                    &filtered_v,
                                    json_object_type,
                                    rename_all,
                                    type_defs
                                )
                            ));
                            continue;
                        }
                        _ => {
                            if let Some(type_name) = json_object_type {
                                // Use TypeName::from_json(json_encode([...])) to construct the
                                // typed config object. ext-php-rs structs expose a from_json()
                                // static method that accepts a JSON string.
                                // Filter out empty string enum values before passing to from_json().
                                let filtered_v = super::values::filter_empty_enum_strings(v);

                                // For empty objects, construct with from_json('{}') to get the
                                // type's defaults rather than passing null (which fails for non-optional params).
                                if let serde_json::Value::Object(obj) = &filtered_v {
                                    if obj.is_empty() {
                                        let arg_var = format!("${}", arg.name);
                                        setup_lines.push(format!(
                                            "{arg_var} = \\{namespace}\\{type_name}::from_json('{{}}');"
                                        ));
                                        parts.push(arg_var);
                                        continue;
                                    }
                                }

                                let arg_var = format!("${}", arg.name);
                                // The PHP `from_json` deserializes into the binding struct, which is
                                // emitted by the PHP backend with `#[serde(rename_all = "...")]` set
                                // to the language's effective rename strategy (camelCase by default
                                // for PHP, or whatever `[crates.php] serde_rename_all` overrides).
                                // The Rust core struct's serde_rename_all is irrelevant here — what
                                // matters is the BINDING struct, since `from_json` calls
                                // `serde_json::from_str::<Self>` against the binding type.
                                let type_rename_all = Some(php_lang_rename_all);
                                let php_value = super::values::json_to_php_camel_keys_with_types(
                                    &filtered_v,
                                    Some(type_name),
                                    type_rename_all,
                                    type_defs,
                                );
                                if crate::e2e::codegen::value_contains_mock_url_placeholder(&filtered_v) {
                                    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                    let base_var = format!("${}MockBaseUrl", arg.name);
                                    let json_var = format!("${}Json", arg.name);
                                    setup_lines.push(format!(
                                        "{base_var} = getenv(\"{env_key}\") ?: (getenv(\"MOCK_SERVER_URL\") . \"/fixtures/{fixture_id}\");"
                                    ));
                                    setup_lines.push(format!(
                                        "{json_var} = str_replace(\"{}\", {base_var}, json_encode({php_value}));",
                                        escape_php(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                                    ));
                                    setup_lines.push(format!(
                                        "{arg_var} = \\{namespace}\\{type_name}::from_json({json_var});"
                                    ));
                                } else {
                                    setup_lines.push(format!(
                                        "{arg_var} = \\{namespace}\\{type_name}::from_json(json_encode({php_value}));"
                                    ));
                                }
                                parts.push(arg_var);
                                continue;
                            }
                            // Fallback when no options_type is configured: pass the fixture object literally.
                            if v.as_object().is_some() {
                                parts.push(super::values::json_to_php(v));
                                continue;
                            }
                        }
                    }
                }
                parts.push(super::values::json_to_php(v));
            }
        }
    }

    (setup_lines, parts.join(", "), teardown_block)
}
