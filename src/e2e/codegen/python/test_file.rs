//! Python test file generation — import resolution and orchestration.

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;

use heck::ToSnakeCase;

use crate::core::hash::{self, CommentStyle};
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::helpers::{
    BytesKind, classify_bytes_value, is_skipped, python_method_helper_import, resolve_client_factory,
    resolve_enum_fields, resolve_function_name, resolve_function_name_for_call, resolve_handle_dict_types,
    resolve_handle_nested_types, resolve_module, resolve_options_type, resolve_options_via,
};
use super::http::render_http_test_function;
use super::test_function::{render_test_function, resolve_field_enum_type};

/// Render a complete Python test file for a single fixture category.
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> String {
    let module = resolve_module(e2e_config);
    let function_name = resolve_function_name(e2e_config);
    let options_type = resolve_options_type(e2e_config);
    let options_via = resolve_options_via(e2e_config);

    // Prefer the global python override; fall back to the first fixture's per-call
    // python override; then the call-level binding-agnostic `options_type`
    // (`[e2e.call] options_type` or `[e2e.calls.<name>] options_type`), which is
    // identical across every binding when the config-class name doesn't differ per language.
    let effective_options_type: Option<String> = options_type.clone().or_else(|| {
        fixtures.iter().find_map(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.overrides
                .get("python")
                .and_then(|o| o.options_type.clone())
                .or_else(|| cc.options_type.clone())
        })
    });
    let effective_options_via: &str = if options_via != "kwargs" {
        options_via
    } else {
        fixtures
            .iter()
            .find_map(|f| {
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                cc.overrides.get("python").and_then(|o| o.options_via.as_deref())
            })
            .unwrap_or(options_via)
    };

    let enum_fields = resolve_enum_fields(e2e_config);
    let handle_nested_types = resolve_handle_nested_types(e2e_config);
    let handle_dict_types = resolve_handle_dict_types(e2e_config);

    let has_error_test = fixtures
        .iter()
        .any(|f| f.assertions.iter().any(|a| a.assertion_type == "error"));
    let has_http_tests = fixtures.iter().any(|f| f.is_http_test());

    // File-level is_async: true if ANY fixture in this file will emit an async test function.
    // The Python CallOverride `async` field takes precedence per-fixture over the call-level
    // `async` flag. For the file-level import decision we need the union across all fixtures.
    // Streaming fixtures also emit async tests, so we must check that too — otherwise files
    // with streaming-only async would omit `import pytest`.
    let global_python_async_override = e2e_config.call.overrides.get("python").and_then(|o| o.r#async);
    let is_async = global_python_async_override.unwrap_or_else(|| {
        fixtures.iter().any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            let per_fixture_override = cc.overrides.get("python").and_then(|o| o.r#async);
            per_fixture_override.unwrap_or(cc.r#async)
                || crate::e2e::codegen::streaming_assertions::resolve_is_streaming(f, cc.streaming_enabled())
        }) || e2e_config.call.r#async
    });
    let has_env_api_key = fixtures
        .iter()
        .any(|f| f.env.as_ref().and_then(|e| e.api_key_var.as_ref()).is_some());
    let needs_pytest = has_error_test || is_async || has_env_api_key;

    let has_mock_url_placeholder = fixtures.iter().any(|f| {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        cc.args.iter().any(|arg| {
            arg.arg_type == "json_object"
                && crate::e2e::codegen::value_contains_mock_url_placeholder(resolve_field(&f.input, &arg.field))
        })
    });

    let needs_json_import = has_mock_url_placeholder
        || effective_options_via == "json"
            && fixtures.iter().any(|f| {
                e2e_config
                    .call
                    .args
                    .iter()
                    .any(|arg| arg.arg_type == "json_object" && !resolve_field(&f.input, &arg.field).is_null())
            });

    let client_factory = resolve_client_factory(e2e_config);
    let needs_os_import = client_factory.is_some()
        || has_mock_url_placeholder
        || e2e_config
            .call
            .args
            .iter()
            .any(|arg| arg.arg_type == "mock_url" || arg.arg_type == "mock_url_list");

    // When options_via == "from_json", the options_type is imported from a separate native
    // module (e.g., the PyO3 _internal_bindings) rather than the main public module.
    let from_json_module: Option<String> = e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.from_json_module.clone())
        .or_else(|| {
            fixtures.iter().find_map(|f| {
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                cc.overrides.get("python").and_then(|o| o.from_json_module.clone())
            })
        });

    let needs_path_import = fixtures.iter().any(|f| {
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
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
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
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
    let mut used_config_types: BTreeSet<String> = BTreeSet::new();

    // Collect all enum and config types referenced in call arguments.
    // Enum types come from two sources:
    // 1. Explicitly configured enum_fields (e.g., [e2e.call] enum_fields = {"format": "OutputFormat"})
    // 2. Auto-detected enum field types in the options_type via resolve_field_enum_type
    // Config types are top-level named types used as constructor arguments (e.g., EmbeddingConfig).
    for fixture in fixtures.iter() {
        // Resolve the per-fixture call config so we iterate the actual args that
        // will be rendered. The global `e2e_config.call.args` covers only the
        // default call (e.g. extract_file); fixtures that opt into a different
        // call via `"call": "embed_texts"` need their own args + options_type,
        // otherwise the rendered constructor (`EmbeddingConfig(...)`) never
        // gets a matching import and the test fails with `NameError`.
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let fixture_opts_type: Option<String> = cc
            .overrides
            .get("python")
            .and_then(|o| o.options_type.clone())
            .or_else(|| cc.options_type.clone())
            .or_else(|| effective_options_type.clone());

        for arg in &cc.args {
            let value = resolve_field(&fixture.input, &arg.field);

            // For json_object args, collect both enum types and config types.
            if arg.arg_type == "json_object" && !value.is_null() {
                let constructor_type =
                    crate::e2e::codegen::recipe::json_object_constructor_type(arg, fixture_opts_type.as_deref(), value);
                if let Some(obj) = value.as_object() {
                    // Collect explicitly configured enum fields. Auto-detected
                    // enums (resolve_field_enum_type below) must mirror the
                    // render path in test_function.rs — otherwise the kwarg
                    // builder will emit `OutputFormat.MARKDOWN` while this
                    // import collector never adds `OutputFormat`, producing a
                    // `NameError` at test runtime.
                    for key in obj.keys() {
                        if let Some(enum_type) = enum_fields.get(key) {
                            used_enum_types.insert(enum_type.clone());
                        } else if let Some(auto_enum_type) =
                            resolve_field_enum_type(key, constructor_type, type_defs, enums)
                        {
                            used_enum_types.insert(auto_enum_type);
                        }
                    }
                }
                // Collect the config type itself (e.g., ExtractionConfig, EmbeddingConfig)
                if let Some(opts_type) = constructor_type {
                    if !value.is_null() && value.is_object() {
                        // This is a constructor call like ExtractionConfig(...), so import the type
                        used_config_types.insert(opts_type.to_string());
                    }
                }
            }

            // For handle args, collect constructor types referenced by element_type
            if arg.arg_type == "handle" {
                if let Some(elem_type) = &arg.element_type {
                    // Only import if it's a named type (not a primitive)
                    let is_primitive = matches!(
                        elem_type.as_str(),
                        "str"
                            | "int"
                            | "float"
                            | "bool"
                            | "bytes"
                            | "list"
                            | "dict"
                            | "tuple"
                            | "Any"
                            | "String"
                            | "&str"
                            | "char"
                            | "u8"
                            | "u16"
                            | "u32"
                            | "u64"
                            | "u128"
                            | "usize"
                            | "i8"
                            | "i16"
                            | "i32"
                            | "i64"
                            | "i128"
                            | "isize"
                            | "f32"
                            | "f64"
                    );
                    if !is_primitive {
                        used_config_types.insert(elem_type.clone());
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
            config,
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
            &used_config_types,
            &mut thirdparty_from,
        );
    }

    stdlib_imports.sort();
    thirdparty_bare.sort();
    thirdparty_from.sort();

    // Render helper functions
    let mut helper_functions = String::new();
    render_item_text_helper(&mut helper_functions);

    // Render all fixtures
    let mut fixtures_body = String::new();
    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_function(&mut fixtures_body, fixture);
        } else {
            render_test_function(
                &mut fixtures_body,
                fixture,
                e2e_config,
                config,
                type_defs,
                enums,
                effective_options_type.as_deref(),
                effective_options_via,
                enum_fields,
                handle_nested_types,
                handle_dict_types,
            );
        }
        let _ = writeln!(fixtures_body);
    }

    // Render using template
    let ctx = minijinja::context! {
        header => hash::header(CommentStyle::Hash),
        docstring => format!("E2e tests for category: {category}."),
        stdlib_imports => stdlib_imports,
        thirdparty_bare => thirdparty_bare,
        thirdparty_from => thirdparty_from,
        helper_functions => helper_functions,
        fixtures_body => fixtures_body,
    };
    crate::e2e::template_env::render("python/test_file.jinja", ctx)
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

#[allow(clippy::too_many_arguments)]
fn build_thirdparty_imports(
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
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
    used_config_types: &BTreeSet<String>,
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
            let cc = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
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

    // Trait-bridge tests emit a teardown like `unregister_ocr_backend("test-backend")`
    // after the registration call. The unregister fn must also be imported from the
    // public binding module, or the test fails at runtime with NameError.
    // Use fixture.resolved_args(cc) to respect fixture-level args that override call-level args.
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        for arg in fixture.resolved_args(cc) {
            if arg.arg_type != "test_backend" {
                continue;
            }
            let Some(trait_name) = arg.trait_name.as_deref() else {
                continue;
            };
            if let Some(bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == trait_name) {
                if let Some(unregister_fn) = bridge.unregister_fn.as_deref() {
                    let unregister_str = unregister_fn.to_string();
                    if !import_names.contains(&unregister_str) {
                        import_names.push(unregister_str);
                    }
                }
            }
        }
    }

    // Import any element_type referenced by a call arg (e.g. `FileJob`, `PageAction`).
    // These names are emitted as bare references inside the test body (constructor calls,
    // type annotations) and must be importable from the public binding module.
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        for arg in &cc.args {
            if let Some(elem_type) = &arg.element_type {
                // Skip plain primitives / strings — only Named types need a Python-side import.
                // `alef.toml` describes call args in a language-agnostic way, so the
                // `element_type` value frequently uses Rust-style names (e.g.
                // `String`, `u32`). The Python binding never re-exports those —
                // they're rendered as native Python types (`str`, `int`, …) at
                // the FFI boundary — so emitting `from <pkg> import String`
                // hard-fails test collection with `ImportError`. Treat both
                // Python-style and Rust-style primitive names as primitives.
                let is_primitive = matches!(
                    elem_type.as_str(),
                    // Python-style primitives
                    "str" | "int" | "float" | "bool" | "bytes" | "list" | "dict" | "tuple" | "Any"
                    // Rust-style primitives that the binding emits as Python primitives
                    | "String" | "&str" | "char"
                    | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
                    | "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
                    | "f32" | "f64"
                );
                if !is_primitive && !import_names.contains(elem_type) {
                    import_names.push(elem_type.clone());
                }
            }
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
        let config_class = options_type.as_deref().unwrap_or_else(|| {
            panic!(
                "python e2e: handle arg present but no `options_type` configured on the call (set `[e2e.call] options_type = \"...\"` to the Python class name of the handle's config struct)"
            )
        });
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

    // Merge all top-level type names (functions, classes, enums) into import_names.
    for config_type in used_config_types {
        if !import_names.contains(config_type) {
            import_names.push(config_type.clone());
        }
    }
    for enum_type in used_enum_types {
        if !import_names.contains(enum_type) {
            import_names.push(enum_type.clone());
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
            if !import_names.contains(opts_type) {
                import_names.push(opts_type.clone());
            }
            thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
        }
    } else {
        thirdparty_from.push(format!("from {module} import {}", import_names.join(", ")));
    }

    // Also collect per-fixture options_type from per-call overrides that use from_json.
    // This handles test files where different calls use different request types.
    let mut extra_from_json_types: BTreeSet<String> = BTreeSet::new();
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
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
    use crate::e2e::escape::sanitize_filename;
    use crate::e2e::fixture::FixtureGroup;

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
        let fixtures: Vec<&crate::e2e::fixture::Fixture> = Vec::new();
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let out = render_test_file("basic", &fixtures, &e2e_config, &config, &type_defs, &enums);
        assert!(out.contains("E2e tests for category: basic"), "got: {out}");
    }
}
