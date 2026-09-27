//! `mock.*` request-count virtual field assertion rendering for Elixir e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `kotlin::assertion_mock_capture`: `mock.*` fields (see
//! `mock_assertions::model`) resolve against the generated mock server's own request log through
//! the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `elixir::test_file::render_test_file`), not a struct field, so interception happens before
//! every other branch in `assertions::render_assertion` -- including the synthetic-field match
//! arms (`chunks_have_content`, `embeddings`, ...) and the streaming-virtual-field branch, either
//! of which would otherwise fall through to `field_resolver.is_valid_for_result`, which rejects
//! `mock.requests.total` as `FieldSkip::NotAvailableOnResultType` (blaming the fixture) rather
//! than alef's own generator gap. The helper is a private function (`defp`) of the generated
//! `ExUnit.Case` module, so every call site here reads a bare function call (same module, no
//! qualifier needed).

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

/// Elixir expression for the mock server's base URL -- a raw `System.get_env/1` expression
/// (not a call to this file's own `mock_server_url/0` private function), since that helper is
/// only emitted when the fixture group has an HTTP fixture (`test_file.rs`'s `has_http` gate) and
/// a `mock.*` assertion can appear on a fixture that trips no such flag (`mock_response` /
/// `client_factory` alone). The fallback matches `mock_server_url/0`'s own default verbatim. ~keep
const MOCK_SERVER_URL_EXPR: &str = "(System.get_env(\"MOCK_SERVER_URL\") || \"http://localhost:8080\")";

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
    let capture = mock_capture("elixir").unwrap_or_else(|_| {
        panic!(
            "Elixir e2e generator: mock.* field '{field}' has no elixir capture-table entry (mock_assertions::snippets)"
        )
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an integer) into the same bare `assert`/`refute` idiom
/// every other numeric comparison in this backend already uses (see `assertions.rs`, none of
/// which attach a message to the emitted `assert`/`refute` line).
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Elixir e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Elixir e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "      assert {expr} == {n}");
        }
        "not_equals" => {
            let _ = writeln!(out, "      assert {expr} != {n}");
        }
        "greater_than" => {
            let _ = writeln!(out, "      assert {expr} > {n}");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "      assert {expr} >= {n}");
        }
        "less_than" => {
            let _ = writeln!(out, "      assert {expr} < {n}");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "      assert {expr} <= {n}");
        }
        other => {
            panic!("Elixir e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
