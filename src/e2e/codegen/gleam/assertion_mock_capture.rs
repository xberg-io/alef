//! `mock.*` request-count virtual field assertion rendering for Gleam e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `kotlin::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `gleam::test_file::render_test_file`), not a struct field, so interception happens as the very
//! first statement in `assertions::render_assertion` -- before the `is_valid_for_result` gate,
//! which would otherwise misclassify a `mock.*` field as `FieldSkip::NotAvailableOnResultType`
//! (blaming the fixture) rather than alef's own generator gap. gleeunit's `should.equal` takes no
//! failure-message argument (unlike JUnit/`kotlin.test`), so unlike the Java/Kotlin renderers this
//! module never builds or escapes a message string.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// Bind the mock server's base URL to `mock_base_url__`, read from the same `MOCK_SERVER_URL`
/// environment variable every other per-fixture mock-server lookup in this backend already reads
/// (`http.rs`'s `GleamTestClientRenderer::render_call`, `test_case.rs`'s client-factory branch),
/// falling back to the same `"http://localhost:8080"` default. A `case` expression cannot be
/// inlined as a single-line function argument in Gleam, so this binds a name first -- re-binding
/// per assertion is harmless (Gleam `let` shadows freely) and keeps this module independent of
/// whatever the fixture's own call-setup code already bound. ~keep
fn render_base_url_binding(out: &mut String) {
    let _ = writeln!(out, "  let mock_base_url__ = case envoy.get(\"MOCK_SERVER_URL\") {{");
    let _ = writeln!(out, "    Ok(u) -> u");
    let _ = writeln!(out, "    Error(_) -> \"http://localhost:8080\"");
    let _ = writeln!(out, "  }}");
}

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to return
/// immediately rather than fall through to ordinary field resolution.
pub(super) fn try_render_mock_capture_assertion(out: &mut String, assertion: &Assertion) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let capture = mock_capture("gleam").unwrap_or_else(|_| {
        panic!(
            "Gleam e2e generator: mock.* field '{field}' has no gleam capture-table entry (mock_assertions::snippets)"
        )
    });
    let path_and_query = build_path_and_query(&query);
    render_base_url_binding(out);
    let expr = format!("{}(mock_base_url__, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `Int`) into the same `gleeunit/should` family every
/// other numeric comparison in this backend already uses -- the `|> fn(n__) { n__ OP n } |>
/// should.equal(True)` idiom `assertions.rs` already applies for `count_min`/`greater_than`/
/// `less_than`, since `|>` cannot pipe directly into a bare comparison operator.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Gleam e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Gleam e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "  {expr} |> should.equal({n})");
        }
        "not_equals" => {
            let _ = writeln!(out, "  {expr} |> fn(n__) {{ n__ != {n} }} |> should.equal(True)");
        }
        "greater_than" => {
            let _ = writeln!(out, "  {expr} |> fn(n__) {{ n__ > {n} }} |> should.equal(True)");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "  {expr} |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)");
        }
        "less_than" => {
            let _ = writeln!(out, "  {expr} |> fn(n__) {{ n__ < {n} }} |> should.equal(True)");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "  {expr} |> fn(n__) {{ n__ <= {n} }} |> should.equal(True)");
        }
        other => {
            panic!("Gleam e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
