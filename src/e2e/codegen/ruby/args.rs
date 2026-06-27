//! Ruby e2e argument/rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::resolve_field;
use crate::e2e::escape::ruby_string_literal;
use heck::ToSnakeCase;
use std::collections::HashMap;

use super::values::{is_base64, is_file_path, json_to_ruby};

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
/// Emit Ruby object-array fixture values for a typed `json_object` array.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    call_receiver: &str,
    module_name: &str,
    options_type: Option<&str>,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    fixture: &crate::e2e::fixture::Fixture,
    adapter_request_type: Option<&str>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> (Vec<String>, String, Vec<String>) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: don't pass the input as a function argument.
        // The input data is for setup/mocking purposes only. Functions with no
        // parameters must be called with no arguments — not with `{}` or `nil`.
        return (Vec::new(), String::new(), Vec::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    // Teardown lines emitted after the call+assertions. Populated by
    // trait-bridge args so RSpec's shared-process registry state is restored
    // between tests (e.g. `<Binding>.unregister_<trait>('test-backend')`).
    let mut teardown_lines: Vec<String> = Vec::new();
    // Track optional args that were skipped; if a later arg is emitted we must back-fill nil
    // to preserve positional correctness (e.g. extract_file(path, nil, config)).
    let mut skipped_optional_count: usize = 0;

    for arg in args {
        if arg.arg_type == "mock_url" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "{} = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "{} = \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}_req", arg.name);
                // Derive the module qualifier from module_name (e.g. "DemoCrawler")
                let mod_qualifier = super::values::ruby_module_name(module_name);
                setup_lines.push(format!(
                    "{req_var} = {mod_qualifier}::{req_type}.new(url: {})",
                    arg.name
                ));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // Array of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the codegen
            // falls back to a JSON-array literal of bare relative paths and the Rust
            // HTTP client rejects them.
            // Flush any pending nil placeholders before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter().filter_map(|v| v.as_str().map(ruby_string_literal)).collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "{name}_base = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\""
            ));
            setup_lines.push(format!(
                "{name} = [{paths_literal}].map {{ |p| p.start_with?('http') ? p : \"#{{{name}_base}}#{{p}}\" }}"
            ));
            parts.push(name.clone());
            continue;
        }

        // Handle bytes arguments: load from file if needed
        if arg.arg_type == "bytes" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let resolved = resolve_field(input, &arg.field);
            if let Some(s) = resolved.as_str() {
                if is_file_path(s) {
                    // File path: load with File.read and convert to bytes array
                    setup_lines.push(format!("{} = File.read(\"{}\").bytes", arg.name, s));
                } else if is_base64(s) {
                    // Base64: decode it
                    setup_lines.push(format!("{} = Base64.decode64(\"{}\").bytes", arg.name, s));
                } else {
                    // Inline text: encode it to binary and convert to bytes array
                    let escaped = ruby_string_literal(s);
                    setup_lines.push(format!("{} = {}.b.bytes", arg.name, escaped));
                }
                parts.push(arg.name.clone());
            } else {
                parts.push("nil".to_string());
            }
            continue;
        }

        // Handle file_path arguments: pass the path string as-is
        if arg.arg_type == "file_path" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            let resolved = resolve_field(input, &arg.field);
            if let Some(s) = resolved.as_str() {
                let escaped = ruby_string_literal(s);
                parts.push(escaped);
            } else if arg.optional {
                skipped_optional_count += 1;
                continue;
            } else {
                parts.push("''".to_string());
            }
            continue;
        }

        if arg.arg_type == "handle" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            // Generate a create_engine (or equivalent) call and pass the variable.
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let config_value = resolve_field(input, &arg.field);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("{} = {call_receiver}.{constructor_name}(nil)", arg.name,));
            } else {
                let literal = json_to_ruby(config_value);
                let name = &arg.name;
                setup_lines.push(format!("{name}_config = {literal}"));
                setup_lines.push(format!(
                    "{} = {call_receiver}.{constructor_name}({name}_config.to_json)",
                    arg.name,
                    name = name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            // Flush any pending nil placeholders for skipped optionals before this positional arg.
            for _ in 0..skipped_optional_count {
                parts.push("nil".to_string());
            }
            skipped_optional_count = 0;
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let emission = crate::e2e::codegen::emit_test_backend("ruby", trait_bridge, &methods, fixture);
                    // Split multi-line setup_block into individual lines so the
                    // Jinja template can indent each line uniformly with `    {{ line }}`.
                    for line in emission.setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(emission.arg_expr);

                    // For register_fn traits (plugin pattern), Magnus requires a second "name" argument.
                    // Extract the backend name from fixture input (same logic as emit_test_backend).
                    if trait_bridge.register_fn.is_some() {
                        let backend_name = super::stubs::extract_backend_name_from_input(&fixture.input, &fixture.id);
                        parts.push(ruby_string_literal(&backend_name));

                        // Emit `<module>.<unregister_fn>('<name>')` after the call so
                        // RSpec's single-process registry is restored between tests.
                        // Without this, the next trait-using fixture fails because the test
                        // registry contains only the test stub and the core's `ensure_*_initialized`
                        // self-heal only triggers when registry is empty.
                        if let Some(unregister_fn) = trait_bridge.unregister_fn.as_deref() {
                            teardown_lines.push(format!(
                                "{call_receiver}.{unregister_fn}({})",
                                ruby_string_literal(&backend_name)
                            ));
                        }
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("ruby");
            setup_lines.push(format!("# {}", emission.arg_expr));
            parts.push("nil".to_string());
            continue;
        }

        let resolved = resolve_field(input, &arg.field);
        let val = if resolved.is_null() { None } else { Some(resolved) };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: defer; emit nil only if a later arg is present.
                skipped_optional_count += 1;
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: flush deferred nils, then pass a default.
                for _ in 0..skipped_optional_count {
                    parts.push("nil".to_string());
                }
                skipped_optional_count = 0;
                let default_val = match arg.arg_type.as_str() {
                    "string" => "''".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // Flush deferred nil placeholders for skipped optional args that precede this one.
                for _ in 0..skipped_optional_count {
                    parts.push("nil".to_string());
                }
                skipped_optional_count = 0;
                // For json_object args with options_type, construct a typed options object.
                // When result_is_simple, the binding accepts a plain Hash (no wrapper class).
                if arg.arg_type == "json_object" && !v.is_null() {
                    // Check for typed object arrays (element_type set)
                    if let Some(_elem_type) = &arg.element_type {
                        if v.is_array() {
                            if let Some(arr) = v.as_array() {
                                // Only emit as tagged-enum array if all elements are objects.
                                // Otherwise fall through to json_to_ruby for primitive arrays (e.g., String, Int).
                                if !arr.is_empty() && arr.iter().all(|item| item.is_object()) {
                                    let mock_base_var = if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                                        let base_var = format!("{}_mock_base_url", arg.name);
                                        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                        setup_lines.push(format!(
                                                "{base_var} = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\""
                                            ));
                                        Some(base_var)
                                    } else {
                                        None
                                    };
                                    parts.push(emit_ruby_object_array_with_mock_base(v, mock_base_var.as_deref()));
                                    continue;
                                }
                            }
                            // Fall through if array is empty or contains non-objects (primitives)
                        }
                    }
                    // Otherwise handle regular typed objects
                    let object_type = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v);
                    if let (Some(opts_type), Some(obj)) = (object_type, v.as_object()) {
                        let mock_base_var = if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                            let base_var = format!("{}_mock_base_url", arg.name);
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            setup_lines.push(format!(
                                "{base_var} = ENV.fetch('{env_key}', nil) || \"#{{ENV.fetch('MOCK_SERVER_URL')}}/fixtures/{fixture_id}\""
                            ));
                            Some(base_var)
                        } else {
                            None
                        };
                        let kwargs: Vec<String> = obj
                            .iter()
                            .filter_map(|(k, vv)| {
                                // Skip empty string values (they cause enum parsing failures)
                                if let Some(s) = vv.as_str() {
                                    if s.is_empty() {
                                        return None; // Skip all empty strings
                                    }
                                    // For known enum fields, use snake_case enum variant
                                    if enum_fields.contains_key(k) {
                                        let snake_key = k.to_snake_case();
                                        let snake_val = s.to_snake_case();
                                        return Some(format!("{snake_key}: '{snake_val}'"));
                                    }
                                }
                                let snake_key = k.to_snake_case();
                                let rb_val =
                                    if let (Some(base_var), Some(raw)) = (mock_base_var.as_deref(), vv.as_str()) {
                                        if raw.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER) {
                                            format!(
                                                "{}.gsub('{}', {base_var})",
                                                json_to_ruby(vv),
                                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                            )
                                        } else {
                                            json_to_ruby(vv)
                                        }
                                    } else {
                                        json_to_ruby(vv)
                                    };
                                Some(format!("{snake_key}: {rb_val}"))
                            })
                            .collect();
                        if result_is_simple {
                            parts.push(format!("{{{}}}", kwargs.join(", ")));
                        } else {
                            parts.push(format!("{opts_type}.new({})", kwargs.join(", ")));
                        }
                        continue;
                    }
                }
                parts.push(json_to_ruby(v));
            }
        }
    }

    (setup_lines, parts.join(", "), teardown_lines)
}

/// Emit Ruby object-array fixture values and optionally replace `$mock_url`.
pub(super) fn emit_ruby_object_array_with_mock_base(arr: &serde_json::Value, mock_base_var: Option<&str>) -> String {
    if let Some(items) = arr.as_array() {
        let item_strs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.as_object().map(|obj| {
                    let object_value = serde_json::Value::Object(obj.clone());
                    if let Some(base_var) = mock_base_var
                        && crate::e2e::codegen::value_contains_mock_url_placeholder(&object_value)
                    {
                        let json_str = serde_json::to_string(&object_value).unwrap_or_else(|_| "{}".to_string());
                        format!(
                            "JSON.parse({}.gsub('{}', {base_var}))",
                            ruby_string_literal(&json_str),
                            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                        )
                    } else {
                        json_to_ruby(&object_value)
                    }
                })
            })
            .collect();
        format!("[{}]", item_strs.join(", "))
    } else {
        "[]".to_string()
    }
}
