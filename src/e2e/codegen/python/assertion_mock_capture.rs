//! `mock.*` request-count virtual field assertion rendering for Python e2e tests (alef issue
//! #443).
//!
//! Mirrors `rust::assertion_mock_capture`: `mock.*` fields (see `mock_assertions::model`) resolve
//! against the generated mock server's own request log through the once-per-suite helper
//! `mock_assertions::render_helper` emits into the file, not a struct field.
//!
//! Called directly from `test_function::result_assertions::emit_result_and_assertions`'s own
//! per-assertion loop -- in BOTH its streaming and non-streaming branches, since a `mock.*`
//! assertion is legal on either kind of fixture -- rather than from `assertions::render_assertion`
//! itself. That mirrors how the streaming-virtual-field interception already works in this
//! backend (`emit_streaming_virtual_assertion` is also called from that same loop, never from
//! `render_assertion`): `render_assertion` has no `is_async` parameter, and this module needs
//! one to decide how the (synchronous) helper is invoked, so intercepting one level up -- where
//! `is_async` is already in scope -- avoids adding that parameter to `render_assertion`'s eleven
//! other call sites, none of which need it.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_python;
use crate::e2e::fixture::Assertion;

/// Python expression for the mock server's base URL -- the single mock server `conftest.py`
/// spawns once per test session (`src/e2e/templates/python/conftest.py.jinja`), not a per-test
/// instance, so every fixture reaches it through this same environment variable.
const MOCK_SERVER_URL_EXPR: &str = "os.environ[\"MOCK_SERVER_URL\"]";

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled).
///
/// `is_async` selects how the (synchronous) helper is invoked: an `async def` test awaits it via
/// `asyncio.to_thread` rather than blocking the event loop directly (see
/// `mock_request_helper.py.jinja`'s own doc comment); a plain `def` test calls it directly, since
/// there is no event loop to block and `await` is not valid there at all.
pub(super) fn try_render_mock_capture_assertion(out: &mut String, assertion: &Assertion, is_async: bool) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let capture = mock_capture("python").unwrap_or_else(|_| {
        panic!(
            "Python e2e generator: mock.* field '{field}' has no python capture-table entry (mock_assertions::snippets)"
        )
    });
    let path_and_query = build_path_and_query(&query);
    let call_expr = if is_async {
        format!(
            "await asyncio.to_thread({}, {MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")",
            capture.helper_name
        )
    } else {
        format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name)
    };
    render_mock_capture_comparison(out, assertion, field, &call_expr);
    true
}

/// Feed the mock-request-count `expr` (an `int`) into the same numeric comparison operators every
/// other integer-valued field already supports.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Python e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Python e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion below, inside a Python string literal --
    // `mock.requests["POST /v1/chat"]` carries literal `"` characters that must be escaped there.
    let escaped_field = escape_python(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    assert {expr} == {n}, \"expected {escaped_field} == {n}\"");
        }
        "not_equals" => {
            let _ = writeln!(out, "    assert {expr} != {n}, \"expected {escaped_field} != {n}\"");
        }
        "greater_than" => {
            let _ = writeln!(out, "    assert {expr} > {n}, \"expected {escaped_field} > {n}\"");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "    assert {expr} >= {n}, \"expected {escaped_field} >= {n}\"");
        }
        "less_than" => {
            let _ = writeln!(out, "    assert {expr} < {n}, \"expected {escaped_field} < {n}\"");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "    assert {expr} <= {n}, \"expected {escaped_field} <= {n}\"");
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
