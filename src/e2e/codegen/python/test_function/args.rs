//! Argument binding and setup rendering for generated Python tests.

use std::collections::{HashMap, HashSet};

use heck::ToSnakeCase;

use crate::e2e::codegen::resolve_field;
use crate::e2e::fixture::Fixture;

use super::super::json::json_to_python_literal;
use super::typed_values::{emit_bytes_arg, emit_json_object_arg};

/// Build arg binding lines and kwarg expressions for a fixture call.
///
/// Returns `(arg_bindings, kwarg_exprs, teardown_block)`. The teardown block
/// contains statements emitted after the fixture call and its assertions —
/// trait-bridge fixtures populate it with `unregister_<trait>("<name>")` so
/// pytest's shared-process registry state is restored between tests.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    fixture: &Fixture,
    call_config: &crate::e2e::config::CallConfig,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> (Vec<String>, Vec<String>, String) {
    let mut arg_bindings = Vec::new();
    let mut kwarg_exprs = Vec::new();
    let mut teardown = String::new();

    for arg in fixture.resolved_args(call_config) {
        let var_name = &arg.name;

        if arg.arg_type == "handle" {
            emit_handle_arg(
                &mut arg_bindings,
                &mut kwarg_exprs,
                fixture,
                arg,
                var_name,
                options_type,
                handle_nested_types,
                handle_dict_types,
            );
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
                    let emission = super::super::emit_test_backend(trait_bridge, &methods, fixture);
                    arg_bindings.push(emission.setup_block);
                    kwarg_exprs.push(emission.arg_expr);
                    teardown.push_str(&emission.teardown_block);
                    continue;
                }
            }
            // Fall back to unimplemented if trait not found
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("python");
            arg_bindings.push(format!("    # {}", emission.arg_expr));
            kwarg_exprs.push("None".to_string());
            continue;
        }

        if arg.arg_type == "mock_url" {
            let fixture_id = &fixture.id;
            let url_expr = if fixture.has_host_root_route() {
                format!(
                    "os.environ.get('MOCK_SERVER_{}') or os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'")
            };
            arg_bindings.push(format!("    {var_name} = {url_expr}"));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            let fixture_id = &fixture.id;
            let base_url_expr = if fixture.has_host_root_route() {
                format!(
                    "os.environ.get('MOCK_SERVER_{}', os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}')",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'")
            };
            arg_bindings.push(format!("    {var_name}_base = {base_url_expr}"));

            // Extract path strings from fixture input array.
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let field_value = crate::e2e::codegen::resolve_urls_field(&fixture.input, &arg.field);
            let paths: Vec<String> = if let Some(arr) = field_value.as_array() {
                arr.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .map(|s| format!("\"{}\"", crate::e2e::escape::escape_python(s)))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let paths_str = paths.join(", ");

            arg_bindings.push(format!(
                "    {var_name} = [p if p.startswith('http') else f'{{{var_name}_base}}{{p}}' for p in [{paths_str}]]"
            ));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        let value = resolve_field(&fixture.input, &arg.field);

        if value.is_null() && arg.optional {
            // Emit None as a placeholder so subsequent positional args keep their
            // index alignment. With kwarg emission this would just be skipped, but
            // since we emit positional args (commit 40ff92c9), an omitted optional
            // arg in the middle would shift later args into the wrong position.
            kwarg_exprs.push("None".to_string());
            continue;
        }

        if arg.arg_type == "json_object"
            && !value.is_null()
            && emit_json_object_arg(
                &mut arg_bindings,
                &mut kwarg_exprs,
                value,
                var_name,
                crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, value),
                options_via,
                enum_fields,
                &arg.element_type,
                &fixture.id,
                fixture.has_host_root_route(),
                type_defs,
                enums,
            )
        {
            continue;
        }

        if arg.optional && value.is_null() {
            continue;
        }

        if value.is_null() && !arg.optional {
            let default_val = match arg.arg_type.as_str() {
                "string" => "\"\"".to_string(),
                "int" | "integer" => "0".to_string(),
                "float" | "number" => "0.0".to_string(),
                "bool" | "boolean" => "False".to_string(),
                _ => "None".to_string(),
            };
            arg_bindings.push(format!("    {var_name} = {default_val}"));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "bytes" {
            emit_bytes_arg(&mut arg_bindings, &mut kwarg_exprs, value, var_name);
            continue;
        }

        let literal = json_to_python_literal(value);
        let noqa = if literal.contains("/tmp/") {
            "  # noqa: S108"
        } else {
            ""
        };
        arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
        kwarg_exprs.push(var_name.to_string());
    }

    (arg_bindings, kwarg_exprs, teardown)
}

#[allow(clippy::too_many_arguments)]
fn emit_handle_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    fixture: &Fixture,
    arg: &crate::e2e::config::ArgMapping,
    var_name: &str,
    options_type: Option<&str>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) {
    let constructor_name = format!("create_{}", arg.name.to_snake_case());
    let config_value = resolve_field(&fixture.input, &arg.field);
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        arg_bindings.push(format!("    {var_name} = {constructor_name}(None)"));
    } else if let Some(obj) = config_value.as_object() {
        let kwargs: Vec<String> = obj
            .iter()
            .map(|(k, v)| {
                let snake_key = k.to_snake_case();
                let py_val = build_handle_kwarg_value(k, v, handle_nested_types, handle_dict_types);
                format!("{snake_key}={py_val}")
            })
            .collect();
        let config_class = options_type.unwrap_or_else(|| {
            panic!(
                "python e2e: handle arg `{}` requires `options_type` on the call config (set `[e2e.call] options_type = \"...\"` to the Python class name of the handle's config struct)",
                arg.name
            )
        });
        let single_line = format!("    {var_name}_config = {config_class}({})", kwargs.join(", "));
        if single_line.len() <= 120 {
            arg_bindings.push(single_line);
        } else {
            let mut lines = format!("    {var_name}_config = {config_class}(\n");
            for kw in &kwargs {
                lines.push_str(&format!("        {kw},\n"));
            }
            lines.push_str("    )");
            arg_bindings.push(lines);
        }
        arg_bindings.push(format!("    {var_name} = {constructor_name}({var_name}_config)"));
    } else {
        let literal = json_to_python_literal(config_value);
        arg_bindings.push(format!("    {var_name} = {constructor_name}({literal})"));
    }
    kwarg_exprs.push(var_name.to_string());
}

fn build_handle_kwarg_value(
    k: &str,
    v: &serde_json::Value,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) -> String {
    if let Some(type_name) = handle_nested_types.get(k) {
        if let Some(nested_obj) = v.as_object() {
            if nested_obj.is_empty() {
                return format!("{type_name}()");
            }
            if handle_dict_types.contains(k) {
                return json_to_python_literal(v);
            }
            let nested_kwargs: Vec<String> = nested_obj
                .iter()
                .map(|(nk, nv)| {
                    let nested_snake_key = nk.to_snake_case();
                    format!("{nested_snake_key}={}", json_to_python_literal(nv))
                })
                .collect();
            return format!("{type_name}({})", nested_kwargs.join(", "));
        }
    }
    if k == "request_timeout" {
        if let Some(ms) = v.as_u64() {
            return format!("{}", ms / 1000);
        }
    }
    json_to_python_literal(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_and_setup_empty_args_returns_empty_vecs() {
        use crate::e2e::fixture::Fixture;
        let fixture = Fixture {
            id: "t".to_string(),
            description: "d".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let call_config = crate::e2e::config::CallConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let (bindings, exprs, _teardown) = build_args_and_setup(
            &fixture,
            &call_config,
            None,
            "kwargs",
            &HashMap::new(),
            &HashMap::new(),
            &HashSet::new(),
            &config,
            &type_defs,
            &enums,
        );
        assert!(bindings.is_empty());
        assert!(exprs.is_empty());
    }
}
