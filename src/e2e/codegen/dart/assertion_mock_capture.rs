//! `mock.*` request-count virtual field assertion rendering for Dart e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `ruby::assertion_mock_capture`: `mock.*` fields (see
//! `mock_assertions::model`) resolve against the generated mock server's own request log through
//! the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `dart::test_file::render_test_file`), not a field on the FRB-generated result type.
//!
//! Called from BOTH `assertions::render_assertion_dart` (the non-streaming path) AND
//! `assertions::render_streaming_assertion_dart` (the streaming path): Dart routes a streaming
//! fixture's assertions through a wholly separate function
//! (`test_case.rs::emit_call_and_assertions` dispatches on `is_streaming` and calls
//! `render_streaming_assertion_dart` instead of `render_assertion_dart`, never both), so a single
//! interception point would silently miss `mock.*` on any streaming Dart fixture -- the exact
//! wrong-backend-wins-the-guard trap `mock_assertions::ensure_capture_declared` exists to prevent.
//! ~keep

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

use super::values::escape_dart;

/// Dart expression for the mock server's base URL. `dart/test_file.rs` already defines a
/// top-level `_sutUrl()` (preferring the per-file spawned server URL over the `MOCK_SERVER_URL` /
/// `SUT_URL` environment variables) for every other mock-URL call site in this backend
/// (`dart/http.rs`'s HTTP fixture renderer reads the identical `_sutUrl()`), and
/// `test_file::render_test_file` is extended by this change to guarantee `_sutUrl()` is emitted
/// whenever a fixture asserts a `mock.*` field, even one with no `mock_url` arg of its own.
const MOCK_SERVER_URL_EXPR: &str = "_sutUrl()";

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
    let capture = mock_capture("dart").unwrap_or_else(|_| {
        panic!("Dart e2e generator: mock.* field '{field}' has no dart capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    // Every generated test body is `async` (see `test_case.rs`'s own doc comment), so `await`ing
    // the helper's `Future<int>` directly inside the `expect(...)` argument list below compiles.
    let expr = format!("await {}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an awaited `int`) into the same `package:test` matcher
/// families every other numeric comparison in this backend already uses (see
/// `assertions::render_assertion_dart`'s `greater_than`/`less_than`/etc. arms).
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Dart e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Dart e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted `expect(...)` failure reason below, inside a
    // single-quoted Dart string literal -- `mock.requests["POST /v1/chat"]` carries literal `"`
    // characters, which a single-quoted literal does not need to escape, but the message is still
    // run through `escape_dart` for the backslash/`'` cases it does need.
    let escaped_field = escape_dart(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    expect({expr}, equals({n}), reason: 'expected {escaped_field} == {n}');");
        }
        "not_equals" => {
            let _ = writeln!(
                out,
                "    expect({expr}, isNot(equals({n})), reason: 'expected {escaped_field} != {n}');"
            );
        }
        "greater_than" => {
            let _ = writeln!(
                out,
                "    expect({expr}, greaterThan({n}), reason: 'expected {escaped_field} > {n}');"
            );
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "    expect({expr}, greaterThanOrEqualTo({n}), reason: 'expected {escaped_field} >= {n}');"
            );
        }
        "less_than" => {
            let _ = writeln!(out, "    expect({expr}, lessThan({n}), reason: 'expected {escaped_field} < {n}');");
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "    expect({expr}, lessThanOrEqualTo({n}), reason: 'expected {escaped_field} <= {n}');"
            );
        }
        other => {
            panic!("Dart e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
