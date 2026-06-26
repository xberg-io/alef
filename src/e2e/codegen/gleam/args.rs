use crate::core::config::GleamElementConstructor;
use crate::e2e::escape::escape_gleam;
use heck::ToSnakeCase;

use super::constructors::render_gleam_element_constructor;
use super::values::json_to_gleam;

/// Build setup lines and the argument list for the function call.
///
/// Returns `None` when the test must be skipped entirely — this happens when a
/// `json_object` arg has no element-constructor recipe and no `json_object_wrapper`
/// configured, meaning the generated call would pass a raw JSON string where the
/// Gleam binding expects a typed record. Callers should emit a `// skipped` comment
/// and `Nil` body rather than broken code.
///
/// Gleam is statically typed, so each arg type must produce a correctly-typed expression:
/// - `file_path` -> quoted string literal
/// - `bytes` -> setup: `let assert Ok(data__) = e2e_gleam.read_file_bytes(...)` and arg: `data__`
/// - `string` + optional -> `option.Some("value")` or `option.None`
/// - `string` non-optional -> `"value"`
/// - `json_object` with recipe -> list/record constructor from `element_constructors`
/// - `json_object` with wrapper -> JSON-string literal wrapped by `json_object_wrapper`
/// - `json_object` with `options_via = "from_json"` -> `<snake_type>_from_json("{json}")` NIF call
/// - `json_object` without recipe, wrapper, or from_json -> caller is signalled to skip
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    test_documents_path: &str,
    element_constructors: &[GleamElementConstructor],
    json_object_wrapper: Option<&str>,
    module_path: &str,
    extra_args: &[String],
    options_type: Option<&str>,
    options_via: &str,
) -> Option<(Vec<String>, String)> {
    if args.is_empty() && extra_args.is_empty() {
        return Some((Vec::new(), String::new()));
    }

    // Pre-check: if any json_object arg has no recipe, wrapper, or from_json override,
    // the call cannot be expressed in Gleam. Signal the caller to skip.
    for arg in args {
        if arg.arg_type == "json_object" {
            let element_type = arg.element_type.as_deref().unwrap_or("");
            let has_recipe =
                !element_type.is_empty() && element_constructors.iter().any(|r| r.element_type == element_type);
            let has_wrapper = json_object_wrapper.is_some();
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let has_from_json = options_via == "from_json"
                && crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, val).is_some();
            // An optional json_object with no value can safely emit option.None / [].
            let val = input.get(field);
            let is_null_optional = arg.optional && matches!(val, None | Some(serde_json::Value::Null));
            if !has_recipe && !has_wrapper && !has_from_json && !is_null_optional {
                return None;
            }
        }
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut bytes_var_counter = 0usize;

    for arg in args {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);

        match arg.arg_type.as_str() {
            "handle" => {
                // Engine construction: create_engine(option.None).
                // Config construction from JSON is complex in Gleam (no JSON string constructor),
                // so we always pass option.None — default engine config covers most test cases.
                let name = &arg.name;
                let constructor = format!("create_{}", name.to_snake_case());
                setup_lines.push(format!(
                    "let assert Ok({name}) = {module_path}.{constructor}(option.None)"
                ));
                parts.push(name.clone());
                continue;
            }
            "mock_url" => {
                // Resolve the mock server base URL at runtime via envoy, then append the fixture path.
                let name = &arg.name;
                setup_lines.push(format!(
                    "let {name} = case envoy.get(\"MOCK_SERVER_URL\") {{ Ok(base) -> base <> \"/fixtures/{fixture_id}\" Error(_) -> \"http://localhost:8080/fixtures/{fixture_id}\" }}"
                ));
                parts.push(name.clone());
                continue;
            }
            "file_path" => {
                // Always a required string path.
                // Gleam e2e runs from e2e/gleam/ so the path resolves relative
                // to the configured test-documents directory.
                let path = val.and_then(|v| v.as_str()).unwrap_or("");
                let full_path = format!("{test_documents_path}/{path}");
                parts.push(format!("\"{}\"", escape_gleam(&full_path)));
            }
            "bytes" => {
                // Read the file at runtime via Erlang file:read_file/1.
                // The fixture `data` field holds the path relative to the
                // configured test-documents directory.
                let path = val.and_then(|v| v.as_str()).unwrap_or("");
                let var_name = if bytes_var_counter == 0 {
                    "data_bytes__".to_string()
                } else {
                    format!("data_bytes_{bytes_var_counter}__")
                };
                bytes_var_counter += 1;
                // Use relative path from e2e/gleam/ project root.
                let full_path = format!("{test_documents_path}/{path}");
                setup_lines.push(format!(
                    "let assert Ok({var_name}) = e2e_gleam.read_file_bytes(\"{}\")",
                    escape_gleam(&full_path)
                ));
                parts.push(var_name);
            }
            "string" if arg.optional => {
                // Optional string: emit option.Some("value") or option.None.
                match val {
                    None | Some(serde_json::Value::Null) => {
                        parts.push("option.None".to_string());
                    }
                    Some(serde_json::Value::String(s)) if s.is_empty() => {
                        parts.push("option.None".to_string());
                    }
                    Some(serde_json::Value::String(s)) => {
                        parts.push(format!("option.Some(\"{}\")", escape_gleam(s)));
                    }
                    Some(v) => {
                        parts.push(format!("option.Some({})", json_to_gleam(v)));
                    }
                }
            }
            "string" => {
                // Non-optional string.
                match val {
                    None | Some(serde_json::Value::Null) => {
                        parts.push("\"\"".to_string());
                    }
                    Some(serde_json::Value::String(s)) => {
                        parts.push(format!("\"{}\"", escape_gleam(s)));
                    }
                    Some(v) => {
                        parts.push(json_to_gleam(v));
                    }
                }
            }
            "json_object" => {
                // from_json path: use `<snake_type>_from_json(json)` NIF.
                if options_via == "from_json" {
                    let empty_obj = serde_json::Value::Object(Default::default());
                    let config_val = val.unwrap_or(&empty_obj);
                    if let Some(opts_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, config_val)
                    {
                        if !config_val.is_null() {
                            let snake_opts = opts_type.to_snake_case();
                            let json_str = serde_json::to_string(config_val).unwrap_or_default();
                            let escaped = escape_gleam(&json_str);
                            let var_name = format!("{}_json__", &arg.name);
                            setup_lines.push(format!(
                                "let assert Ok({var_name}) = {module_path}.{snake_opts}_from_json(\"{escaped}\")"
                            ));
                            parts.push(var_name);
                        }
                        continue;
                    }
                }

                // Look up a per-`element_type` constructor recipe declared in
                // `[crates.gleam.element_constructors]`. When present, build a
                // record literal from the recipe; otherwise fall back to a
                // generic JSON-string emission via `json_to_gleam`.
                let element_type = arg.element_type.as_deref().unwrap_or("");
                let recipe = if element_type.is_empty() {
                    None
                } else {
                    element_constructors.iter().find(|r| r.element_type == element_type)
                };

                if let Some(recipe) = recipe {
                    // List-of-records emission: each JSON-array item becomes
                    // one constructor call; non-array values produce an empty
                    // list (preserving the iter15 behaviour).
                    let items_expr = match val {
                        Some(serde_json::Value::Array(arr)) => {
                            let items: Vec<String> = arr
                                .iter()
                                .map(|item| render_gleam_element_constructor(item, recipe, test_documents_path))
                                .collect();
                            format!("[{}]", items.join(", "))
                        }
                        _ => "[]".to_string(),
                    };
                    if arg.optional && (val.is_none() || val == Some(&serde_json::Value::Null)) {
                        parts.push("[]".to_string());
                    } else {
                        parts.push(items_expr);
                    }
                } else if arg.optional && (val.is_none() || val == Some(&serde_json::Value::Null)) {
                    parts.push("option.None".to_string());
                } else {
                    let empty_obj = serde_json::Value::Object(Default::default());
                    let config_val = val.unwrap_or(&empty_obj);
                    let json_literal = json_to_gleam(config_val);
                    // When the project has configured a wrapper (e.g.
                    // `sample_core.config_from_json_string({json})`), substitute
                    // the placeholder; otherwise emit the bare JSON-string
                    // literal.
                    let emitted = match json_object_wrapper {
                        Some(template) => template.replace("{json}", &json_literal),
                        None => json_literal,
                    };
                    parts.push(emitted);
                }
            }
            "int" | "integer" => match val {
                None | Some(serde_json::Value::Null) if arg.optional => {}
                None | Some(serde_json::Value::Null) => parts.push("0".to_string()),
                Some(v) => parts.push(json_to_gleam(v)),
            },
            "bool" | "boolean" => match val {
                Some(serde_json::Value::Bool(true)) => parts.push("True".to_string()),
                Some(serde_json::Value::Bool(false)) | None | Some(serde_json::Value::Null) => {
                    if !arg.optional {
                        parts.push("False".to_string());
                    }
                }
                Some(v) => parts.push(json_to_gleam(v)),
            },
            _ => {
                // Fallback for unknown types.
                match val {
                    None | Some(serde_json::Value::Null) if arg.optional => {}
                    None | Some(serde_json::Value::Null) => parts.push("Nil".to_string()),
                    Some(v) => parts.push(json_to_gleam(v)),
                }
            }
        }
    }

    // Append verbatim extra_args (e.g. "option.None" for optional query params
    // like `list_files(client, query)` where gleam needs `option.None`).
    for extra in extra_args {
        parts.push(extra.clone());
    }

    Some((setup_lines, parts.join(", ")))
}
