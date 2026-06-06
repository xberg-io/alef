//! Python test function body rendering (non-HTTP fixtures).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use heck::{ToShoutySnakeCase, ToSnakeCase};

use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_python, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

use super::assertions::render_assertion;
use super::helpers::{
    BytesKind, classify_bytes_value, is_skipped, resolve_assert_enum_fields, resolve_client_factory,
    resolve_function_name_for_call,
};
use super::json::{json_to_python_literal, value_to_python_string};
use super::visitors::emit_python_visitor_method;

/// Render a pytest test function for a non-HTTP fixture.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) {
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
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    );
    let field_resolver = &call_field_resolver;
    let function_name = resolve_function_name_for_call(call_config);
    let result_var = &call_config.result_var;

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

    let (arg_bindings, kwarg_exprs, teardown_block) = build_args_and_setup(
        fixture,
        call_config,
        effective_options_type,
        effective_options_via,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        config,
        type_defs,
        enums,
    );

    // Build visitor class if present
    let mut visitor_class = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        let _ = writeln!(visitor_class, "    class _TestVisitor:");
        for (method_name, action) in &visitor_spec.callbacks {
            emit_python_visitor_method(&mut visitor_class, method_name, action);
        }
    }

    // Build arg bindings string
    let arg_bindings_str = arg_bindings.iter().map(|b| format!("{b}\n")).collect::<String>();

    let call_args_str = {
        let mut exprs = kwarg_exprs.clone();
        if fixture.visitor.is_some() {
            exprs.push("visitor=_TestVisitor()".to_string());
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
                "        print(\"{fixture_id}: using real API ({api_key_var} is set)\", flush=True)  # noqa: T201"
            );
            let _ = writeln!(client_setup, "        client = {factory}(api_key=api_key)");
            let _ = writeln!(client_setup, "    else:");
            let _ = writeln!(
                client_setup,
                "        print(\"{fixture_id}: using mock server ({api_key_var} not set)\", flush=True)  # noqa: T201"
            );
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
            let _ = writeln!(client_setup, "    if not api_key:  # noqa: SIM102");
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
        let mut error_assertion_block = String::new();
        emit_error_assertion(
            &mut error_assertion_block,
            fixture,
            &arg_bindings_str,
            &call_expr,
            is_streaming_error_call,
        );

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
    );

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

fn emit_error_assertion(
    out: &mut String,
    fixture: &Fixture,
    arg_bindings_str: &str,
    call_expr: &str,
    is_streaming_error_call: bool,
) {
    let error_assertion = fixture.assertions.iter().find(|a| a.assertion_type == "error");
    let has_message = error_assertion
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.as_str())
        .is_some();

    // Re-indent arg_bindings by an extra 4 spaces so they land inside the `with`
    // block. arg_bindings already begin with 4 spaces (function-body level);
    // prepending 4 more puts them at the with-body level (8 spaces).
    let indented_bindings: String = arg_bindings_str
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("    {l}\n"))
        .collect();

    if has_message {
        let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:  # noqa: B017");
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            // The streaming iterator returns synchronously (chat_stream returns the
            // iterator without await); errors only appear when iterating via
            // __anext__. Strip the `await ` prefix the async-call codegen would
            // attach, then drain the iterator inside the raises block so the
            // exception propagates before the with-block exits.
            let sync_call_expr = call_expr.strip_prefix("await ").unwrap_or(call_expr);
            let _ = writeln!(out, "        _iterator = {sync_call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
        if let Some(msg) = error_assertion.and_then(|a| a.value.as_ref()).and_then(|v| v.as_str()) {
            let escaped = escape_python(msg);
            // Match against EITHER the rendered exception message OR the
            // exception class name. Different crates use different
            // fixture-shape conventions:
            //   * config-validation fixtures may use field names that are substrings
            //     of the user-facing error message, never of a class name.
            //   * API-error fixtures may use class-name prefixes such as
            //     `Authentication`, `BadRequest`, or `ContentPolicy`.
            //     `BadRequestError`, `ContentPolicyError`), not message text.
            // The disjunction lets a single codegen path satisfy both.
            let _ = writeln!(
                out,
                "    assert \"{escaped}\" in str(exc_info.value) or \"{escaped}\" in type(exc_info.value).__name__  # noqa: S101"
            );
        }
    } else {
        let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            let _ = writeln!(out, "        _iterator = {call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_result_and_assertions(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &crate::e2e::config::CallConfig,
    call_expr: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    is_streaming: bool,
) {
    // For streaming fixtures, streaming virtual fields are always usable
    // (they resolve against the collected `chunks` list, not the result type).
    let chunks_var = "chunks";
    let _ = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        if is_streaming {
            if let Some(f) = &a.field {
                if crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                    return true;
                }
            }
        }
        if result_is_simple {
            if let Some(f) = &a.field {
                let f_lower = f.to_lowercase();
                if !f.is_empty()
                    && f_lower != "content"
                    && f_lower != "result"
                    && (f_lower.starts_with("metadata")
                        || f_lower.starts_with("document")
                        || f_lower.starts_with("structure")
                        || f_lower.starts_with("pages")
                        || f_lower.starts_with("chunks")
                        || f_lower.starts_with("tables")
                        || f_lower.starts_with("images")
                        || f_lower.starts_with("mime_type")
                        || f_lower.starts_with("is_")
                        || f_lower == "byte_length"
                        || f_lower == "page_count"
                        || f_lower == "output_format"
                        || f_lower == "extraction_method")
                {
                    return false;
                }
            }
            return true;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    let fields_enum = e2e_config.effective_fields_enum(call_config);
    let assert_enum_fields = resolve_assert_enum_fields(call_config);

    // For streaming fixtures: bind the raw iterator, then drain it into a list.
    // The Python ChatStreamIterator exposes __aiter__/__anext__ (async iterator),
    // so the test function must be `async def` and we use `async for` to drain.
    // Note: chat_stream() itself is NOT a coroutine in Python — it returns the
    // iterator synchronously (blocking on stream acquisition via block_on), so
    // no `await` prefix is used on the call expression.
    if is_streaming {
        let _ = writeln!(out, "    {result_var} = {call_expr}");
        if let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "python", result_var, chunks_var,
        ) {
            let _ = writeln!(out, "    {collect}");
        }
        // Render streaming assertions using the collected chunks
        for assertion in &fixture.assertions {
            if assertion.assertion_type == "not_error" || assertion.assertion_type == "error" {
                continue;
            }
            if let Some(f) = &assertion.field {
                if crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                    emit_streaming_virtual_assertion(out, assertion, f, chunks_var);
                    continue;
                }
            }
            // Non-streaming-virtual assertions on streaming fixtures are skipped
            // (the result type doesn't have these fields during iteration).
        }
    } else {
        // For non-streaming: render assertions to a temporary buffer first,
        // then check if result_var is referenced. Only emit the assignment if it is.
        let mut temp_assertions = String::new();

        for assertion in &fixture.assertions {
            if assertion.assertion_type == "not_error" {
                if !call_config.returns_result {
                    continue;
                }
                continue;
            }
            render_assertion(
                &mut temp_assertions,
                assertion,
                result_var,
                field_resolver,
                fields_enum,
                assert_enum_fields,
                result_is_simple,
            );
        }

        // Check if result_var appears in actual code (not in comments).
        // Only count lines that start with "assert" or contain actual code tokens.
        // Comments (lines starting with #) are skipped to avoid false positives
        // from strings like "field 'result' not available" in comment text.
        let result_var_used = temp_assertions.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with('#') && trimmed.contains(result_var)
        });

        let py_result_var = if result_var_used {
            result_var.to_string()
        } else {
            "_".to_string()
        };
        let _ = writeln!(out, "    {py_result_var} = {call_expr}");
        out.push_str(&temp_assertions);
    }
}

/// Emit a Python assertion for a streaming virtual field using the collected
/// `chunks` list.  Mirrors the pattern in rust/assertions.rs.
fn emit_streaming_virtual_assertion(
    out: &mut String,
    assertion: &crate::e2e::fixture::Assertion,
    field: &str,
    chunks_var: &str,
) {
    use crate::e2e::codegen::streaming_assertions::StreamingFieldResolver;

    let Some(expr) = StreamingFieldResolver::accessor(field, "python", chunks_var) else {
        let _ = writeln!(out, "    # skipped: streaming field '{field}': no python accessor");
        return;
    };

    match assertion.assertion_type.as_str() {
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({expr}) >= {n}  # noqa: S101");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    assert len({expr}) == {n}  # noqa: S101");
                }
            }
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                if val.is_string() {
                    let _ = writeln!(out, "    assert {expr}.strip() {op} {expected}.strip()  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {expr} {op} {expected}  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert {expr}  # noqa: S101");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert not {expr}  # noqa: S101");
        }
        "is_true" => {
            // Normalize "true"/"false" literals to Python's True/False.
            let py_expr = if expr == "true" {
                "True".to_string()
            } else if expr == "false" {
                "False".to_string()
            } else {
                expr.clone()
            };
            let _ = writeln!(out, "    assert {py_expr}  # noqa: S101");
        }
        "is_false" => {
            let py_expr = if expr == "true" {
                "True".to_string()
            } else if expr == "false" {
                "False".to_string()
            } else {
                expr.clone()
            };
            let _ = writeln!(out, "    assert not {py_expr}  # noqa: S101");
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {expected}  # noqa: S101");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} >= {expected}  # noqa: S101");
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expected} in {expr}  # noqa: S101");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    # skipped: streaming field '{field}': assertion type '{}' not rendered",
                assertion.assertion_type
            );
        }
    }
}

/// Build arg binding lines and kwarg expressions for a fixture call.
///
/// Returns `(arg_bindings, kwarg_exprs, teardown_block)`. The teardown block
/// contains statements emitted after the fixture call and its assertions —
/// trait-bridge fixtures populate it with `unregister_<trait>("<name>")` so
/// pytest's shared-process registry state is restored between tests.
#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
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
                    let emission = super::emit_test_backend(trait_bridge, &methods, fixture);
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
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_python(s))))
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
                options_type,
                options_via,
                enum_fields,
                &arg.element_type,
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

/// Resolve the enum type name for a field if it's an enum type in the TypeDef,
/// and return None if it's not an enum or the type cannot be resolved.
fn resolve_field_enum_type(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Option<String> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    // Unwrap Optional and Vec wrappers to get the inner type
    let inner_name = match &field.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }?;

    // Check if this is an enum type
    if enums.iter().any(|e| e.name == inner_name) {
        Some(inner_name.to_string())
    } else {
        None
    }
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

/// Returns `true` if the arg was fully emitted (caller should `continue`).
#[allow(clippy::too_many_arguments)]
fn emit_json_object_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    element_type: &Option<String>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> bool {
    match options_via {
        "dict" => {
            // When we have an array of objects and an element_type, emit dict literals (not constructor calls).
            // The bindings expect [{"type": "click", "selector": "#id"}, ...], not [PageAction(...), ...]
            if let (Some(_elem_type), Some(arr)) = (element_type, value.as_array()) {
                if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
                    let items: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_object())
                        .map(|obj| {
                            let dict_items: Vec<String> = obj
                                .iter()
                                .map(|(k, v)| {
                                    format!(
                                        "{}: {}",
                                        json_to_python_literal(&serde_json::Value::String(k.clone())),
                                        json_to_python_literal(v)
                                    )
                                })
                                .collect();
                            format!("{{{}}}", dict_items.join(", "))
                        })
                        .collect();
                    arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                    kwarg_exprs.push(var_name.to_string());
                    return true;
                }
            }
            // Fall through to default dict behavior
            let literal = json_to_python_literal(value);
            let noqa = if literal.contains("/tmp/") {
                "  # noqa: S108"
            } else {
                ""
            };
            arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "json" => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let escaped = escape_python(&json_str);
            arg_bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "from_json" => {
            if let Some(opts_type) = options_type {
                let json_str = serde_json::to_string(value).unwrap_or_default();
                let escaped = escape_python(&json_str);
                arg_bindings.push(format!("    {var_name} = {opts_type}.from_json(\"{escaped}\")"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
        _ => {
            // When we have an array with element_type, construct typed instances for Python.
            if let Some(elem_type) = element_type {
                if !value.is_null() {
                    if let Some(arr) = value.as_array() {
                        if arr.iter().all(|item| item.is_object()) {
                            let items: Vec<String> = arr
                                .iter()
                                .filter_map(|item| item.as_object())
                                .map(|obj| emit_python_typed_instance(obj, elem_type))
                                .collect();
                            arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                            kwarg_exprs.push(var_name.to_string());
                            return true;
                        }
                    }
                }
            }
            // "kwargs" mode
            if let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) {
                let kwargs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let snake_key = k.to_snake_case();
                        let py_val = if let Some(enum_type) = enum_fields.get(k) {
                            // Explicit override: use the configured enum type
                            if let Some(s) = v.as_str() {
                                let upper_val = s.to_shouty_snake_case();
                                format!("{enum_type}.{upper_val}")
                            } else {
                                json_to_python_literal(v)
                            }
                        } else if let Some(auto_enum_type) =
                            resolve_field_enum_type(k, Some(opts_type), type_defs, enums)
                        {
                            // Auto-detect: if field type is an enum, emit as EnumType.VARIANT
                            if let Some(s) = v.as_str() {
                                let upper_val = s.to_shouty_snake_case();
                                format!("{auto_enum_type}.{upper_val}")
                            } else {
                                json_to_python_literal(v)
                            }
                        } else {
                            json_to_python_literal(v)
                        };
                        format!("{snake_key}={py_val}")
                    })
                    .collect();
                let constructor = format!("{opts_type}({})", kwargs.join(", "));
                arg_bindings.push(format!("    {var_name} = {constructor}"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
    }
}

fn emit_bytes_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
) {
    if let Some(raw) = value.as_str() {
        match classify_bytes_value(raw) {
            BytesKind::FilePath => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = Path(\"{escaped}\").read_bytes()"));
            }
            BytesKind::InlineText => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = b\"{escaped}\""));
            }
            BytesKind::Base64 => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = base64.b64decode(\"{escaped}\")"));
            }
        }
    } else {
        arg_bindings.push(format!("    {var_name} = None"));
    }
    kwarg_exprs.push(var_name.to_string());
}

/// Emit a Python dict literal for a typed object-array element.
#[allow(dead_code)]
fn emit_python_object_item(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let items: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            format!(
                "{}: {}",
                json_to_python_literal(&serde_json::Value::String(k.clone())),
                json_to_python_literal(v)
            )
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// Emit a Python constructor call for a typed instance (e.g., BatchFileItem(...)).
fn emit_python_typed_instance(obj: &serde_json::Map<String, serde_json::Value>, elem_type: &str) -> String {
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            format!("{}={}", snake_key, json_to_python_literal(v))
        })
        .collect();
    format!("{}({})", elem_type, kwargs.join(", "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

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

    #[test]
    fn emit_bytes_arg_file_path_uses_path_read_bytes() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("pdf/memo.pdf".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "content");
        assert!(bindings[0].contains("Path("), "got: {:?}", bindings[0]);
        assert!(bindings[0].contains("read_bytes"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_bytes_arg_base64_uses_b64decode() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("/9j/4AAQ".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "data");
        assert!(bindings[0].contains("b64decode"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_json_object_arg_dict_mode_emits_literal() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::json!({"key": "val"});
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let done = emit_json_object_arg(
            &mut bindings,
            &mut exprs,
            &value,
            "opts",
            None,
            "dict",
            &HashMap::new(),
            &None,
            &type_defs,
            &enums,
        );
        assert!(done);
        assert!(bindings[0].contains("\"key\""), "got: {:?}", bindings[0]);
    }

    #[test]
    fn render_test_function_skipped_fixture_emits_skip_decorator() {
        use crate::e2e::fixture::{Fixture, SkipDirective};
        let fixture = Fixture {
            id: "skipped_test".to_string(),
            description: "A skipped test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: Some(SkipDirective {
                languages: vec!["python".to_string()],
                reason: Some("not supported".to_string()),
            }),
            env: None,
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
        render_test_function(
            &mut out,
            &fixture,
            &e2e_config,
            &config,
            &type_defs,
            &enums,
            None,
            "kwargs",
            &HashMap::new(),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(out.contains("pytest.mark.skip"), "got: {out}");
        assert!(out.contains("not supported"), "got: {out}");
    }

    #[test]
    fn resolve_field_enum_type_detects_enum_field() {
        use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

        let enum_def = EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "module::TierStrategy".to_string(),
            variants: vec![EnumVariant {
                name: "Auto".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let type_def = TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "module::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "tier_strategy".to_string(),
                ty: TypeRef::Named("TierStrategy".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let enums = vec![enum_def];
        let type_defs = vec![type_def];

        let result = resolve_field_enum_type("tier_strategy", Some("ConversionOptions"), &type_defs, &enums);
        assert_eq!(result, Some("TierStrategy".to_string()));
    }

    #[test]
    fn resolve_field_enum_type_returns_none_for_non_enum_field() {
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};

        let type_def = TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "module::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Named("u64".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let enums: Vec<crate::core::ir::EnumDef> = vec![];
        let type_defs = vec![type_def];

        let result = resolve_field_enum_type("timeout", Some("ConversionOptions"), &type_defs, &enums);
        assert_eq!(result, None);
    }
}
