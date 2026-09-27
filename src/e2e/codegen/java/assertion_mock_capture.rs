//! `mock.*` request-count virtual field assertion rendering for Java e2e tests (alef issue #443).
//!
//! Mirrors `rust::assertion_mock_capture` / `python::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `java::test_file::render_test_file`), not a struct field, so interception happens before
//! every other branch in `assertions::render_assertion`, including the `result_is_option`/
//! `result_is_bytes` special cases, which would otherwise misclassify a `mock.*` field as a
//! bare-result assertion. The helper is a private static method of the generated JUnit test
//! class, so every call site here reads a bare method call (same class, no qualifier needed).

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_java;
use crate::e2e::fixture::Assertion;

/// Java expression for the mock server's base URL -- every other per-fixture mock-URL call site
/// in this backend (`args.rs`, `test_method.rs`) already reads it through this same environment
/// variable.
const MOCK_SERVER_URL_EXPR: &str = "System.getenv(\"MOCK_SERVER_URL\")";

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
    let capture = mock_capture("java").unwrap_or_else(|_| {
        panic!("Java e2e generator: mock.* field '{field}' has no java capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `int`) into the same JUnit `assertEquals`/etc. family
/// every other numeric comparison in this backend already uses.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Java e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Java e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion message below, inside a Java string literal --
    // `mock.requests["POST /v1/chat"]` carries literal `"` characters that must be escaped there.
    let escaped_field = escape_java(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(
                out,
                "        assertEquals({n}, {expr}, \"expected {escaped_field} == {n}\");"
            );
        }
        "not_equals" => {
            let _ = writeln!(
                out,
                "        assertNotEquals({n}, {expr}, \"expected {escaped_field} != {n}\");"
            );
        }
        "greater_than" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} > {n}, \"expected {escaped_field} > {n}\");"
            );
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} >= {n}, \"expected {escaped_field} >= {n}\");"
            );
        }
        "less_than" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} < {n}, \"expected {escaped_field} < {n}\");"
            );
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} <= {n}, \"expected {escaped_field} <= {n}\");"
            );
        }
        other => {
            panic!("Java e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
