//! Rust e2e test-file rendering.

use std::fmt::Write as FmtWrite;

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::helpers::{body_references_symbol, resolve_function_name_for_call, resolve_module, resolve_module_for_call};
use super::test_function::render_test_function;
use crate::e2e::codegen::rust::args::{resolve_handle_config_type, resolve_visitor_trait};

#[allow(clippy::too_many_arguments)]
pub fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    dep_name: &str,
    needs_mock_server: bool,
    // `docs_client` carries the override for the single fixture being rendered as a
    // snippet. `None` renders the executable e2e suite, whose client keeps pointing at
    // the mock server whatever the fixtures' docs metadata says. ~keep
    docs_client: Option<&crate::e2e::fixture::FixtureDocsClient>,
    snippet_expects_error: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash));
    let _ = writeln!(out, "//! E2e tests for category: {category}");
    let _ = writeln!(out);

    let module = resolve_module(e2e_config, dep_name);

    // Call-based: has mock_response OR is a plain function-call fixture (no http, no mock) with a
    // configured function name. Pure schema/spec stubs (function name empty) use the stub path.
    let file_has_call_based = fixtures.iter().any(|f| {
        if f.mock_response.is_some() {
            return true;
        }
        if f.http.is_none() && f.mock_response.is_none() {
            let call_config = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            let fn_name = resolve_function_name_for_call(call_config);
            return !fn_name.is_empty();
        }
        false
    });

    // Collect all unique (module, function) pairs needed across call-based fixtures only.
    // Resolve client_factory from the default call's rust override. When set, the generated tests
    // create a client via `module::factory(...)` and call methods on it rather than importing and
    // calling free functions. In that case we skip the function `use` imports entirely.
    let rust_call_override = e2e_config.call.overrides.get("rust");
    let client_factory = rust_call_override.and_then(|o| o.client_factory.as_deref());

    // Http fixtures and pure stub fixtures use different code paths and don't import the call function.
    if file_has_call_based && client_factory.is_none() {
        let mut imported: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
        for fixture in fixtures.iter().filter(|f| {
            if f.mock_response.is_some() {
                return true;
            }
            if f.http.is_none() && f.mock_response.is_none() {
                let call_config = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let fn_name = resolve_function_name_for_call(call_config);
                return !fn_name.is_empty();
            }
            false
        }) {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            let fn_name = resolve_function_name_for_call(call_config);
            let mod_name = resolve_module_for_call(call_config, dep_name);
            imported.insert((mod_name, fn_name));
        }
        // Emit use statements, grouping by module when possible.
        let mut by_module: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for (mod_name, fn_name) in &imported {
            by_module.entry(mod_name.clone()).or_default().push(fn_name.clone());
        }
        for (mod_name, fns) in &by_module {
            if fns.len() == 1 {
                let _ = writeln!(out, "use {mod_name}::{};", fns[0]);
            } else {
                let joined = fns.join(", ");
                let _ = writeln!(out, "use {mod_name}::{{{joined}}};");
            }
        }
    }

    // Http fixtures reference `App` and `RequestContext` via their fully-qualified
    // module path (`{dep_name}::App::new()`, `{dep_name}::RequestContext`) — no
    // top-of-file `use` import is required and emitting one trips `-D unused_imports`.

    // Render test function bodies into a side buffer so we can gate optional imports
    // on whether the body actually references each imported symbol. Emitting unused
    // imports trips `-D unused_imports` in the consumer crate.
    let mut body_buf = String::new();
    for fixture in fixtures {
        render_test_function(
            &mut body_buf,
            fixture,
            e2e_config,
            config,
            type_defs,
            enums,
            functions,
            dep_name,
            client_factory,
            docs_client,
            snippet_expects_error,
        );
        let _ = writeln!(body_buf);
    }

    // Collect all crate-level type imports (handle config types, constructor names, trait
    // imports, options_type annotations) into one BTreeSet so a symbol that appears in
    // more than one source is emitted only once, preventing E0252 "defined multiple times".
    let mut crate_imports: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Import handle constructor functions and any resolved config types they use.
    let has_handle_args = fixtures.iter().any(|fixture| {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve("rust", fixture, call_config, type_defs);
        recipe.args.iter().any(|a| a.arg_type == "handle")
    });
    if has_handle_args {
        let mut handle_config_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for fixture in fixtures {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            let recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve("rust", fixture, call_config, type_defs);
            for arg in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
                let value = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
                if value.is_null() || value.is_object() && value.as_object().is_some_and(|o| o.is_empty()) {
                    continue;
                }
                if let Some(config_type) = resolve_handle_config_type(arg, recipe.options_type, type_defs) {
                    handle_config_types.insert(config_type);
                }
            }
        }
        for config_type in handle_config_types {
            if body_references_symbol(&body_buf, &config_type) {
                crate_imports.insert(config_type);
            }
        }
        let mut constructor_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for fixture in fixtures {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            let recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve("rust", fixture, call_config, type_defs);
            for arg in recipe.args.iter().filter(|a| a.arg_type == "handle") {
                use heck::ToSnakeCase;
                constructor_names.insert(format!("create_{}", arg.name.to_snake_case()));
            }
        }
        for constructor_name in constructor_names {
            if body_references_symbol(&body_buf, &constructor_name) {
                crate_imports.insert(constructor_name);
            }
        }
    }

    // When client_factory is set, emit trait imports required to call methods on the client object.
    // Traits like LlmClient, FileClient, etc. must be in scope for method dispatch to work.
    if client_factory.is_some() && file_has_call_based {
        let trait_imports: Vec<String> = e2e_config
            .call
            .overrides
            .get("rust")
            .map(|o| o.trait_imports.clone())
            .unwrap_or_default();
        for trait_name in trait_imports {
            crate_imports.insert(trait_name);
        }
    }

    // When the rust override specifies an `options_type`,
    // type annotations are emitted on json_object bindings so that `Default::default()`
    // and `serde_json::from_value(…)` can be resolved without a trailing positional arg.
    // Import the named type so it is in scope in every test function in this file.
    // Only consider json_object args without an element_type: those with an element_type
    // are annotated as Vec<ElemType> and do not use options_type for their annotation.
    if file_has_call_based {
        for fixture in fixtures {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            let recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve("rust", fixture, call_config, type_defs);
            if recipe
                .args
                .iter()
                .any(|a| a.arg_type == "json_object" && a.element_type.is_none())
                && let Some(opts_type) = recipe.options_type
                && body_references_symbol(&body_buf, opts_type)
            {
                crate_imports.insert(opts_type.to_string());
            }
        }
    }

    // Emit all collected crate-level imports in a single sorted pass (deduped by BTreeSet).
    for symbol in &crate_imports {
        let _ = writeln!(out, "use {module}::{symbol};");
    }

    // `common::runtime()` backs every async test in this file (mock-server, HTTP, or a
    // plain fixture whose resolved call config sets `async = true`) — not only the ones
    // that also spawn a mock server — so `mod common;` is declared whenever any of those
    // are present, independent of `mod mock_server;` below. ~keep
    let file_needs_common = fixtures.iter().any(|f| {
        f.needs_mock_server()
            || e2e_config
                .resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input)
                .r#async
    });
    if file_needs_common {
        let _ = writeln!(out, "mod common;");
    }

    // Import the mock_server module when any fixture in this file uses mock_response.
    let file_needs_mock = needs_mock_server
        && fixtures
            .iter()
            .any(|f| f.mock_response.is_some() || f.needs_mock_server());
    if file_needs_mock {
        let _ = writeln!(out, "mod mock_server;");
        let _ = writeln!(out, "#[allow(unused_imports)]");
        let _ = writeln!(out, "use mock_server::{{MockRoute, MockServer}};");
    }

    // Import the visitor trait, result enum, and node context when any fixture
    // in this file declares a `visitor` block. Without these, the inline
    // `impl <visitor_trait> for _TestVisitor` block fails to resolve.
    // Visitor types live in the `visitor` sub-module of the crate, not the crate root.
    // The trait name is read from `[e2e.call.overrides.rust] visitor_trait`; omitting it
    // while a fixture declares a visitor is a configuration error.
    let file_needs_visitor = fixtures.iter().any(|f| f.visitor.is_some());
    if file_needs_visitor {
        let visitor_trait = resolve_visitor_trait(rust_call_override).unwrap_or_else(|| {
            panic!(
                "category '{}': fixture declares a visitor block but \
                 `[e2e.call.overrides.rust] visitor_trait` is not configured",
                category
            )
        });
        let _ = writeln!(
            out,
            "use {module}::visitor::{{{visitor_trait}, NodeContext, VisitResult}};"
        );
    }

    // Collect and import element types from json_object args that have an element_type specified.
    // These types are used in serde_json::from_value::<Vec<{elem}>>() for batch operations.
    // Collect from all calls used in call-based fixtures (not just the default call).
    if file_has_call_based {
        let mut element_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for fixture in fixtures.iter().filter(|f| {
            if f.mock_response.is_some() {
                return true;
            }
            if f.http.is_none() && f.mock_response.is_none() {
                let call_config = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let fn_name = resolve_function_name_for_call(call_config);
                return !fn_name.is_empty();
            }
            false
        }) {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            for arg in fixture.resolved_args(call_config) {
                if arg.arg_type == "json_object"
                    && let Some(ref elem_type) = arg.element_type
                {
                    element_types.insert(elem_type.clone());
                }
            }
        }
        for elem_type in &element_types {
            // Skip primitive / std types — they're already in scope via the Rust prelude
            // and emitting `use demo_crate::String;` (etc.) would fail with E0432.
            if matches!(
                elem_type.as_str(),
                "String"
                    | "str"
                    | "bool"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "f32"
                    | "f64"
                    | "char"
            ) {
                continue;
            }
            if body_references_symbol(&body_buf, elem_type) {
                let _ = writeln!(out, "use {module}::{elem_type};");
            }
        }
    }

    let _ = writeln!(out);
    out.push_str(&body_buf);

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
