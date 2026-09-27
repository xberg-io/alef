//! `mock.*` request-count virtual field assertion rendering for Go e2e tests (alef issue #443).
//!
//! Mirrors `rust::assertion_mock_capture` / `python::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `go::test_file::render_test_file`), not a struct field, so interception happens before
//! wildcard/field resolution, exactly like streaming virtual fields (`assertion_streaming` /
//! `assertions::streaming_fields`).

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_go;
use crate::e2e::fixture::Assertion;

/// Go expression for the mock server's base URL -- every other per-fixture mock-URL call site in
/// this backend (`test_file.rs`, `args.rs`) already reads it through this same environment
/// variable, set once by the test harness for the whole run. ~keep
const MOCK_SERVER_URL_EXPR: &str = "os.Getenv(\"MOCK_SERVER_URL\")";

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
    let capture = mock_capture("go").unwrap_or_else(|_| {
        panic!("Go e2e generator: mock.* field '{field}' has no go capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    render_mock_capture_comparison(out, assertion, field, capture.helper_name, &path_and_query);
    true
}

/// Feed the mock-request-count call (an `int`) into a `t.Errorf`-guarded comparison, matching the
/// plain `if ... { t.Errorf(...) }` idiom every other numeric comparison in this backend already
/// uses (`assertions/comparisons.rs`) rather than pulling in `testify/assert` for this one case.
/// Wrapped in its own `{ ... }` block so multiple `mock.*` assertions in one test function don't
/// collide on the `v` binding.
fn render_mock_capture_comparison(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    helper_name: &str,
    path_and_query: &str,
) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Go e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Go e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted `t.Errorf` message below, inside a Go string literal --
    // `mock.requests["POST /v1/chat"]` carries literal `"` characters that must be escaped there.
    let escaped_field = escape_go(field);
    let (fail_if, word) = match assertion_type {
        "equals" | "count_equals" => ("!=", "=="),
        "not_equals" => ("==", "!="),
        "greater_than" => ("<=", ">"),
        "greater_than_or_equal" | "count_min" => ("<", ">="),
        "less_than" => (">=", "<"),
        "less_than_or_equal" => (">", "<="),
        other => {
            panic!("Go e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    };
    let _ = writeln!(out, "\t{{");
    let _ = writeln!(
        out,
        "\t\tv := {helper_name}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")"
    );
    let _ = writeln!(out, "\t\tif v {fail_if} {n} {{");
    let _ = writeln!(
        out,
        "\t\t\tt.Errorf(\"expected {escaped_field} {word} {n}, got %d\", v)"
    );
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
