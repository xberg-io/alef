//! `mock.*` request-count virtual field assertion rendering for Rust e2e tests (alef issue #443).
//!
//! Mirrors `assertion_streaming.rs`: `mock.*` fields (see `mock_assertions::model`) resolve
//! against the generated mock server's own request log through the once-per-suite helper
//! `mock_assertions::render_helper` emits into the file (see
//! `rust::test_file::file_rendering::render_test_file`), not a struct field, so interception
//! happens before `is_valid_for_result`, exactly like streaming virtual fields.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_rust;
use crate::e2e::fixture::Assertion;

/// Rust expression for the mock server's base URL.
///
/// `rust::test_file::test_function::finalize_test_body` decides the `mock_server`/`_mock_server`
/// binding name by scanning the rendered test body for the literal substring `"mock_server."` --
/// keep this expression starting with exactly that, or a fixture whose only mock-server reference
/// is a `mock.*` assertion silently gets the `_mock_server` (never-read) binding and fails to
/// compile with an unresolved `mock_server` name. ~keep
const MOCK_SERVER_URL_EXPR: &str = "mock_server.url.as_str()";

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to return
/// immediately rather than fall through to struct-field assertion handling.
pub(super) fn try_render_mock_capture_assertion(out: &mut String, assertion: &Assertion) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let capture = mock_capture("rust").unwrap_or_else(|_| {
        panic!("Rust e2e generator: mock.* field '{field}' has no rust capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!(
        "{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\").await",
        capture.helper_name
    );
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (a `u64`) into the same numeric comparison operators every
/// other integer-valued field already supports.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Rust e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Rust e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion MESSAGE below, i.e. inside a Rust string
    // literal in the generated file -- `mock.requests["POST /v1/chat"]` carries literal `"`
    // characters that must be escaped there, unlike every other (host-side, compile-time) `panic!`
    // message in this function.
    let escaped_field = escape_rust(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    assert_eq!({expr}, {n}, \"expected {escaped_field} == {n}\");");
        }
        "not_equals" => {
            let _ = writeln!(out, "    assert_ne!({expr}, {n}, \"expected {escaped_field} != {n}\");");
        }
        "greater_than" => {
            let _ = writeln!(out, "    assert!({expr} > {n}, \"expected {escaped_field} > {n}\");");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "    assert!({expr} >= {n}, \"expected {escaped_field} >= {n}\");");
        }
        "less_than" => {
            let _ = writeln!(out, "    assert!({expr} < {n}, \"expected {escaped_field} < {n}\");");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "    assert!({expr} <= {n}, \"expected {escaped_field} <= {n}\");");
        }
        other => {
            panic!("Rust e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
