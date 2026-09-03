//! Python test file generation — import resolution and orchestration.

// `python/mod.rs` is over the file-size cap and in the baseline, so it may not grow --
// declared here via `#[path]` at the sibling files that exist only to support this one. ~keep
#[path = "import_lines.rs"]
mod import_lines;
#[cfg(test)]
#[path = "lint_clean_python_tests.rs"]
mod lint_clean_python_tests;
// Split out to claw back headroom under the file-size ratchet's baselined ceiling for this
// file (see `tests/file_size_baseline.txt`) -- these tests have no dependency on anything else
// in `mod tests` below, so they move cleanly. ~keep
#[cfg(test)]
#[path = "test_file_misc_tests.rs"]
mod test_file_misc_tests;
// Same reason as `test_file_misc_tests` above: the dead-helper-emission regression group (a
// shared fixture builder plus its tests) has no dependency on anything else in `mod tests`. ~keep
#[cfg(test)]
#[path = "dead_helper_tests.rs"]
mod dead_helper_tests;
// Declared here for the same `python/mod.rs` size reason as the modules above. It needs
// `render_test_file` (to prove the class body it executes is the one that ships) and reaches the
// visitor generators through `super::super`. ~keep
#[cfg(test)]
#[path = "visitor_context_runtime_tests.rs"]
mod visitor_context_runtime_tests;

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;

use heck::ToSnakeCase;

use crate::core::hash::{self, CommentStyle};
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use self::import_lines::{ImportNeeds, compute_pytest_and_sys_import_needs, finalize_stdlib_and_bare_imports};
use super::helpers::{
    self, BytesKind, classify_bytes_value, python_method_helper_import, resolve_client_factory, resolve_enum_fields,
    resolve_function_name, resolve_function_name_for_call, resolve_handle_dict_types, resolve_handle_nested_types,
    resolve_module, resolve_options_type, resolve_options_via,
};
use super::http::render_http_test_function;
use super::test_function::handle_values::collect_used_nested_types;
use super::test_function::helper_functions::{render_item_texts_helper, render_text_helper};
use super::test_function::{
    KwargRenderContext, LeafSource, RenderTestFunctionContext, render_test_function, resolve_field_enum_type,
};

/// Render a complete Python test file for a single fixture category.
///
/// `force_bind_result` overrides the usual assertion-driven heuristic for
/// binding the call's result to `result_var` — the docs-snippet caller sets
/// this when it will print the result unconditionally (fixture assertions are
/// stripped for snippets), so the emitted call must still assign it. ~keep
#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
    force_bind_result: bool,
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
    // Only honor "from_json" when the pyo3 backend actually injects a from_json() staticmethod
    // for this type (gated on per-type has_serde AND crate-level serde availability AND
    // core→binding convertibility) — every DTO still has a plain kwargs constructor, so
    // downgrading keeps the emitted call and its import valid. Computed once per file/snippet
    // render; `type_defs`/`enums`/`config` don't change across fixtures in the same category. ~keep
    let convertible_types = helpers::core_to_binding_convertible_types(type_defs, enums);
    let crate_has_serde = crate::backends::pyo3::gen_bindings::crate_has_serde(config);
    let effective_options_via = helpers::effective_options_via_for_type(
        effective_options_via,
        effective_options_type.as_deref(),
        type_defs,
        &convertible_types,
        crate_has_serde,
    );

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
    let client_factory = resolve_client_factory(e2e_config);
    let (needs_pytest, needs_sys_import) =
        compute_pytest_and_sys_import_needs(fixtures, client_factory.as_deref(), has_error_test, is_async);

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

    let needs_os_import = client_factory.is_some()
        || has_http_tests
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
        if f.docs
            .as_ref()
            .and_then(|docs| docs.presentation.as_ref())
            .is_some_and(|presentation| !presentation.files.is_empty())
        {
            return true;
        }
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
                    collect_json_object_enum_types(
                        obj,
                        constructor_type,
                        enum_fields,
                        type_defs,
                        enums,
                        &mut used_enum_types,
                    );
                }
                // Collect the config type itself (e.g., ExtractionConfig, EmbeddingConfig)
                if let Some(opts_type) = constructor_type
                    && !value.is_null()
                    && value.is_object()
                {
                    // This is a constructor call like ExtractionConfig(...), so import the type
                    used_config_types.insert(opts_type.to_string());
                }
                // Nested config/struct fields also need imports; split out to keep this
                // already-over-cap file's growth minimal. ~keep
                let context = KwargRenderContext {
                    type_defs,
                    enums,
                    enum_fields,
                    docs_files: &[],
                    leaf_source: LeafSource::Literal,
                };
                import_lines::collect_nested_config_types(
                    arg,
                    value,
                    constructor_type,
                    context,
                    &mut used_config_types,
                    &mut used_enum_types,
                );
            }

            // For handle args, collect constructor types referenced by element_type
            if arg.arg_type == "handle"
                && let Some(elem_type) = &arg.element_type
            {
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

    used_config_types.extend(super::test_function::error_types::collect_used_error_types(
        fixtures, errors,
    ));

    let mut stdlib_imports: Vec<String> = Vec::new();
    let mut thirdparty_bare: Vec<String> = Vec::new();
    let mut thirdparty_from: Vec<String> = Vec::new();

    // Import candidates are derived for every non-HTTP fixture, skipped or not. A skipped fixture
    // still has its full call body emitted (only a `@pytest.mark.skip` decorator is added), and the
    // docs-snippet emitter lifts this same import block out of the rendered file for a snippet that
    // is published regardless of skip status — so suppressing imports here emits bodies whose
    // symbols do not resolve. `prune_unreferenced_from_imports` below removes anything the emitted
    // unit does not actually reference, so widening the candidate set cannot add a dead import. ~keep
    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test());
    if has_non_http_fixtures {
        let thirdparty_import_context = ThirdpartyImportContext {
            fixtures,
            e2e_config,
            config,
            module: &module,
            function_name: &function_name,
            client_factory: client_factory.as_deref(),
            options_type: &effective_options_type,
            options_via: effective_options_via,
            from_json_module: from_json_module.as_deref(),
            needs_options_type,
            enum_fields,
            handle_nested_types,
            handle_dict_types,
            used_enum_types: &used_enum_types,
            used_config_types: &used_config_types,
            type_defs,
            convertible_types: &convertible_types,
            crate_has_serde,
        };
        build_thirdparty_imports(thirdparty_import_context, &mut thirdparty_from);
    }

    thirdparty_from.sort();

    // Render all fixtures
    let mut fixtures_body = String::new();
    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_function(&mut fixtures_body, fixture);
        } else {
            let render_test_function_context = RenderTestFunctionContext {
                e2e_config,
                config,
                type_defs,
                enums,
                functions,
                errors,
                options_type: effective_options_type.as_deref(),
                options_via: effective_options_via,
                enum_fields,
                handle_nested_types,
                handle_dict_types,
                force_bind_result,
                convertible_types: &convertible_types,
                crate_has_serde,
            };
            render_test_function(&mut fixtures_body, fixture, render_test_function_context);
        }
        let _ = writeln!(fixtures_body);
    }

    let import_needs = ImportNeeds {
        has_http_tests,
        needs_base64_import,
        needs_json_import,
        needs_os_import,
        needs_path_import,
        needs_sys_import,
        needs_pytest,
    };
    finalize_stdlib_and_bare_imports(&fixtures_body, import_needs, &mut stdlib_imports, &mut thirdparty_bare);

    // Each helper is emitted iff the unit that ships in this file references it. The two
    // are gated separately because they have independent callers: the array
    // `contains`/`contains_any` paths call `_alef_e2e_item_texts`, while the enum `equals`
    // path (`python/assertion.jinja`) calls `_alef_e2e_text` directly. Gating the pair on
    // `_alef_e2e_item_texts` alone emitted `_alef_e2e_text`'s definition into only the one
    // file that happened to have an array assertion, leaving 22 F821 undefined-name errors
    // across the four files that call it from an enum assertion. There is no shared python
    // module to import from — `conftest.py` carries pytest fixtures, not importable helpers —
    // so every file that calls a helper must also define it. ~keep
    let mut item_texts_helper = String::new();
    if references_identifier(&fixtures_body, "_alef_e2e_item_texts") {
        render_item_texts_helper(&mut item_texts_helper);
    }
    let mut helper_functions = String::new();
    if references_identifier(&fixtures_body, "_alef_e2e_text")
        || references_identifier(&item_texts_helper, "_alef_e2e_text")
    {
        render_text_helper(&mut helper_functions);
    }
    helper_functions.push_str(&item_texts_helper);

    import_lines::prune_unreferenced_from_imports(
        &mut thirdparty_from,
        &[helper_functions.as_str(), fixtures_body.as_str()],
    );

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

/// Collect explicitly configured enum fields for one `json_object` arg's object value.
/// Auto-detected enums (`resolve_field_enum_type`) must mirror the render path in
/// `test_function.rs` -- otherwise the kwarg builder emits `OutputFormat.MARKDOWN` while this
/// import collector never adds `OutputFormat`, producing a `NameError` at test runtime. Split
/// out of `render_test_file`'s import-collection loop to keep that loop's nesting depth under
/// the file's cap.
fn collect_json_object_enum_types(
    obj: &serde_json::Map<String, serde_json::Value>,
    constructor_type: Option<&str>,
    enum_fields: &std::collections::HashMap<String, String>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    used_enum_types: &mut BTreeSet<String>,
) {
    for key in obj.keys() {
        if let Some(enum_type) = enum_fields.get(key) {
            used_enum_types.insert(enum_type.clone());
        } else if let Some(auto_enum_type) = resolve_field_enum_type(key, constructor_type, type_defs, enums) {
            used_enum_types.insert(auto_enum_type);
        }
    }
}

/// True when `name` occurs in `source` as a whole Python identifier rather than as a
/// substring of a longer one (`Widget` must not match inside `WidgetRequest`).
pub(super) fn references_identifier(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
    let mut offset = 0;
    while let Some(found) = source[offset..].find(name) {
        let start = offset + found;
        let end = start + name.len();
        let before_ok = source[..start].chars().next_back().is_none_or(|c| !is_ident_char(c));
        let after_ok = source[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            return true;
        }
        offset = start + name.len().max(1);
    }
    false
}

/// Read-only inputs to [`build_thirdparty_imports`], bundled because every field is invariant
/// borrowed/`Copy` state that single call collects import candidates from -- only
/// `thirdparty_from` (the output accumulator that function mutates) stays its own `&mut`
/// parameter, matching the split `KwargRenderContext`/`ArgSink` draw in `typed_values.rs`.
#[derive(Clone, Copy)]
struct ThirdpartyImportContext<'a> {
    fixtures: &'a [&'a Fixture],
    e2e_config: &'a E2eConfig,
    config: &'a crate::core::config::ResolvedCrateConfig,
    module: &'a str,
    function_name: &'a str,
    client_factory: Option<&'a str>,
    options_type: &'a Option<String>,
    options_via: &'a str,
    from_json_module: Option<&'a str>,
    needs_options_type: bool,
    enum_fields: &'a std::collections::HashMap<String, String>,
    handle_nested_types: &'a std::collections::HashMap<String, String>,
    handle_dict_types: &'a std::collections::HashSet<String>,
    used_enum_types: &'a BTreeSet<String>,
    used_config_types: &'a BTreeSet<String>,
    type_defs: &'a [crate::core::ir::TypeDef],
    convertible_types: &'a ahash::AHashSet<String>,
    crate_has_serde: bool,
}

fn build_thirdparty_imports(context: ThirdpartyImportContext<'_>, thirdparty_from: &mut Vec<String>) {
    let ThirdpartyImportContext {
        fixtures,
        e2e_config,
        config,
        module,
        function_name,
        client_factory,
        options_type,
        options_via,
        from_json_module,
        needs_options_type,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        used_enum_types,
        used_config_types,
        type_defs,
        convertible_types,
        crate_has_serde,
    } = context;

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
            if let Some(bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == trait_name)
                && let Some(unregister_fn) = bridge.unregister_fn.as_deref()
            {
                let unregister_str = unregister_fn.to_string();
                if !import_names.contains(&unregister_str) {
                    import_names.push(unregister_str);
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
        let mut used_types: BTreeSet<String> = BTreeSet::new();
        for fixture in fixtures.iter() {
            for arg in e2e_config.call.args.iter().filter(|arg| arg.arg_type == "handle") {
                let config_value = resolve_field(&fixture.input, &arg.field);
                for (key, value) in config_value.as_object().into_iter().flatten() {
                    collect_used_nested_types(key, value, handle_nested_types, handle_dict_types, &mut used_types);
                }
            }
        }
        for type_name in used_types {
            if !import_names.contains(&type_name) {
                import_names.push(type_name);
            }
        }
    }

    for fixture in fixtures.iter() {
        for assertion in &fixture.assertions {
            if assertion.assertion_type == "method_result"
                && let Some(method_name) = &assertion.method
                && let Some(name) = python_method_helper_import(method_name)
                && !import_names.contains(&name)
            {
                import_names.push(name);
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

    let mut extra_from_json_imports: BTreeSet<(String, String)> = BTreeSet::new();
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        if let Some(python_override) = cc.overrides.get("python")
            && python_override.options_via.as_deref() == Some("from_json")
            && let Some(options_type) = &python_override.options_type
            && helpers::effective_options_via_for_type(
                "from_json",
                Some(options_type.as_str()),
                type_defs,
                convertible_types,
                crate_has_serde,
            ) == "from_json"
        {
            let native_module = python_override.from_json_module.as_deref().unwrap_or(module);
            extra_from_json_imports.insert((native_module.to_string(), options_type.clone()));
        }
    }

    let filtered_public_imports = public_import_names(&import_names, &extra_from_json_imports);

    if let (true, Some(opts_type)) = (
        needs_options_type && (options_via == "kwargs" || options_via == "from_json"),
        options_type,
    ) {
        if options_via == "from_json" {
            // Import opts_type from the native bindings module (e.g., PyO3 _internal_bindings),
            // not the public module — it needs the native from_json() staticmethod. Exclude it
            // from the public import line so the class isn't imported from both modules (the
            // second import silently shadows the first). ~keep
            let public_names: Vec<&str> = filtered_public_imports
                .iter()
                .copied()
                .filter(|name| *name != opts_type)
                .collect();
            if !public_names.is_empty() {
                thirdparty_from.push(format!("from {module} import {}", public_names.join(", ")));
            }
            let native_mod = from_json_module.unwrap_or(module);
            thirdparty_from.push(format!("from {native_mod} import {opts_type}"));
        } else {
            if !import_names.contains(opts_type) {
                import_names.push(opts_type.clone());
            }
            let public_names = public_import_names(&import_names, &extra_from_json_imports);
            if !public_names.is_empty() {
                thirdparty_from.push(format!("from {module} import {}", public_names.join(", ")));
            }
        }
    } else if !filtered_public_imports.is_empty() {
        thirdparty_from.push(format!("from {module} import {}", filtered_public_imports.join(", ")));
    }

    for (native_module, options_type) in extra_from_json_imports {
        let imp = format!("from {native_module} import {options_type}");
        if !thirdparty_from.contains(&imp) {
            thirdparty_from.push(imp);
        }
    }

    let _ = enum_fields;
}

fn public_import_names<'a>(import_names: &'a [String], native_imports: &BTreeSet<(String, String)>) -> Vec<&'a str> {
    import_names
        .iter()
        .filter(|name| !native_imports.iter().any(|(_, native_type)| native_type == *name))
        .map(String::as_str)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_test_file_no_fixtures_produces_header_only() {
        let fixtures: Vec<&crate::e2e::fixture::Fixture> = Vec::new();
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let out = render_test_file(
            "basic",
            &fixtures,
            &e2e_config,
            &config,
            &type_defs,
            &enums,
            &[],
            &[],
            false,
        );
        assert!(out.contains("E2e tests for category: basic"), "got: {out}");
    }

    /// Direct coverage of the import-deduplication fix on `build_thirdparty_imports`'s
    /// `options_via == "from_json"` branch — reachable through `render_test_file`/
    /// `render_snippet_body` for any type that passes pyo3's Rust-codegen gate (see
    /// `helpers::pyo3_would_inject_from_json`).
    #[test]
    fn build_thirdparty_imports_does_not_duplicate_the_from_json_type_across_modules() {
        let fixtures: Vec<&crate::e2e::fixture::Fixture> = Vec::new();
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let options_type = Some("WidgetRequest".to_string());
        // Mirrors what `render_test_file`'s scan populates for a json_object arg constructed
        // via `WidgetRequest(...)` — the type name lands in `used_config_types` regardless of
        // `options_via`.
        let used_config_types: BTreeSet<String> = ["WidgetRequest".to_string()].into_iter().collect();
        let mut thirdparty_from: Vec<String> = Vec::new();

        let thirdparty_import_context = ThirdpartyImportContext {
            fixtures: &fixtures,
            e2e_config: &e2e_config,
            config: &config,
            module: "my_lib",
            function_name: "create_widget",
            client_factory: Some("create_client"),
            options_type: &options_type,
            options_via: "from_json",
            from_json_module: Some("my_lib._internal_bindings"),
            needs_options_type: true,
            enum_fields: &std::collections::HashMap::new(),
            handle_nested_types: &std::collections::HashMap::new(),
            handle_dict_types: &std::collections::HashSet::new(),
            used_enum_types: &BTreeSet::new(),
            used_config_types: &used_config_types,
            type_defs: &[],
            convertible_types: &ahash::AHashSet::new(),
            crate_has_serde: false,
        };
        build_thirdparty_imports(thirdparty_import_context, &mut thirdparty_from);

        let import_lines_with_type: Vec<&String> = thirdparty_from
            .iter()
            .filter(|line| line.starts_with("from ") && line.contains("WidgetRequest"))
            .collect();
        assert_eq!(
            import_lines_with_type,
            vec!["from my_lib._internal_bindings import WidgetRequest"],
            "WidgetRequest must be imported from exactly one module, got: {thirdparty_from:?}"
        );
        assert!(
            thirdparty_from.contains(&"from my_lib import create_client".to_string()),
            "the client factory must still be imported from the public module, got: {thirdparty_from:?}"
        );
    }
}
