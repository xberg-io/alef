//! Result and streaming assertion rendering for generated Python tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::super::assertions::render_assertion;
use super::super::helpers::{resolve_assert_enum_fields, strip_redundant_call_arg_parens};
use super::super::json::value_to_python_string;

/// True when `body` contains at least one line that is not blank and not a
/// `#`-prefixed comment — i.e. an executable `assert` statement. A body made
/// up only of "# skipped: ..." comments is not executable.
fn has_real_assertion(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    })
}

/// When a fixture declares at least one assertion but the rendered body has
/// no executable statement — its only assertion was `not_error`, or every
/// field assertion resolved to a "skipped" comment — inject a real assertion
/// on the result instead of leaving the test vacuous. Fixtures that declare
/// NO assertions at all are left untouched: that's a pre-existing, intentional
/// "just call it" smoke test contract (see
/// `should_discard_result_when_force_bind_result_is_unset_and_unused`). ~keep
fn apply_vacuous_assertion_fallback(
    temp_assertions: &mut String,
    has_declared_assertions: bool,
    result_var: &str,
    returns_void: bool,
) {
    if !has_declared_assertions || has_real_assertion(temp_assertions) {
        return;
    }
    // ~keep A void call's binding return is PyO3's mapping of Rust `()` — Python `None` — on
    // every successful call, so `assert result is not None` would fail every successful call,
    // not just an unsuccessful one: a guaranteed-red test, worse than the vacuous body it was
    // meant to replace. There is no result to assert on; a bare, unbound call statement is
    // already non-vacuous in pytest, since an uncaught exception fails the test on its own.
    if returns_void {
        return;
    }
    let _ = writeln!(temp_assertions, "    assert {result_var} is not None");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_result_and_assertions(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &crate::e2e::config::CallConfig,
    call_expr: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    is_streaming: bool,
    is_async: bool,
    force_bind_result: bool,
    streaming_item_type: Option<&str>,
) {
    // Streaming virtual fields resolve against the collected `chunks` list, not
    // the result type.
    //
    // ~keep This used to be preceded by a `let _ = fixture.assertions.iter()
    // .any(...)` closure computing a `has_usable_assertion`-shaped predicate
    // (excluding not_error/error, accepting streaming-virtual and
    // result_is_simple fields) whose result was discarded (`let _ =`) and never
    // referenced anywhere in this function — dead code that looked like a check.
    // The real usability decision lives below, derived from what
    // `apply_vacuous_assertion_fallback`/`temp_assertions` actually render, not
    // a separately maintained predicate that can drift out of sync with it (see
    // the php/typescript/ruby fixes for the same drift in this defect class).
    let chunks_var = "chunks";

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
        // Render streaming assertions into a buffer first (not directly into `out`) so
        // the vacuous-fallback and strict-availability checks below see the whole body,
        // mirroring the non-streaming branch. Before this, a streaming fixture whose only
        // assertions were non-streaming-virtual field checks rendered NO output at all —
        // not even a skip comment — leaving a vacuously-passing test with no fallback. ~keep
        let mut streaming_assertions = String::new();
        for assertion in &fixture.assertions {
            if assertion.assertion_type == "not_error" || assertion.assertion_type == "error" {
                continue;
            }
            if let Some(f) = &assertion.field
                && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
            {
                emit_streaming_virtual_assertion(
                    &mut streaming_assertions,
                    assertion,
                    f,
                    chunks_var,
                    streaming_item_type,
                    field_resolver.python_typeddict_map(),
                );
                continue;
            }
            // `mock.*` request-count virtual fields (alef issue #443) are legal on a streaming
            // fixture too (asserting the request count alongside the drained chunks), so they
            // must be checked here rather than falling into the generic
            // "not available on streaming result type" skip below.
            if super::super::assertion_mock_capture::try_render_mock_capture_assertion(
                &mut streaming_assertions,
                assertion,
                is_async,
            ) {
                continue;
            }
            // Non-streaming-virtual assertions on streaming fixtures are skipped
            // (the result type doesn't have these fields during iteration).
            if let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty()) {
                let _ = writeln!(
                    streaming_assertions,
                    "    # skipped: {}",
                    FieldSkip::NotAvailableOnStreamingResultType.message(f)
                );
            }
        }
        apply_vacuous_assertion_fallback(
            &mut streaming_assertions,
            !fixture.assertions.is_empty(),
            chunks_var,
            call_config.returns_void,
        );
        crate::e2e::codegen::fail_on_unavailable_field_markers(
            &streaming_assertions,
            "python",
            &fixture.id,
            &fixture.assertions,
        );
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&streaming_assertions, "python", &fixture.id);
        out.push_str(&streaming_assertions);
    } else {
        // For non-streaming: render assertions to a temporary buffer first,
        // then check if result_var is referenced. Only emit the assignment if it is.
        let mut temp_assertions = String::new();

        for assertion in &fixture.assertions {
            // `not_error` has no explicit rendering: an uncaught exception already
            // fails the test, so the check is implicit in the call succeeding.
            if assertion.assertion_type == "not_error" {
                continue;
            }
            // `mock.*` request-count virtual fields (alef issue #443): intercept before
            // `render_assertion`, which has no `is_async` parameter -- see this module's doc
            // comment for why the interception lives here instead.
            if super::super::assertion_mock_capture::try_render_mock_capture_assertion(
                &mut temp_assertions,
                assertion,
                is_async,
            ) {
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

        apply_vacuous_assertion_fallback(
            &mut temp_assertions,
            !fixture.assertions.is_empty(),
            result_var,
            call_config.returns_void,
        );
        crate::e2e::codegen::fail_on_unavailable_field_markers(
            &temp_assertions,
            "python",
            &fixture.id,
            &fixture.assertions,
        );
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&temp_assertions, "python", &fixture.id);

        // Check if result_var appears in actual code (not in comments).
        // Only count lines that start with "assert" or contain actual code tokens.
        // Comments (lines starting with #) are skipped to avoid false positives
        // from strings like "field 'result' not available" in comment text.
        let result_var_used = temp_assertions.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with('#') && trimmed.contains(result_var)
        });

        let result_binding =
            (result_var_used || fixture.has_docs_presentation() || force_bind_result).then_some(result_var);
        out.push_str(&crate::e2e::template_env::render(
            "python/call_statement.py.jinja",
            crate::alef_context! { result_binding => result_binding, call_expr => call_expr },
        ));
        out.push_str(&temp_assertions);
    }
}

/// Emit a Python assertion for a streaming virtual field using the collected
/// `chunks` list.  Mirrors the pattern in rust/assertions.rs.
///
/// `streaming_item_type` and `typeddict_map` let the accessor render subscript access
/// (`c["field"]`) instead of `.field` when the stream chunk type (or a type it transitively
/// owns, e.g. its `choices`/`delta`) is one the pyo3 backend emits as a `TypedDict` — see
/// `streaming_assertions::accessors::accessor_with_typeddict_map`. `streaming_item_type: None`
/// (unresolved from `alef.toml`) and an empty `typeddict_map` both fall back to the pre-existing
/// dotted-access rendering.
fn emit_streaming_virtual_assertion(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    chunks_var: &str,
    streaming_item_type: Option<&str>,
    typeddict_map: &crate::e2e::field_access::PythonTypedDictMap,
) {
    use crate::e2e::codegen::streaming_assertions::StreamingFieldResolver;

    let Some(expr) = StreamingFieldResolver::accessor_with_typeddict_map(
        field,
        "python",
        chunks_var,
        None,
        streaming_item_type,
        Some(typeddict_map),
    ) else {
        let _ = writeln!(
            out,
            "    # skipped: {}",
            FieldSkip::NoPythonStreamingAccessor.message(field)
        );
        return;
    };

    // Defensive, matching the same fix at the `FieldResolver`-backed assertion sites
    // (`python/assertion.jinja`'s `not_empty`/`min_length`/etc.): a `StreamingFieldResolver`
    // accessor is hand-rolled per field today and never itself produces an enclosing-paren-
    // wrapped ternary, but should one gain optional-narrowing in the future, stripping here
    // keeps `len(expr)` from turning into ruff-`UP034`-flagged `len((expr))`. No-op otherwise. ~keep
    let expr_arg = strip_redundant_call_arg_parens(&expr);
    match assertion.assertion_type.as_str() {
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({expr_arg}) >= {n}");
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({expr_arg}) == {n}");
            }
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                if val.is_string() {
                    let _ = writeln!(out, "    assert {expr}.strip() {op} {expected}.strip()");
                } else {
                    let _ = writeln!(out, "    assert {expr} {op} {expected}");
                }
            }
        }
        "not_empty" => {
            // Bare truthiness would reject a legitimate 0/0.0/False. Only sized values
            // carry an emptiness notion; everything else just has to be present.
            // `expr_arg` (not `expr`) inside `hasattr`/`len`: both are call-argument position,
            // where an enclosing paren pair would be redundant; `expr` itself keeps its parens
            // before `is not None`, where they're load-bearing.
            let _ = writeln!(
                out,
                "    assert {expr} is not None and (not hasattr({expr_arg}, \"__len__\") or len({expr_arg}) > 0)"
            );
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert not {expr}");
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
            let _ = writeln!(out, "    assert {py_expr}");
        }
        "is_false" => {
            let py_expr = if expr == "true" {
                "True".to_string()
            } else if expr == "false" {
                "False".to_string()
            } else {
                expr.clone()
            };
            let _ = writeln!(out, "    assert not {py_expr}");
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {expected}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} >= {expected}");
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expected} in {expr}");
            }
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on synthetic field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "result_assertions/tests.rs"]
mod tests;
