//! Python test function body rendering (non-HTTP fixtures).

mod args;
mod error_assertions;
pub(super) mod error_types;
pub(super) mod handle_values;
pub(super) mod helper_functions;
mod result_assertions;
mod typed_values;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_python, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

use super::helpers::{self, is_skipped, resolve_client_factory, resolve_function_name_for_call};
use super::visitor_context::{distinct_context_probes, visitor_callback_probes};
use super::visitors::{
    emit_python_visitor_context_assertions, emit_python_visitor_context_probes, emit_python_visitor_method,
};
use args::build_args_and_setup;
use error_assertions::emit_error_assertion;
use result_assertions::emit_result_and_assertions;
pub(super) use typed_values::{
    KwargRenderContext, LeafSource, UsedTypeNames, render_kwarg_field_value, resolve_field_enum_type,
};

/// Read-only inputs to [`render_test_function`], bundled because every field is invariant
/// borrowed/`Copy` state describing the fixture category being rendered -- `out`, the string
/// buffer the function appends rendered lines to, stays its own `&mut` parameter alongside
/// `fixture` (the one subject each call renders), matching the split `KwargRenderContext`/
/// `ArgSink` draw in `typed_values.rs`.
#[derive(Clone, Copy)]
pub(super) struct RenderTestFunctionContext<'a> {
    pub e2e_config: &'a E2eConfig,
    pub config: &'a crate::core::config::ResolvedCrateConfig,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub functions: &'a [crate::core::ir::FunctionDef],
    pub errors: &'a [crate::core::ir::ErrorDef],
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    pub enum_fields: &'a HashMap<String, String>,
    pub handle_nested_types: &'a HashMap<String, String>,
    pub handle_dict_types: &'a HashSet<String>,
    pub force_bind_result: bool,
    pub convertible_types: &'a ahash::AHashSet<String>,
    pub crate_has_serde: bool,
    pub options_wrapped_types: &'a HashSet<String>,
}

/// Render a pytest test function for a non-HTTP fixture.
pub(super) fn render_test_function(out: &mut String, fixture: &Fixture, context: RenderTestFunctionContext<'_>) {
    let RenderTestFunctionContext {
        e2e_config,
        config,
        type_defs,
        enums,
        functions,
        errors,
        options_type,
        options_via,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        force_bind_result,
        convertible_types,
        crate_has_serde,
        options_wrapped_types,
    } = context;

    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let mut call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Fallback: if the resolved call has required args missing from input,
    // try to find a better-matching call from the named calls.
    call_config = super::super::select_best_matching_call(call_config, e2e_config, fixture);
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    // Anchor the IR-derived enum classification (`with_ir_enum_map`) at the call's declared
    // Rust return type so a leaf field name that means different things on different types
    // resolves per owner, mirroring the rust/csharp/gleam/swift/dart e2e generators. This is
    // purely additive: `is_enum` still consults `with_enum_fields` (the hand-maintained
    // `fields_enum` config) FIRST, so an explicit config entry always wins. ~keep
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        "python",
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    // Anchor the IR-derived `TypedDict`-vs-attribute-access classification the same way: the
    // pyo3 backend's own predicate (`is_dataclass_backed_config`, via `python_typeddict_fields`)
    // decides subscript vs. attribute access per owner type, so the python e2e renderer can only
    // ever agree with what `options.py` actually emits. Purely additive against a resolver built
    // before this map existed: an empty/default map leaves every path on attribute access. ~keep
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_python_typeddict_facts(FieldResolver::python_typeddict_facts(type_defs), call_root_type.clone())
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(type_defs, "python"),
        call_root_type,
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
    let field_resolver = &call_field_resolver;
    let function_name = resolve_function_name_for_call(call_config);
    let result_var = call_config.effective_result_var();

    let python_override = call_config.overrides.get("python");
    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and only fall back to the per-language override for
    // backwards compatibility with fixtures that still declare it there.
    let result_is_simple = call_config.result_is_simple || python_override.is_some_and(|o| o.result_is_simple);

    // options_type: prefer per-call override, fall back to file-level python override, then call parameter.
    let top_level_options_type = e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_type.as_deref());
    let effective_options_type = python_override
        .and_then(|o| o.options_type.as_deref())
        .or(top_level_options_type)
        .or(options_type);

    // options_via: prefer per-call override, fall back to file-level python override, then call parameter.
    let top_level_options_via = e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_via.as_deref());
    let effective_options_via = python_override
        .and_then(|o| o.options_via.as_deref())
        .or(top_level_options_via)
        .unwrap_or(options_via);
    // Only honor "from_json" when the pyo3 backend actually injects a from_json()
    // staticmethod for this type (gated on per-type has_serde AND crate-level serde
    // availability AND core→binding convertibility) AND the type's public name isn't
    // shadowed by options.py's method-less dataclass mirror — every DTO still has a plain
    // kwargs constructor, so downgrading keeps the emitted call valid. ~keep
    let effective_options_via = helpers::effective_options_via_for_type(
        effective_options_via,
        effective_options_type,
        type_defs,
        convertible_types,
        crate_has_serde,
        options_wrapped_types,
    );

    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };

    let skip_decorator = if is_skipped(fixture, "python") {
        let reason = fixture
            .skip
            .as_ref()
            .and_then(|s| s.reason.as_deref())
            .unwrap_or("skipped for python");
        let escaped = escape_python(reason);
        format!("@pytest.mark.skip(reason=\"{escaped}\")\n")
    } else {
        String::new()
    };

    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Streaming fixtures require async test functions so the async iterator
    // (ChatStreamIterator.__anext__) can be driven with `async for`.
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    // Streaming error tests: when a streaming call (declared via `streaming = true` or
    // heuristically detected by function name containing "stream") expects an error,
    // the Python binding returns the iterator synchronously; errors only surface when
    // iterating via __anext__. Make the test async and drain the iterator inside
    // `pytest.raises` so the error propagates before the `with` block exits.
    //
    // Triggers in two cases:
    // - Declared streaming call (`call_config.streaming_enabled() = true`) + error fixture.
    // - Heuristic name-based detection (function name contains "stream") for
    //   fixtures that pre-date the explicit `streaming` flag.
    let is_streaming_error_call =
        has_error_assertion && (is_streaming || function_name.to_lowercase().contains("stream"));
    let is_async = is_streaming
        || is_streaming_error_call
        || python_override.and_then(|o| o.r#async).unwrap_or(call_config.r#async);
    let async_decorator = if is_async {
        "@pytest.mark.asyncio\n".to_string()
    } else {
        String::new()
    };
    let async_kw = if is_async { "async " } else { "" };

    let arg_setup_context = args::ArgSetupContext {
        call_config,
        options_type: effective_options_type,
        options_via: effective_options_via,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        config,
        type_defs,
        enums,
    };
    let (arg_bindings, kwarg_exprs, teardown_block) = build_args_and_setup(fixture, arg_setup_context);

    // Build visitor class if present. Each callback is resolved to the bridge whose trait
    // declares it, so a crate with more than one visitor bridge probes each callback against its
    // own context type instead of one globally-picked one.
    let callback_probes = visitor_callback_probes(config, type_defs, errors, convertible_types, fixture);
    let distinct_probes = distinct_context_probes(&callback_probes);
    let probe_context = !distinct_probes.is_empty();
    let mut visitor_class = String::new();
    if fixture.visitor.is_some() {
        let _ = writeln!(visitor_class, "    class _TestVisitor:");
        emit_python_visitor_context_probes(&mut visitor_class, &distinct_probes);
        for (method_name, action, probe) in &callback_probes {
            emit_python_visitor_method(
                &mut visitor_class,
                method_name,
                action,
                probe.as_ref().map(|probe| probe.probe_method.as_str()),
            );
        }
    }

    // Build arg bindings string
    let mut arg_bindings_str = arg_bindings.iter().map(|b| format!("{b}\n")).collect::<String>();
    if fixture.visitor.is_some() {
        arg_bindings_str.push_str("    _visitor = _TestVisitor()\n");
    }

    let call_args_str = {
        let mut exprs = kwarg_exprs.clone();
        if fixture.visitor.is_some() {
            exprs.push("visitor=_visitor".to_string());
        }
        exprs.join(", ")
    };
    // For streaming fixtures, chat_stream() is synchronous (block_on) and returns
    // the iterator directly — do NOT await it even though the test function is async
    // (the async is needed to drive `async for chunk in result`).
    let await_prefix = if is_async && !is_streaming { "await " } else { "" };

    // Client factory: when configured, create a client and dispatch as a method.
    // Fixtures with mock_response point the client at the mock server via base_url so
    // the fixture response is served via prefix routing.
    // Fixtures without mock_response (real-API smoke tests) use no base_url override.
    let client_factory = resolve_client_factory(e2e_config);
    let mut client_setup = String::new();
    let call_expr = if let Some(ref factory) = client_factory {
        let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
        let api_key_opt = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
        if let Some(api_key_var) = api_key_opt.filter(|_| has_mock) {
            let fixture_id = &fixture.id;
            let mock_base_url_expr = if fixture.has_host_root_route() {
                format!(
                    "os.environ.get(\"MOCK_SERVER_{}\") or os.environ[\"MOCK_SERVER_URL\"] + \"/fixtures/{fixture_id}\"",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("os.environ[\"MOCK_SERVER_URL\"] + \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(client_setup, "    api_key = os.environ.get(\"{api_key_var}\")");
            let _ = writeln!(client_setup, "    if api_key:");
            let _ = writeln!(
                client_setup,
                "        sys.stdout.write(\"{fixture_id}: using real API ({api_key_var} is set)\\n\")"
            );
            let _ = writeln!(client_setup, "        sys.stdout.flush()");
            let _ = writeln!(client_setup, "        client = {factory}(api_key=api_key)");
            let _ = writeln!(client_setup, "    else:");
            let _ = writeln!(
                client_setup,
                "        sys.stdout.write(\"{fixture_id}: using mock server ({api_key_var} not set)\\n\")"
            );
            let _ = writeln!(client_setup, "        sys.stdout.flush()");
            let _ = writeln!(
                client_setup,
                "        client = {factory}(api_key=\"test-key\", base_url={mock_base_url_expr})"
            );
        } else if has_mock {
            let fixture_id = &fixture.id;
            let base_url_expr = if fixture.has_host_root_route() {
                format!(
                    "os.environ.get(\"MOCK_SERVER_{}\") or os.environ[\"MOCK_SERVER_URL\"] + \"/fixtures/{fixture_id}\"",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("os.environ[\"MOCK_SERVER_URL\"] + \"/fixtures/{fixture_id}\"")
            };
            let _ = writeln!(
                client_setup,
                "    client = {factory}(api_key=\"test-key\", base_url={base_url_expr})"
            );
        } else if let Some(api_key_var) = api_key_opt {
            let _ = writeln!(client_setup, "    api_key = os.environ.get(\"{api_key_var}\")");
            let _ = writeln!(client_setup, "    if not api_key:");
            let _ = writeln!(client_setup, "        pytest.skip(\"{api_key_var} not set\")");
            let _ = writeln!(client_setup, "    client = {factory}(api_key=api_key)");
        } else {
            let _ = writeln!(client_setup, "    client = {factory}(api_key=\"test-key\")");
        }
        format!("{await_prefix}client.{function_name}({call_args_str})")
    } else {
        format!("{await_prefix}{function_name}({call_args_str})")
    };
    // Prepend client setup to arg bindings so it lands inside the test function body.
    let arg_bindings_str = format!("{client_setup}{arg_bindings_str}");

    if has_error_assertion {
        // For error-assertion fixtures, the engine creation and other arg bindings
        // must happen INSIDE the `pytest.raises` block — otherwise validation
        // errors raised at engine-creation time fly past the assertion uncaught
        // and crash the test (e.g. `validation_max_depth_too_high` raises in
        // `create_engine(CrawlConfig(max_depth=200))` before the `await scrape(...)`
        // call ever runs). Pass arg_bindings_str to emit_error_assertion so it
        // can emit them indented one level deeper, inside the with block.
        // The module a resolvable `error.<field>` assertion imports its `{Error}Info` companion
        // from. `from_json_module` already exists precisely to name the native extension module
        // (as opposed to the public package) from generated Python test code -- see
        // `test_file.rs`'s `from_json_module` handling for the sibling use of the same override
        // -- so this reuses it rather than adding a second, parallel config knob for the same
        // fact. ~keep
        let native_module = python_override
            .and_then(|o| o.from_json_module.clone())
            .unwrap_or_else(|| helpers::resolve_module(e2e_config));
        let mut error_assertion_block = String::new();
        emit_error_assertion(
            &mut error_assertion_block,
            fixture,
            &arg_bindings_str,
            &call_expr,
            is_streaming_error_call,
            errors,
            &native_module,
        );
        // ~keep The ledger recording now lives inside `error_path_assertions::render`, which every
        // backend's error block shares. Gating here as well would double-count every python marker.

        let ctx = minijinja::context! {
            skip_decorator => skip_decorator,
            async_decorator => async_decorator,
            async_kw => async_kw,
            fn_name => fn_name,
            docstring => desc_with_period,
            visitor_class => visitor_class,
            arg_bindings => String::new(),
            call_expr => call_expr,
            is_error_assertion => true,
            error_assertion_block => error_assertion_block,
            result_assertions => String::new(),
        };
        let rendered = crate::e2e::template_env::render("python/test_function.jinja", ctx);
        out.push_str(&rendered);
        return;
    }

    // Build result and assertions
    //
    // The stream chunk item type is only needed for streaming fixtures, and only to classify
    // that type (and the ones it transitively owns) against `TypedDict` — see
    // `emit_streaming_virtual_assertion`. Resolved the same way rust/go/ruby resolve it for
    // their own streaming accessors (`recipe::streaming_item_type`): an explicit
    // `[crates.e2e.call.streaming] item_type` wins, else the matching `[[crates.adapters]]
    // pattern = "streaming"` entry's `item_type`.
    let streaming_item_type = is_streaming
        .then(|| {
            crate::e2e::codegen::recipe::streaming_item_type(call_config, &config.adapters, &[function_name.as_str()])
        })
        .flatten();
    let mut result_assertions = String::new();
    emit_result_and_assertions(
        &mut result_assertions,
        fixture,
        e2e_config,
        call_config,
        &call_expr,
        result_var,
        field_resolver,
        result_is_simple,
        is_streaming,
        force_bind_result,
        streaming_item_type,
    );

    if fixture.visitor.is_some() && probe_context {
        if !result_assertions.ends_with('\n') {
            result_assertions.push('\n');
        }
        emit_python_visitor_context_assertions(&mut result_assertions);
    }

    // Append trait-bridge teardown after assertions. This restores shared
    // global state (e.g. plugin registries) between pytest
    // tests in the same process. See `emit_test_backend` for the rationale.
    if !teardown_block.is_empty() {
        if !result_assertions.ends_with('\n') {
            result_assertions.push('\n');
        }
        result_assertions.push_str(&teardown_block);
    }

    let ctx = minijinja::context! {
        skip_decorator => skip_decorator,
        async_decorator => async_decorator,
        async_kw => async_kw,
        fn_name => fn_name,
        docstring => desc_with_period,
        visitor_class => visitor_class,
        arg_bindings => arg_bindings_str,
        call_expr => call_expr,
        is_error_assertion => false,
        error_assertion_block => String::new(),
        result_assertions => result_assertions,
    };
    let rendered = crate::e2e::template_env::render("python/test_function.jinja", ctx);
    out.push_str(&rendered);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_test_function_skipped_fixture_emits_skip_decorator() {
        use crate::e2e::fixture::{Fixture, SkipDirective};
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "skipped_test".to_string(),
            description: "A skipped test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: Vec::new(),
            call: None,
            skip: Some(SkipDirective {
                languages: vec!["python".to_string()],
                reason: Some("not supported".to_string()),
            }),
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
        let e2e_config = crate::e2e::config::E2eConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let mut out = String::new();
        let context = RenderTestFunctionContext {
            e2e_config: &e2e_config,
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            functions: &[],
            errors: &[],
            options_type: None,
            options_via: "kwargs",
            enum_fields: &HashMap::new(),
            handle_nested_types: &HashMap::new(),
            handle_dict_types: &HashSet::new(),
            force_bind_result: false,
            convertible_types: &ahash::AHashSet::new(),
            crate_has_serde: false,
            options_wrapped_types: &HashSet::new(),
        };
        render_test_function(&mut out, &fixture, context);
        assert!(out.contains("pytest.mark.skip"), "got: {out}");
        assert!(out.contains("not supported"), "got: {out}");
    }
}
