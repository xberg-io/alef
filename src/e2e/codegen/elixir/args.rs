//! Elixir e2e argument and setup rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_elixir;
use heck::ToSnakeCase;
use std::collections::HashMap;

use super::stubs::emit_test_backend;
use super::values::json_to_elixir;

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    module_path: &str,
    options_type: Option<&str>,
    options_default_fn: Option<&str>,
    enum_fields: &HashMap<String, String>,
    fixture: &crate::e2e::fixture::Fixture,
    _handle_struct_type: Option<&str>,
    _handle_atom_list_fields: &std::collections::HashSet<String>,
    test_documents_path: &str,
    adapter_request_type: Option<&str>,
    enums: &[crate::core::ir::EnumDef],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    force_keyword_args: bool,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // No args config: pass the whole input only when it's non-empty AND not just the harness setup dict.
        // Functions with no parameters (e.g. language_count) have empty input
        // and must be called with no arguments - not with `%{}`.
        // Filter out the harness' internal "setup" field - it's not part of the fixture's actual input.
        let cleaned_input = match input {
            serde_json::Value::Object(m) => {
                let mut cleaned = m.clone();
                cleaned.remove("setup");
                if cleaned.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::Object(cleaned)
                }
            }
            other => other.clone(),
        };
        let is_empty_input = matches!(cleaned_input, serde_json::Value::Null);
        if is_empty_input {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_elixir(&cleaned_input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // NOTE: Elixir requires all positional args before keyword args. To avoid syntax errors,
    // count how many optional args will be rendered as keywords upfront, then decide
    // whether json_object args should be positional or keyword. This aligns with the
    // Rustler backend's keyword-opts threshold: use keyword form for 2+ trailing optional
    // params, stay positional for 1 or 0.
    let trailing_keyword_count = args
        .iter()
        .rev()
        .take_while(|a| a.optional)
        .filter(|a| {
            // An arg will be rendered as keyword if it's optional AND has a provided value
            // that's not null. We can't fully evaluate this without checking the input,
            // but we can count optional params at the end - a conservative heuristic.
            a.arg_type != "mock_url" && a.arg_type != "mock_url_list" && a.arg_type != "handle"
        })
        .count();
    let use_keyword_form_for_optional_args = trailing_keyword_count >= 2;

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "{} = System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "{} = (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}_req", arg.name);
                setup_lines.push(format!("{req_var} = %{module_path}.{req_type}{{url: {}}}", arg.name,));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // list of URLs: each element is either a bare path (`/seed1`) - prefixed
            // with the per-fixture mock-server URL at runtime - or an absolute URL
            // kept as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>`
            // first, then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the
            // codegen falls back to a JSON-array literal of bare relative paths and
            // the Rust HTTP client rejects them.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_elixir(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "{name}_base = System.get_env(\"{env_key}\") || ((System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
            ));
            setup_lines.push(format!(
                "{name} = Enum.map([{paths_literal}], fn p -> if String.starts_with?(p, \"http\"), do: p, else: {name}_base <> p end)"
            ));
            parts.push(name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a create_{name} call using {:ok, name} = ... pattern.
            // The NIF now accepts config as an optional JSON string (not a NifStruct/NifMap)
            // so that partial maps work: serde_json::from_str respects #[serde(default)].
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let config_value = if arg.field == "input" {
                input
            } else {
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                input.get(field).unwrap_or(&serde_json::Value::Null)
            };
            let name = &arg.name;
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("{{:ok, {name}}} = {module_path}.{constructor_name}(nil)"));
            } else {
                // Serialize the config map to a JSON string with Jason so that Rust can
                // deserialize it with serde_json and apply field defaults for missing keys.
                let json_str = serde_json::to_string(config_value).unwrap_or_else(|_| "{}".to_string());
                let escaped = escape_elixir(&json_str);
                setup_lines.push(format!("{name}_config = \"{escaped}\""));
                setup_lines.push(format!(
                    "{{:ok, {name}}} = {module_path}.{constructor_name}({name}_config)",
                ));
            }
            parts.push(arg.name.clone());
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
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.name == super_trait) {
                            for method in &super_type.methods {
                                // Only add if not already present (avoid duplicates).
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    // Derive the NIF module from the test module path: the NIF module
                    // follows the "{AppModule}.Native" convention used by the Elixir scaffold.
                    let elixir_nif_module = format!("{module_path}.Native");
                    let emission = emit_test_backend(trait_bridge, &methods, fixture, &elixir_nif_module);

                    // Extract only the test-level setup part (after the marker).
                    // Module-level defs are emitted at file level by render_test_file, not here.
                    if let Some(pos) = emission.setup_block.find("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
                        let marker_end = emission.setup_block[pos + 32..]
                            .find('\n')
                            .map(|i| pos + 32 + i + 1)
                            .unwrap_or_else(|| emission.setup_block.len());
                        let test_setup = emission.setup_block[marker_end..].trim_start().to_string();
                        if !test_setup.is_empty() {
                            setup_lines.push(test_setup);
                        }
                    } else {
                        // Fallback for non-marker blocks (shouldn't happen for trait bridges)
                        setup_lines.push(emission.setup_block);
                    }

                    parts.push(emission.arg_expr);

                    // For register_fn traits (plugin pattern), Rustler requires a second "name" argument.
                    // Extract the backend name from fixture input (same logic as emit_test_backend).
                    if trait_bridge.register_fn.is_some() {
                        let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
                        parts.push(format!("\"{}\"", escape_elixir(&backend_name)));
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("elixir");
            setup_lines.push(format!("# {}", emission.arg_expr));
            parts.push("nil".to_string());
            continue;
        }

        let val = if arg.field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional params map to the keyword-opts `opts \\ []` argument.
                // When the value is absent, omit the keyword entirely - the default `[]` applies.
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For file_path args, prepend the path to the test_documents directory
                // relative to the e2e/elixir/ directory where `mix test` runs.
                if arg.arg_type == "file_path" {
                    if let Some(path_str) = v.as_str() {
                        let full_path = format!("{test_documents_path}/{path_str}");
                        let formatted = format!("\"{}\"", escape_elixir(&full_path));
                        if arg.optional {
                            parts.push(format!("{}: {formatted}", arg.name));
                        } else {
                            parts.push(formatted);
                        }
                        continue;
                    }
                }
                // For bytes args, use File.read! for file paths and Base.decode64! for base64.
                // Inline text (starts with '<', '{', '[' or contains spaces) is used as-is (UTF-8 binary).
                if arg.arg_type == "bytes" {
                    if let Some(raw) = v.as_str() {
                        let var_name = &arg.name;
                        if raw.starts_with('<') || raw.starts_with('{') || raw.starts_with('[') || raw.contains(' ') {
                            // Inline text - use as a binary string.
                            let formatted = format!("\"{}\"", escape_elixir(raw));
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                        } else {
                            let first = raw.chars().next().unwrap_or('\0');
                            let is_file_path = (first.is_ascii_alphanumeric() || first == '_')
                                && raw
                                    .find('/')
                                    .is_some_and(|slash_pos| slash_pos > 0 && raw[slash_pos + 1..].contains('.'));
                            if is_file_path {
                                // Looks like "dir/file.ext" - read from the
                                // configured test-documents directory.
                                let full_path = format!("{test_documents_path}/{raw}");
                                let escaped = escape_elixir(&full_path);
                                setup_lines.push(format!("{var_name} = File.read!(\"{escaped}\")"));
                                if arg.optional {
                                    parts.push(format!("{}: {var_name}", arg.name));
                                } else {
                                    parts.push(var_name.to_string());
                                }
                            } else {
                                // Treat as base64-encoded binary.
                                setup_lines.push(format!(
                                    "{var_name} = Base.decode64!(\"{}\", padding: false)",
                                    escape_elixir(raw)
                                ));
                                if arg.optional {
                                    parts.push(format!("{}: {var_name}", arg.name));
                                } else {
                                    parts.push(var_name.to_string());
                                }
                            }
                        }
                        continue;
                    }
                }
                // For json_object args with options_type+options_via, build a proper struct.
                if arg.arg_type == "json_object" && !v.is_null() {
                    let object_type = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v);
                    if let (Some(_opts_type), Some(options_fn), Some(obj)) =
                        (object_type, options_default_fn, v.as_object())
                    {
                        // Add setup line to initialize options from default function.
                        let options_var = format!("{}_value", arg.name);
                        setup_lines.push(format!("{options_var} = {module_path}.{options_fn}()"));

                        // For each field in the options object, add a struct update line.
                        for (k, vv) in obj.iter() {
                            let snake_key = k.to_snake_case();
                            let elixir_val = if let Some(_enum_type) = enum_fields.get(k) {
                                if let Some(s) = vv.as_str() {
                                    let snake_val = s.to_snake_case();
                                    // Use atom for enum values, not string
                                    format!(":{snake_val}")
                                } else {
                                    json_to_elixir(vv)
                                }
                            } else {
                                json_to_elixir(vv)
                            };
                            setup_lines.push(format!(
                                "{options_var} = %{{{options_var} | {snake_key}: {elixir_val}}}"
                            ));
                        }

                        // Push the variable name as the argument.
                        // Optional args (with `\\ []` or `\\ nil`) always use keyword form
                        // so that the facade can handle them via Keyword.get() or defaults.
                        parts.push(format!("{}: {options_var}", arg.name));
                        continue;
                    }
                    // When options_type is set but options_via is NOT, emit a struct-literal form.
                    // The auto-generated Rustler facade signature (`def f(html, options \\ nil)
                    // when is_map(options)`) requires a map, not a JSON string - and Elixir
                    // structs ARE maps, so a struct literal matches the guard. Falling through
                    // to the JSON-string emission below would yield `f(html, "{json}")`, which
                    // crashes the facade with FunctionClauseError. Emit positional/keyword
                    // form per `use_keyword_form_for_optional_args` to mirror the threshold
                    // applied to JSON-string emission.
                    if let (Some(opts_type), None, Some(obj)) = (object_type, options_default_fn, v.as_object()) {
                        let options_var = format!("{}_value", arg.name);
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            let base_var = format!("{}_mock_base_url", arg.name);
                            setup_lines.push(format!(
                                "{base_var} = System.get_env(\"{env_key}\") || \"#{{System.get_env(\"MOCK_SERVER_URL\")}}/fixtures/{fixture_id}\""
                            ));
                            let fields = render_struct_fields(obj, enum_fields, Some(&base_var));
                            setup_lines.push(format!("{options_var} = %{module_path}.{opts_type}{{{fields}}}"));
                            if use_keyword_form_for_optional_args && arg.optional {
                                parts.push(format!("{}: {options_var}", arg.name));
                            } else {
                                parts.push(options_var.to_string());
                            }
                            continue;
                        }
                        let fields = render_struct_fields(obj, enum_fields, None);
                        setup_lines.push(format!("{options_var} = %{module_path}.{opts_type}{{{fields}}}"));
                        if use_keyword_form_for_optional_args && arg.optional {
                            parts.push(format!("{}: {options_var}", arg.name));
                        } else {
                            parts.push(options_var.to_string());
                        }
                        continue;
                    }
                    if let Some(elem_type) = &arg.element_type {
                        // Internally-tagged enums (#[serde(tag = "type")]) - emit a list of
                        // Rustler NifTaggedEnum tuples. `:variant_atom` for unit variants,
                        // `{:variant_atom, %{field: value}}` for struct variants. Variant
                        // and field atoms are derived from Rust names via snake_case;
                        // Rustler's NifTaggedEnum decoder ignores serde renames.
                        if v.is_array()
                            && let Some(enum_def) = enums.iter().find(|e| &e.name == elem_type && e.serde_tag.is_some())
                        {
                            let formatted = emit_tagged_enum_array(v, enum_def, enums);
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                            continue;
                        }
                        // When element_type is set to a simple type (e.g. Vec<String>).
                        // The NIF accepts an Elixir list directly - emit one.
                        if v.is_array() {
                            if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                                let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                let base_var = format!("{}_mock_base_url", arg.name);
                                let json_var = format!("{}_json", arg.name);
                                let value_var = format!("{}_value", arg.name);
                                let formatted = json_to_elixir(v);
                                setup_lines.push(format!(
                                    "{base_var} = System.get_env(\"{env_key}\") || \"#{{System.get_env(\"MOCK_SERVER_URL\")}}/fixtures/{fixture_id}\""
                                ));
                                setup_lines.push(format!(
                                    "{json_var} = Jason.encode!({formatted}) |> String.replace(\"{}\", {base_var})",
                                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                ));
                                setup_lines.push(format!("{value_var} = Jason.decode!({json_var})"));
                                if arg.optional {
                                    parts.push(format!("{}: {value_var}", arg.name));
                                } else {
                                    parts.push(value_var);
                                }
                                continue;
                            }
                            let formatted = json_to_elixir(v);
                            if arg.optional {
                                parts.push(format!("{}: {formatted}", arg.name));
                            } else {
                                parts.push(formatted);
                            }
                            continue;
                        }
                    }
                    // When there's no options_type+options_via, the Elixir NIF expects a JSON
                    // string (Option<String> decoded by serde_json) rather than an Elixir map.
                    // Serialize the JSON value to a string literal here.
                    // Emit as positional or keyword based on trailing optional arg count.
                    // If 2+ trailing optional args exist, use keyword form to avoid mixing
                    // positional args after keyword args. Otherwise, stay positional for
                    // compatibility with positional-default style facades.
                    if !v.is_null() {
                        let json_str = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
                        let escaped = escape_elixir(&json_str);
                        let formatted = format!("\"{escaped}\"");
                        if use_keyword_form_for_optional_args && arg.optional {
                            parts.push(format!("{}: {formatted}", arg.name));
                        } else {
                            parts.push(formatted);
                        }
                        continue;
                    }
                }
                // Optional args use keyword-opts form: `name: value`.
                let elixir_val = json_to_elixir(v);
                if arg.optional {
                    parts.push(format!("{}: {elixir_val}", arg.name));
                } else {
                    parts.push(elixir_val);
                }
            }
        }
    }

    // Elixir requires all positional args before keyword args.
    // Separate positional and keyword args, preserving order within each group.
    // With the keyword-opts threshold applied above (use_keyword_form_for_optional_args),
    // we should never encounter a positional arg after a keyword arg.
    if force_keyword_args {
        let args_string = parts
            .into_iter()
            .zip(args.iter())
            .map(|(part, arg)| {
                let prefix = format!("{}: ", arg.name);
                if part.starts_with(&prefix) {
                    part
                } else {
                    format!("{}: {part}", arg.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        return (setup_lines, args_string);
    }

    let mut positional_args = Vec::new();
    let mut keyword_args = Vec::new();

    for part in parts {
        let is_keyword = part.contains(": ") && !part.starts_with('"');
        if is_keyword {
            keyword_args.push(part);
        } else {
            positional_args.push(part);
        }
    }

    let mut final_args = positional_args;
    final_args.extend(keyword_args);

    (setup_lines, final_args.join(", "))
}

/// Apply a serde `rename_all` strategy to a PascalCase variant name to derive
/// the wire-format tag value used in fixture inputs.
fn apply_rename_all(name: &str, strategy: Option<&str>) -> String {
    use heck::{ToKebabCase, ToLowerCamelCase, ToShoutyKebabCase, ToShoutySnakeCase, ToUpperCamelCase};
    match strategy {
        Some("snake_case") | None => name.to_snake_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_upper_camel_case(),
        Some("SCREAMING_SNAKE_CASE") | Some("UPPERCASE") => name.to_shouty_snake_case(),
        Some("kebab-case") => name.to_kebab_case(),
        Some("SCREAMING-KEBAB-CASE") => name.to_shouty_kebab_case(),
        Some("lowercase") => name.to_lowercase(),
        Some(_) => name.to_snake_case(),
    }
}

fn render_struct_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &HashMap<String, String>,
    mock_base_var: Option<&str>,
) -> String {
    obj.iter()
        .map(|(k, vv)| {
            let snake_key = k.to_snake_case();
            let elixir_val = if enum_fields.contains_key(k) {
                if let Some(s) = vv.as_str() {
                    let snake_val = s.to_snake_case();
                    format!(":{snake_val}")
                } else {
                    render_elixir_value(vv, mock_base_var)
                }
            } else {
                render_elixir_value(vv, mock_base_var)
            };
            format!("{snake_key}: {elixir_val}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_elixir_value(value: &serde_json::Value, mock_base_var: Option<&str>) -> String {
    if let Some(base_var) = mock_base_var
        && crate::e2e::codegen::value_contains_mock_url_placeholder(value)
    {
        match value {
            serde_json::Value::String(s) => format!(
                "String.replace(\"{}\", \"{}\", {base_var})",
                escape_elixir(s),
                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
            ),
            _ => {
                let value_literal = json_to_elixir(value);
                format!(
                    "Jason.decode!(Jason.encode!({value_literal}) |> String.replace(\"{}\", {base_var}), keys: :atoms)",
                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                )
            }
        }
    } else {
        json_to_elixir(value)
    }
}

/// Match an input JSON value (string) against a unit-only enum and return the
/// corresponding Rustler atom literal (e.g. `:down`). Returns None if the enum
/// is not unit-only or the value does not match any variant.
fn match_unit_enum_atom(value: &serde_json::Value, enum_def: &crate::core::ir::EnumDef) -> Option<String> {
    let s = value.as_str()?;
    if enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
        return None;
    }
    for variant in &enum_def.variants {
        let wire_tag = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        if wire_tag == s {
            return Some(format!(":{}", variant.name.to_snake_case()));
        }
    }
    None
}

/// Emit an Elixir list literal of Rustler NifTaggedEnum tuples for an internally-tagged
/// enum array. Each element renders as `:variant_atom` (unit) or
/// `{:variant_atom, %{field: value}}` (struct), with variant/field atoms derived
/// from the Rust names via snake_case (NifTaggedEnum ignores serde rename for atoms).
fn emit_tagged_enum_array(
    value: &serde_json::Value,
    enum_def: &crate::core::ir::EnumDef,
    all_enums: &[crate::core::ir::EnumDef],
) -> String {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return json_to_elixir(value),
    };
    let tag_key = enum_def.serde_tag.as_deref().unwrap_or("type");
    let mut elements: Vec<String> = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = match item.as_object() {
            Some(o) => o,
            None => {
                elements.push(json_to_elixir(item));
                continue;
            }
        };
        let tag_value = obj.get(tag_key).and_then(|v| v.as_str()).unwrap_or("");
        let matched = enum_def.variants.iter().find(|variant| {
            let wire_tag = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
            wire_tag == tag_value
        });
        let Some(variant) = matched else {
            elements.push(json_to_elixir(item));
            continue;
        };
        let variant_atom = format!(":{}", variant.name.to_snake_case());
        if variant.fields.is_empty() {
            elements.push(variant_atom);
            continue;
        }
        let mut field_strs: Vec<String> = Vec::with_capacity(variant.fields.len());
        for field in &variant.fields {
            let wire_field = field.serde_rename.as_deref().unwrap_or(&field.name);
            let rust_field_atom = field.name.clone();
            let emitted_val = if let Some(field_val) = obj.get(wire_field) {
                // If the field's type is a Named reference to a unit-only enum, convert
                // the input string value to an atom via that enum's rename_all.
                if let crate::core::ir::TypeRef::Named(type_name) = &field.ty {
                    all_enums
                        .iter()
                        .find(|e| &e.name == type_name && e.serde_tag.is_none())
                        .and_then(|nested| match_unit_enum_atom(field_val, nested))
                        .unwrap_or_else(|| json_to_elixir(field_val))
                } else {
                    json_to_elixir(field_val)
                }
            } else if field.optional {
                // Optional fields missing from the JSON should use `nil` as default
                "nil".to_string()
            } else {
                // Non-optional fields missing from the JSON should not be included
                // (could indicate an error in the fixture, but we skip for safety)
                continue;
            };
            field_strs.push(format!("{rust_field_atom}: {emitted_val}"));
        }
        let map_body = field_strs.join(", ");
        elements.push(format!("{{{variant_atom}, %{{{map_body}}}}}"));
    }
    format!("[{}]", elements.join(", "))
}

/// Extract the backend name from fixture input for register_fn traits.
///
/// Looks for a "name" field at the root or nested one level deep,
/// then falls back to the first string value encountered, then to the fallback.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}
