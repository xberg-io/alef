//! Python test file generation — import resolution and orchestration.

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;

use heck::ToSnakeCase;

use crate::codegen::resolve_field;
use crate::config::E2eConfig;
use crate::fixture::Fixture;
use alef_core::hash::{self, CommentStyle};

use super::helpers::{
    BytesKind, classify_bytes_value, is_skipped, python_method_helper_import, resolve_client_factory,
    resolve_enum_fields, resolve_function_name, resolve_function_name_for_call, resolve_handle_dict_types,
    resolve_handle_nested_types, resolve_module, resolve_options_type, resolve_options_via,
};
use super::http::render_http_test_function;
use super::test_function::render_test_function;

use crate::field_access::FieldResolver;

/// Render a complete Python test file for a single fixture category.
pub(super) fn render_test_file(category: &str, fixtures: &[&Fixture], e2e_config: &E2eConfig) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "\"\"\"E2e tests for category: {category}.\"\"\"");

    let module = resolve_module(e2e_config);
    let function_name = resolve_function_name(e2e_config);
    let options_type = resolve_options_type(e2e_config);
    let options_via = resolve_options_via(e2e_config);

    // Prefer the global override; fall back to the first fixture's per-call override.
    let effective_options_type: Option<String> = options_type.clone().or_else(|| {
        fixtures.iter().find_map(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.overrides.get("python").and_then(|o| o.options_type.clone())
        })
    });
    let effective_options_via: &str = if options_via != "kwargs" {
        options_via
    } else {
        fixtures
            .iter()
            .find_map(|f| {
                let cc = e2e_config.resolve_call(f.call.as_deref());
                cc.overrides.get("python").and_then(|o| o.options_via.as_deref())
            })
            .unwrap_or(options_via)
    };

    let enum_fields = resolve_enum_fields(e2e_config);
    let handle_nested_types = resolve_handle_nested_types(e2e_config);
    let handle_dict_types = resolve_handle_dict_types(e2e_config);
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
    );

    let has_error_test = fixtures
        .iter()
        .any(|f| f.assertions.iter().any(|a| a.assertion_type == "error"));
    let has_skipped = fixtures.iter().any(|f| is_skipped(f, "python"));
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    // File-level is_async: true if ANY fixture in this file will emit an async test function.
    // The Python CallOverride `async` field takes precedence per-fixture over the call-level
    // `async` flag. For the file-level import decision we need the union across all fixtures.
    let global_python_async_override = e2e_config.call.overrides.get("python").and_then(|o| o.r#async);
    let is_async = global_python_async_override.unwrap_or_else(|| {
        fixtures.iter().any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            let per_fixture_override = cc.overrides.get("python").and_then(|o| o.r#async);
            per_fixture_override.unwrap_or(cc.r#async)
        }) || e2e_config.call.r#async
    });
    let has_env_api_key = fixtures
        .iter()
        .any(|f| f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some());
    let needs_pytest = has_error_test || has_skipped || is_async || has_env_api_key;

    let needs_json_import = effective_options_via == "json"
        && fixtures.iter().any(|f| {
            e2e_config
                .call
                .args
                .iter()
                .any(|arg| arg.arg_type == "json_object" && !resolve_field(&f.input, &arg.field).is_null())
        });

    let client_factory = resolve_client_factory(e2e_config);
    let needs_os_import = client_factory.is_some() || e2e_config.call.args.iter().any(|arg| arg.arg_type == "mock_url");

    // When options_via == "from_json", the options_type is imported from a separate native
    // module (e.g., the PyO3 _internal_bindings) rather than the main public module.
    let from_json_module: Option<String> = e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.from_json_module.clone())
        .or_else(|| {
            fixtures.iter().find_map(|f| {
                let cc = e2e_config.resolve_call(f.call.as_deref());
                cc.overrides.get("python").and_then(|o| o.from_json_module.clone())
            })
        });

    let needs_path_import = fixtures.iter().any(|f| {
        let cc = e2e_config.resolve_call(f.call.as_deref());
        cc.args.iter().any(|arg| {
            if arg.arg_type != "bytes" {
                return false;
            }
            let val = resolve_field(&f.input, &arg.field);
            val.as_str()
                .is_some_and(|s| matches!(classify_bytes_value(s), BytesKind::FilePath))
        })
    });
    let needs_base64_import = fixtures.iter().any(|f| {
        let cc = e2e_config.resolve_call(f.call.as_deref());
        cc.args.iter().any(|arg| {
            if arg.arg_type != "bytes" {
                return false;
            }
            let val = resolve_field(&f.input, &arg.field);
            val.as_str()
                .is_some_and(|s| matches!(classify_bytes_value(s), BytesKind::Base64))
        })
    });

    let _ = has_http_tests;

    let needs_options_type = (effective_options_via == "kwargs" || effective_options_via == "from_json")
        && effective_options_type.is_some()
        && fixtures.iter().any(|f| {
            e2e_config
                .call
                .args
                .iter()
                .any(|arg| arg.arg_type == "json_object" && !resolve_field(&f.input, &arg.field).is_null())
        });

    let mut used_enum_types: BTreeSet<String> = BTreeSet::new();
    if needs_options_type && !enum_fields.is_empty() {
        for fixture in fixtures.iter() {
            for arg in &e2e_config.call.args {
                if arg.arg_type == "json_object" {
                    let value = resolve_field(&fixture.input, &arg.field);
                    if let Some(obj) = value.as_object() {
                        for key in obj.keys() {
                            if let Some(enum_type) = enum_fields.get(key) {
                                used_enum_types.insert(enum_type.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut stdlib_imports: Vec<String> = Vec::new();
    let mut thirdparty_bare: Vec<String> = Vec::new();
    let mut thirdparty_from: Vec<String> = Vec::new();

    if needs_base64_import {
        stdlib_imports.push("import base64".to_string());
    }
    if needs_json_import {
        stdlib_imports.push("import json".to_string());
    }
    if needs_os_import {
        stdlib_imports.push("import os".to_string());
    }
    if needs_path_import {
        stdlib_imports.push("from pathlib import Path".to_string());
    }
    if needs_pytest {
        thirdparty_bare.push("import pytest  # noqa: F401".to_string());
    }

    let has_non_http_fixtures = fixtures
        .iter()
        .any(|f| !f.is_http_test() && !is_skipped(f, "python") && !f.assertions.is_empty());
    if has_non_http_fixtures {
        build_thirdparty_imports(
            fixtures,
            e2e_config,
            &module,
            &function_name,
            client_factory.as_deref(),
            &effective_options_type,
            effective_options_via,
            from_json_module.as_deref(),
            needs_options_type,
            enum_fields,
            handle_nested_types,
            &used_enum_types,
            &mut thirdparty_from,
        );
    }

    stdlib_imports.sort();
    thirdparty_bare.sort();
    thirdparty_from.sort();

    if !stdlib_imports.is_empty() {
        for imp in &stdlib_imports {
            let _ = writeln!(out, "{imp}");
        }
        let _ = writeln!(out);
    }
    for imp in &thirdparty_bare {
        let _ = writeln!(out, "{imp}");
    }
    for imp in &thirdparty_from {
        let _ = writeln!(out, "{imp}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out);
    render_item_text_helper(&mut out);

    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_function(&mut out, fixture);
        } else if !is_skipped(fixture, "python") && fixture.assertions.is_empty() {
            emit_skipped_placeholder(&mut out, fixture);
        } else {
            render_test_function(
                &mut out,
                fixture,
                e2e_config,
                effective_options_type.as_deref(),
                effective_options_via,
                enum_fields,
                handle_nested_types,
                handle_dict_types,
                &field_resolver,
            );
        }
        let _ = writeln!(out);
    }

    out
}

fn render_item_text_helper(out: &mut String) {
    let _ = writeln!(out, "def _alef_e2e_text(value: object) -> str:");
    let _ = writeln!(out, "    return \"\" if value is None else str(value)");
    let _ = writeln!(out);
    let _ = writeln!(out);
    let _ = writeln!(out, "def _alef_e2e_item_texts(item: object) -> tuple[str, ...]:");
    let _ = writeln!(out, "    raw_items = getattr(item, \"items\", None)");
    let _ = writeln!(
        out,
        "    items_text = \" \".join(str(value) for value in raw_items) if isinstance(raw_items, list) else \"\""
    );
    let _ = writeln!(out, "    return (");
    let _ = writeln!(out, "        _alef_e2e_text(item),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"kind\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"name\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"source\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"alias\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"text\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"signature\", None)),");
    let _ = writeln!(out, "        items_text,");
    let _ = writeln!(out, "    )");
    let _ = writeln!(out);
    let _ = writeln!(out);
}

fn emit_skipped_placeholder(out: &mut String, fixture: &Fixture) {
    use crate::escape::sanitize_ident;
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };
    let _ = writeln!(
        out,
        "@pytest.mark.skip(reason=\"no assertions configured for this fixture in python e2e\")"
    );
    let _ = writeln!(out, "def test_{fn_name}() -> None:");
    let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");
}

#[allow(clippy::too_many_arguments)]
fn build_thirdparty_imports(
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module: &str,
    function_name: &str,
    client_factory: Option<&str>,
    options_type: &Option<String>,
    options_via: &str,
    from_json_module: Option<&str>,
    needs_options_type: bool,
    enum_fields: &std::collections::HashMap<String, String>,
    handle_nested_types: &std::collections::HashMap<String, String>,
    used_enum_types: &BTreeSet<String>,
    thirdparty_from: &mut Vec<String>,
) {
    let handle_constructors: Vec<String> = e2e_config
        .call
        .args
        .iter()
        .filter(|arg| arg.arg_type == "handle")
        .map(|arg| format!("create_{}", arg.name.to_snake_case()))
        .collect();

    let mut import_names: Vec<String> = Vec::new();

    // When a client_factory is configured, import only the factory function.
    // Individual API functions are called as methods on the client instance.
    if let Some(factory) = client_factory {
        import_names.push(factory.to_string());
    } else {
        for fixture in fixtures.iter() {
            let cc = e2e_config.resolve_call(fixture.call.as_deref());
            let fn_name = resolve_function_name_for_call(cc);
            if !import_names.contains(&fn_name) {
                import_names.push(fn_name);
            }
        }
        if import_names.is_empty() {
            import_names.push(function_name.to_string());
        }
    }
    for ctor in &handle_constructors {
        if !import_names.contains(ctor) {
            import_names.push(ctor.clone());
        }
    }

    let needs_config_import = e2e_config.call.args.iter().any(|arg| {
        arg.arg_type == "handle"
            && fixtures.iter().any(|f| {
                let val = resolve_field(&f.input, &arg.field);
                !val.is_null() && val.as_object().is_some_and(|o| !o.is_empty())
            })
    });
    if needs_config_import {
        let config_class = options_type.as_deref().unwrap_or("CrawlConfig");
        if !import_names.contains(&config_class.to_string()) {
            import_names.push(config_class.to_string());
        }
    }

    if !handle_nested_types.is_empty() {
        let mut used_nested_types: BTreeSet<String> = BTreeSet::new();
        for fixture in fixtures.iter() {
            for arg in &e2e_config.call.args {
                if arg.arg_type == "handle" {
                    let config_value = resolve_field(&fixture.input, &arg.field);
                    if let Some(obj) = config_value.as_object() {
                        for key in obj.keys() {
                            if let Some(type_name) = handle_nested_types.get(key) {
                                if obj[key].is_object() {
                                    used_nested_types.insert(type_name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        for type_name in used_nested_types {
            if !import_names.contains(&type_name) {
                import_names.push(type_name);
            }
        }
    }

    for fixture in fixtures.iter() {
        for assertion in &fixture.assertions {
            if assertion.assertion_type == "method_result" {
                if let Some(method_name) = &assertion.method {
                    if let Some(name) = python_method_helper_import(method_name) {
                        if !import_names.contains(&name) {
                            import_names.push(name);
                        }
                    }
                }
            }
        }
    }

    if let (true, Some(opts_type)) = (
        needs_options_type && (options_via == "kwargs" || options_via == "from_json"),
        options_type,
    ) {
        if options_via == "from_json" {
            // Import opts_type from the native bindings module (e.g., PyO3 _internal_bindings),
            // not the public module — it needs the native from_json() staticmethod.
            thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
            let native_mod = from_json_module.unwrap_or(module);
            thirdparty_from.push(format!("from {native_mod} import {opts_type}"));
        } else {
            import_names.push(opts_type.clone());
            thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
        }
        if !used_enum_types.is_empty() {
            let enum_mod = e2e_config
                .call
                .overrides
                .get("python")
                .and_then(|o| o.enum_module.as_deref())
                .unwrap_or(module);
            let enum_names: Vec<&String> = used_enum_types.iter().collect();
            thirdparty_from.push(format!(
                "from {enum_mod} import {}",
                enum_names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
    } else {
        thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
    }

    // Also collect per-fixture options_type from per-call overrides that use from_json.
    // This handles test files where different calls use different request types.
    let mut extra_from_json_types: BTreeSet<String> = BTreeSet::new();
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call(fixture.call.as_deref());
        if let Some(py_override) = cc.overrides.get("python") {
            if py_override.options_via.as_deref() == Some("from_json") {
                if let Some(opts_type) = &py_override.options_type {
                    let native_mod = py_override.from_json_module.as_deref().unwrap_or(module);
                    extra_from_json_types.insert(format!("from {native_mod} import {opts_type}"));
                }
            }
        }
    }
    for imp in extra_from_json_types {
        if !thirdparty_from.contains(&imp) {
            thirdparty_from.push(imp);
        }
    }

    let _ = enum_fields;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escape::sanitize_filename;
    use crate::fixture::FixtureGroup;

    fn test_filenames(groups: &[FixtureGroup]) -> Vec<String> {
        groups
            .iter()
            .map(|g| format!("test_{}.py", sanitize_filename(&g.category)))
            .collect()
    }

    #[test]
    fn test_filenames_produces_snake_case_names() {
        let groups = vec![
            FixtureGroup {
                category: "MyCategory".to_string(),
                fixtures: Vec::new(),
            },
            FixtureGroup {
                category: "another-thing".to_string(),
                fixtures: Vec::new(),
            },
        ];
        let names = test_filenames(&groups);
        assert_eq!(names[0], "test_mycategory.py");
        assert_eq!(names[1], "test_another_thing.py");
    }

    #[test]
    fn render_test_file_no_fixtures_produces_header_only() {
        let fixtures: Vec<&crate::fixture::Fixture> = Vec::new();
        let e2e_config = crate::config::E2eConfig::default();
        let out = render_test_file("basic", &fixtures, &e2e_config);
        assert!(out.contains("E2e tests for category: basic"), "got: {out}");
    }

    #[test]
    fn emit_skipped_placeholder_contains_skip_decorator() {
        let fixture = crate::fixture::Fixture {
            id: "foo_bar".to_string(),
            description: "Some test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            visitor: None,
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let mut out = String::new();
        emit_skipped_placeholder(&mut out, &fixture);
        assert!(out.contains("pytest.mark.skip"), "got: {out}");
        assert!(out.contains("test_foo_bar"), "got: {out}");
    }
}
