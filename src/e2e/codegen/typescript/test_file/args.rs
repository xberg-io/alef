use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    options_type: Option<&str>,
    fixture: &crate::e2e::fixture::Fixture,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    handle_config_type: Option<&str>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // When the call has no configured args and the fixture input is an
        // empty object, emit no positional arguments. This lets `extra_args`
        // (e.g. `undefined`) become the sole call argument — matching the
        // shape expected by zero-arg or single-optional-arg functions like
        // `listFiles(query?)` in WASM, where passing `{}` would fail the
        // `instanceof` check.
        let runtime_input = strip_setup_metadata(input);
        if runtime_input
            .as_object()
            .map(|m| m.is_empty())
            .unwrap_or_else(|| runtime_input.is_null())
        {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_js(&runtime_input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // Check if any later arg (after current) is a json_object that will get a default value
    // (needed to insert undefineds as placeholders for earlier missing optional args)
    fn has_later_json_object_default(args: &[ArgMapping], from_idx: usize, input: &serde_json::Value) -> bool {
        args[from_idx..].iter().any(|arg| {
            if arg.arg_type != "json_object" || !arg.optional {
                return false;
            }
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field).is_none() || input.get(field).map(|v| v.is_null()).unwrap_or(true)
        })
    }

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let url_expr = if fixture.has_host_root_route() {
                format!(
                    "process.env.MOCK_SERVER_{} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`")
            };
            setup_lines.push(format!("const {} = {url_expr};", arg.name));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // string[] of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the codegen
            // falls back to a JSON-array literal of bare relative paths and the Rust
            // HTTP client rejects them.
            let fixture_id = &fixture.id;
            let env_upper = fixture_id.to_uppercase();
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let val = if let Some(v) = input.get(field).filter(|v| !v.is_null()) {
                v.clone()
            } else {
                crate::e2e::codegen::resolve_urls_field(input, &arg.field).clone()
            };
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_js(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "const {name}Base = process.env.MOCK_SERVER_{env_upper} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
            ));
            setup_lines.push(format!(
                "const {name} = [{paths_literal}].map((p) => p.startsWith(\"http\") ? p : {name}Base + p);"
            ));
            parts.push(name.clone());
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
                    let emission = crate::e2e::codegen::emit_test_backend(lang, trait_bridge, &methods, fixture);
                    setup_lines.push(emission.setup_block);
                    // Assign the bridge to a variable for NAPI cleanup
                    if lang == "node" {
                        let bridge_var = format!("_bridge_{}", arg.name);
                        setup_lines.push(format!("const {} = {};", bridge_var, emission.arg_expr));
                        parts.push(bridge_var);
                    } else {
                        parts.push(emission.arg_expr);
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented(lang);
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            let is_null_config = config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty());
            // WASM: std::env::var is unavailable on wasm32 so SsrfPolicy::from_env()
            // always returns deny_private=true. E2e suites target localhost (mock server),
            // so we must override ssrf.denyPrivate=false on every engine config.
            // Detect whether the config type exposes an `ssrf` field by checking the
            // WASM type-prefix: if the type_defs include an SsrfPolicy struct, we know
            // the binding exposes it. Emit the override whenever lang=="wasm" and the
            // handle has a config type.
            let wasm_has_ssrf_field = lang == "wasm"
                && handle_config_type.is_some()
                && type_defs.iter().any(|td| {
                    (td.name == "SsrfPolicy" || td.name.ends_with("SsrfPolicy"))
                        && td.fields.iter().any(|f| f.name == "deny_private")
                });
            if is_null_config && !wasm_has_ssrf_field {
                setup_lines.push(format!("const {} = {constructor_name}(null);", arg.name));
            } else if is_null_config && wasm_has_ssrf_field {
                // Null config but WASM needs SSRF override — materialise a default config.
                let config_type = handle_config_type.unwrap();
                setup_lines.push(format!(
                    "const {name}Config = {config_type}.default();",
                    name = arg.name
                ));
                setup_lines.push(format!("{name}Config.ssrf.denyPrivate = false;", name = arg.name));
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            } else {
                // WASM: if handle_config_type is set, use factory pattern + setters
                if let Some(config_type) = handle_config_type {
                    // Construct config object with setters
                    setup_lines.push(format!(
                        "const {name}Config = {config_type}.default();",
                        name = arg.name
                    ));
                    if let Some(obj) = config_value.as_object() {
                        // Derive nested types for the handle config type so nested objects
                        // are wrapped with their proper class constructors
                        let derived_nested = derive_nested_types_for_wasm(config_type, type_defs, wasm_type_prefix);
                        let effective_nested: std::collections::HashMap<String, String> = {
                            let mut m = derived_nested;
                            for (k, v) in nested_types {
                                m.insert(k.clone(), v.clone());
                            }
                            m
                        };

                        for (key, val) in obj {
                            let camel_key = snake_to_camel(key);
                            let value_expr = if let serde_json::Value::Object(nested_obj) = val {
                                if let Some(nested_type) = effective_nested.get(key.as_str()) {
                                    // Use builder expression for nested class types
                                    ts_builder_expression_inner(
                                        nested_obj,
                                        nested_type,
                                        nested_types,
                                        lang,
                                        enum_fields,
                                        bigint_fields,
                                        type_defs,
                                        enums,
                                        wasm_type_prefix,
                                        0,
                                    )
                                } else {
                                    json_to_js_camel(val)
                                }
                            } else {
                                json_to_js_camel(val)
                            };
                            setup_lines.push(format!("{name}Config.{camel_key} = {value_expr};", name = arg.name));
                        }
                    }
                    // WASM: inject ssrf.denyPrivate=false if the binding exposes SsrfPolicy.
                    // E2e suites hit localhost; std::env::var is unavailable on wasm32 so
                    // SsrfPolicy::from_env() cannot read private-network override environment.
                    if wasm_has_ssrf_field {
                        setup_lines.push(format!("{name}Config.ssrf.denyPrivate = false;", name = arg.name));
                    }
                } else {
                    // Other languages: pass config object directly or via constructor
                    let literal = json_to_js_camel(config_value);
                    setup_lines.push(format!("const {name}Config = {literal};", name = arg.name));
                }
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let runtime_input;
        let val = if field == "input" {
            runtime_input = strip_setup_metadata(input);
            Some(&runtime_input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // For optional json_object args, pass `undefined` so we keep argument
                // positions intact without needing a placeholder value. The previous
                // `{} as OptionsType` pattern broke wasm-bindgen, where the runtime
                // `instanceof` check rejected plain object literals — wasm exposes
                // options as opaque classes, not interfaces.
                if arg.arg_type == "json_object"
                    || has_later_arg_value(args, idx + 1, input)
                    || has_later_json_object_default(args, idx + 1, input)
                {
                    parts.push("undefined".to_string());
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "bytes" {
                    // For bytes type, if value is a string path, read the file
                    if let Some(path) = v.as_str() {
                        let var_name = format!("_{}_content", sanitize_ident(&arg.name));
                        setup_lines.push(format!(
                            "const {var_name} = await (await import(\"node:fs/promises\")).readFile(\"{}\");",
                            escape_js(path)
                        ));
                        parts.push(var_name);
                    } else {
                        // Binary array fallback
                        parts.push(format!("Buffer.from({})", json_to_js(v)));
                    }
                } else if arg.arg_type == "json_object" {
                    if v.is_array() {
                        // Array args use fixture-shaped object literals; element_type is
                        // still used by typed bindings/imports, not product-specific constructors.
                        parts.push(json_to_js_camel(v));
                    } else if let Some(raw_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v)
                    {
                        let opts_type = canonical_ts_type_name(lang, raw_type, config);
                        // Object value with known options type — construct properly for wasm-bindgen.
                        if v.is_object() && v.as_object().is_some_and(|o| o.is_empty()) {
                            // Empty options: pass undefined so wasm-bindgen's instanceof
                            // guard accepts the call (a `{}` cast produces a plain literal
                            // that fails the runtime class check).
                            parts.push("undefined".to_string());
                        } else if let Some(obj) = v.as_object() {
                            if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                                let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                let var_prefix = sanitize_ident(&arg.name);
                                setup_lines.push(format!(
                                    "const {var_prefix}MockBaseUrl = process.env.{env_key} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
                                ));
                                let json_literal = json_to_js_camel(v);
                                setup_lines.push(format!(
                                    "const {var_prefix}Json = JSON.stringify({json_literal}).replaceAll(\"{}\", {var_prefix}MockBaseUrl);",
                                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                ));
                                setup_lines.push(format!(
                                    "const {name} = JSON.parse({var_prefix}Json) as {opts_type};",
                                    name = arg.name
                                ));
                                parts.push(arg.name.clone());
                                continue;
                            }
                            // Build TypeScript code to construct the options object properly,
                            // handling nested types via their static factory methods.
                            let ts_code = ts_builder_expression(
                                obj,
                                &opts_type,
                                nested_types,
                                lang,
                                enum_fields,
                                bigint_fields,
                                type_defs,
                                enums,
                                wasm_type_prefix,
                            );
                            parts.push(ts_code);
                        } else {
                            parts.push(format!("{} as {opts_type}", json_to_js_camel(v)));
                        }
                    } else if lang == "node" {
                        // For node (napi-rs), tagged-data enum discriminants are
                        // always exposed as `"kind"` in TypeScript, regardless of the
                        // original Rust serde_tag attribute. Pre-process the JSON to
                        // rename serde_tag keys (e.g. `role`, `type`) to `"kind"` when
                        // the value matches a known enum variant, then convert to JS.
                        let preprocessed = rename_napi_serde_tags_to_kind(v, enums);
                        parts.push(json_to_js_camel(&preprocessed));
                    } else {
                        parts.push(json_to_js_camel(v));
                    }
                    continue;
                } else {
                    parts.push(json_to_js(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "))
}
