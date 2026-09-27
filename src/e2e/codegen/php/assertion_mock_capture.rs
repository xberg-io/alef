//! `mock.*` request-count virtual field assertion rendering for PHP e2e tests (alef issue #443).
//!
//! Mirrors `rust::assertion_mock_capture` / `python::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `php::test_file::render_test_file`), not a struct property, so interception happens before
//! `is_valid_for_result`, exactly like streaming virtual fields (`php::assertions`'s own
//! streaming-virtual-field block). The helper is a PRIVATE METHOD of the generated PHPUnit test
//! class (PHP has no bare top-level functions inside a class-scoped file the way rust/python do),
//! so every call site here reads `$this->{{ helper_name }}(...)`, not a bare function call.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_php;
use crate::e2e::fixture::Assertion;

/// PHP expression for the mock server's base URL -- every other per-fixture mock-URL call site
/// in this backend (`args.rs`, `test_method.rs`, `test_file.jinja`'s own `setUp()`) already
/// reads it through this same environment variable.
const MOCK_SERVER_URL_EXPR: &str = "getenv('MOCK_SERVER_URL')";

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
    let capture = mock_capture("php").unwrap_or_else(|_| {
        panic!("PHP e2e generator: mock.* field '{field}' has no php capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!(
        "$this->{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")",
        capture.helper_name
    );
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `int`) into the same PHPUnit assertion families every
/// other numeric comparison in this backend already uses (`php::assertions`'s
/// `$this->assertGreaterThan`/etc. calls).
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "PHP e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "PHP e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion message below, inside a PHP double-quoted
    // string literal -- `mock.requests["POST /v1/chat"]` carries literal `"` characters that
    // must be escaped there.
    let escaped_field = escape_php(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(
                out,
                "        $this->assertEquals({n}, {expr}, \"expected {escaped_field} == {n}\");"
            );
        }
        "not_equals" => {
            let _ = writeln!(
                out,
                "        $this->assertNotEquals({n}, {expr}, \"expected {escaped_field} != {n}\");"
            );
        }
        "greater_than" => {
            let _ = writeln!(
                out,
                "        $this->assertGreaterThan({n}, {expr}, \"expected {escaped_field} > {n}\");"
            );
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "        $this->assertGreaterThanOrEqual({n}, {expr}, \"expected {escaped_field} >= {n}\");"
            );
        }
        "less_than" => {
            let _ = writeln!(
                out,
                "        $this->assertLessThan({n}, {expr}, \"expected {escaped_field} < {n}\");"
            );
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "        $this->assertLessThanOrEqual({n}, {expr}, \"expected {escaped_field} <= {n}\");"
            );
        }
        other => {
            panic!("PHP e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
