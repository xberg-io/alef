//! `mock.*` request-count virtual field assertion rendering for Ruby e2e tests (alef issue #443).
//!
//! Mirrors `rust::assertion_mock_capture` / `python::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `ruby::spec_file::render_spec_file`), not a struct attribute.
//!
//! Called from BOTH `assertions::render_assertion` (the non-streaming example path,
//! `examples.rs`) AND `streaming_assertion::emit_chat_stream_assertion` (the streaming example
//! path): unlike every other backend in this wave, Ruby routes a streaming fixture's assertions
//! through a wholly separate function that never calls `render_assertion` at all
//! (`examples::render_chat_stream_example` calls `emit_chat_stream_assertion` directly), so a
//! single interception point would silently miss `mock.*` on any streaming Ruby fixture -- the
//! exact wrong-backend-wins-the-guard trap `mock_assertions::ensure_capture_declared` exists to
//! prevent. ~keep

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_ruby_single;
use crate::e2e::fixture::Assertion;

/// Ruby expression for the mock server's base URL -- every other per-fixture mock-URL call site
/// in this backend (`args.rs`, `examples.rs`, `test_file.jinja`'s own `mock_server_url` `let`)
/// already reads it through this same environment variable.
const MOCK_SERVER_URL_EXPR: &str = "ENV.fetch('MOCK_SERVER_URL')";

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
    let capture = mock_capture("ruby").unwrap_or_else(|_| {
        panic!("Ruby e2e generator: mock.* field '{field}' has no ruby capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `Integer`) into the same RSpec matcher families every
/// other numeric assertion in this backend already uses.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Ruby e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Ruby e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted `expect(...)` failure message below, inside a
    // single-quoted Ruby string literal -- `mock.requests["POST /v1/chat"]` carries literal `"`
    // characters, which a single-quoted literal does not need to escape, but the message itself
    // is still run through `escape_ruby_single` for the backslash/`'` cases it does need.
    let escaped_field = escape_ruby_single(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    expect({expr}).to eq({n}), 'expected {escaped_field} == {n}'");
        }
        "not_equals" => {
            let _ = writeln!(
                out,
                "    expect({expr}).not_to eq({n}), 'expected {escaped_field} != {n}'"
            );
        }
        "greater_than" => {
            let _ = writeln!(out, "    expect({expr}).to be > {n}, 'expected {escaped_field} > {n}'");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "    expect({expr}).to be >= {n}, 'expected {escaped_field} >= {n}'"
            );
        }
        "less_than" => {
            let _ = writeln!(out, "    expect({expr}).to be < {n}, 'expected {escaped_field} < {n}'");
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "    expect({expr}).to be <= {n}, 'expected {escaped_field} <= {n}'"
            );
        }
        other => {
            panic!("Ruby e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
