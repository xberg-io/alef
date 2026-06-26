//! Go argument and setup rendering.

use crate::e2e::escape::go_string_literal;

use super::json_values::{convert_json_for_go, element_type_to_go_slice, json_to_go};
use super::test_backend::emit_test_backend_with_context;

fn json_object_go_type<'a>(arg: &'a crate::e2e::config::ArgMapping, options_type: Option<&'a str>) -> Option<&'a str> {
    arg.go_type.as_deref().or(arg.element_type.as_deref()).or(options_type)
}

fn qualified_go_type(import_alias: &str, type_name: &str) -> String {
    if type_name.contains('.') {
        type_name.to_string()
    } else {
        format!("{import_alias}.{type_name}")
    }
}

pub(super) fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        let candidate = format!("{}Config", arg.name.to_uppercase_first());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

trait UppercaseFirst {
    fn to_uppercase_first(&self) -> String;
}

impl UppercaseFirst for str {
    fn to_uppercase_first(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    import_alias: &str,
    options_type: Option<&str>,
    fixture: &crate::e2e::fixture::Fixture,
    options_ptr: bool,
    expects_error: bool,
    data_enum_names: &std::collections::HashSet<&str>,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> (Vec<String>, Vec<String>, String) {
    let fixture_id = &fixture.id;
    use heck::ToUpperCamelCase;

    if args.is_empty() {
        return (Vec::new(), Vec::new(), String::new());
    }

    let mut package_decls: Vec<String> = Vec::new();
    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!("{} := os.Getenv(\"{env_key}\")", arg.name));
                setup_lines.push(format!(
                    "if {} == \"\" {{ {} = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\" }}",
                    arg.name, arg.name
                ));
            } else {
                setup_lines.push(format!(
                    "{} := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);

            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter().filter_map(|v| v.as_str().map(go_string_literal)).collect()
            } else {
                Vec::new()
            };

            let paths_literal = paths.join(", ");
            let var_name = &arg.name;

            setup_lines.push(format!(
                "{var_name}Base := os.Getenv(\"{env_key}\")\n\tif {var_name}Base == \"\" {{\n\t\t{var_name}Base = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"\n\t}}"
            ));
            setup_lines.push(format!(
                "var {var_name} []string\n\tfor _, p := range []string{{{paths_literal}}} {{\n\t\tif strings.HasPrefix(p, \"http\") {{\n\t\t\t{var_name} = append({var_name}, p)\n\t\t}} else {{\n\t\t\t{var_name} = append({var_name}, {var_name}Base + p)\n\t\t}}\n\t}}"
            ));
            parts.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();

                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    let excluded_named =
                        crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(config, type_defs, &methods);
                    let enum_names: std::collections::HashSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
                    let emission = emit_test_backend_with_context(
                        trait_bridge,
                        &methods,
                        fixture,
                        &excluded_named,
                        import_alias,
                        &enum_names,
                        enums,
                    );
                    package_decls.push(emission.setup_block);
                    parts.push(emission.arg_expr);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("go");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("nil".to_string());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("Create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            let create_err_handler = if expects_error {
                "assert.Error(t, createErr)\n\t\treturn".to_string()
            } else {
                "t.Fatalf(\"create handle failed: %v\", createErr)".to_string()
            };
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!(
                    "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}",
                    name = arg.name,
                ));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let go_literal = go_string_literal(&json_str);
                let name = &arg.name;
                if let Some(config_type) = resolve_handle_config_type(arg, options_type, type_defs) {
                    setup_lines.push(format!(
                        "var {name}Config {import_alias}.{config_type}\n\tif err := json.Unmarshal([]byte({go_literal}), &{name}Config); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                    ));
                    setup_lines.push(format!(
                        "{name}, createErr := {import_alias}.{constructor_name}(&{name}Config)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}"
                    ));
                } else {
                    setup_lines.push(format!(
                        "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}"
                    ));
                }
            }
            parts.push(arg.name.clone());
            continue;
        }

        let val: Option<&serde_json::Value> = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };

        if arg.arg_type == "bytes" {
            let var_name = format!("{}Bytes", arg.name);
            match val {
                None | Some(serde_json::Value::Null) => {
                    if arg.optional {
                        parts.push("nil".to_string());
                    } else {
                        parts.push("[]byte{}".to_string());
                    }
                }
                Some(serde_json::Value::String(s)) => {
                    let go_path = go_string_literal(s);
                    setup_lines.push(format!(
                        "{var_name}, {var_name}Err := os.ReadFile({go_path})\n\tif {var_name}Err != nil {{\n\t\tt.Fatalf(\"read fixture {s}: %v\", {var_name}Err)\n\t}}"
                    ));
                    parts.push(var_name);
                }
                Some(other) => {
                    parts.push(format!("[]byte({})", json_to_go(other)));
                }
            }
            continue;
        }

        match val {
            None | Some(serde_json::Value::Null) if arg.optional => match arg.arg_type.as_str() {
                "string" => {
                    parts.push("nil".to_string());
                }
                "json_object" => {
                    if options_ptr {
                        parts.push("nil".to_string());
                    } else if let Some(opts_type) = json_object_go_type(arg, options_type) {
                        parts.push(format!("{}{{}}", qualified_go_type(import_alias, opts_type)));
                    } else {
                        parts.push("nil".to_string());
                    }
                }
                _ => {
                    parts.push("nil".to_string());
                }
            },
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" | "i64" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => {
                        if options_ptr {
                            "nil".to_string()
                        } else if let Some(opts_type) = json_object_go_type(arg, options_type) {
                            format!("{}{{}}", qualified_go_type(import_alias, opts_type))
                        } else {
                            "nil".to_string()
                        }
                    }
                    _ => "nil".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => match arg.arg_type.as_str() {
                "json_object" => {
                    let is_array = v.is_array();
                    let is_empty_obj = !is_array && v.is_object() && v.as_object().is_some_and(|o| o.is_empty());
                    if is_empty_obj {
                        if options_ptr {
                            parts.push("nil".to_string());
                        } else if let Some(opts_type) = json_object_go_type(arg, options_type) {
                            parts.push(format!("{}{{}}", qualified_go_type(import_alias, opts_type)));
                        } else {
                            parts.push("nil".to_string());
                        }
                    } else if is_array {
                        let go_slice_type = if let Some(go_t) = arg.go_type.as_deref() {
                            if go_t.starts_with('[') {
                                go_t.to_string()
                            } else {
                                let qualified = if go_t.contains('.') {
                                    go_t.to_string()
                                } else {
                                    format!("{import_alias}.{go_t}")
                                };
                                format!("[]{qualified}")
                            }
                        } else {
                            element_type_to_go_slice(arg.element_type.as_deref(), import_alias)
                        };

                        let element_type_name = if let Some(go_t) = arg.go_type.as_deref() {
                            if go_t.starts_with('[') {
                                None
                            } else if let Some(idx) = go_t.rfind('.') {
                                Some(&go_t[idx + 1..])
                            } else {
                                Some(go_t)
                            }
                        } else {
                            arg.element_type.as_deref()
                        };

                        let is_sum_type = element_type_name.is_some_and(|et| data_enum_names.contains(et));
                        let converted_v = convert_json_for_go(v.clone());
                        let var_name = &arg.name;

                        if is_sum_type {
                            let element_type = element_type_name.unwrap();
                            let json_str = serde_json::to_string(&converted_v).unwrap_or_default();
                            let go_literal = go_string_literal(&json_str);
                            setup_lines.push(format!(
                                "var {var_name}Raw []json.RawMessage\n\tif err := json.Unmarshal([]byte({go_literal}), &{var_name}Raw); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                            ));
                            setup_lines.push(format!(
                                "var {var_name} {go_slice_type}\n\tfor _, raw := range {var_name}Raw {{\n\t\telem, err := {import_alias}.Unmarshal{element_type}(raw)\n\t\tif err != nil {{\n\t\t\tt.Fatalf(\"unmarshal {element_type} failed: %v\", err)\n\t\t}}\n\t\t{var_name} = append({var_name}, elem)\n\t}}"
                            ));
                        } else {
                            let json_str = serde_json::to_string(&converted_v).unwrap_or_default();
                            let go_literal = go_string_literal(&json_str);
                            setup_lines.push(format!(
                                "var {var_name} {go_slice_type}\n\tif err := json.Unmarshal([]byte({go_literal}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                            ));
                        }
                        parts.push(var_name.to_string());
                    } else if let Some(opts_type) = json_object_go_type(arg, options_type) {
                        let remapped_v = if Some(opts_type) == options_type && options_ptr {
                            convert_json_for_go(v.clone())
                        } else {
                            v.clone()
                        };
                        let json_str = serde_json::to_string(&remapped_v).unwrap_or_default();
                        let go_literal = go_string_literal(&json_str);
                        let var_name = &arg.name;
                        let type_name = qualified_go_type(import_alias, opts_type);
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(&remapped_v) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            setup_lines.push(format!(
                                "{var_name}MockBaseURL := os.Getenv(\"{env_key}\")\n\tif {var_name}MockBaseURL == \"\" {{\n\t\t{var_name}MockBaseURL = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"\n\t}}"
                            ));
                            setup_lines.push(format!(
                                "{var_name}JSON := strings.ReplaceAll({go_literal}, \"{}\", {var_name}MockBaseURL)",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                        }
                        let json_expr = if crate::e2e::codegen::value_contains_mock_url_placeholder(&remapped_v) {
                            format!("{var_name}JSON")
                        } else {
                            go_literal
                        };
                        setup_lines.push(format!(
                            "var {var_name} {type_name}\n\tif err := json.Unmarshal([]byte({json_expr}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
                        ));
                        let arg_expr = if Some(opts_type) == options_type && options_ptr {
                            format!("&{var_name}")
                        } else {
                            var_name.to_string()
                        };
                        parts.push(arg_expr);
                    } else {
                        parts.push(json_to_go(v));
                    }
                }
                "string" if arg.optional => {
                    let var_name = format!("{}Val", arg.name);
                    let go_val = json_to_go(v);
                    setup_lines.push(format!("{var_name} := {go_val}"));
                    parts.push(format!("&{var_name}"));
                }
                _ => {
                    parts.push(json_to_go(v));
                }
            },
        }
    }

    (package_decls, setup_lines, parts.join(", "))
}
