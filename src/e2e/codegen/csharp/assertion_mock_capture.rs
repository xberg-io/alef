//! `mock.*` request-count virtual field assertion rendering for C# e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture`: `mock.*` fields (see `mock_assertions::model`)
//! resolve against the generated mock server's own request log through the once-per-suite
//! helper `mock_assertions::render_helper` emits into the file
//! ([`append_helper_if_referenced`], called from `csharp::render_test_file`), not a struct
//! field, so interception happens before every other branch in both of this backend's assertion
//! renderers -- see the module-level doc on why there are two.
//!
//! ~keep C# has TWO assertion call sites, not one: `assertions::render_assertion` handles every
//! ordinary (non-streaming) fixture, but a streaming fixture never reaches it --
//! `streaming::render_streaming_test_method` dispatches straight to
//! `streaming::emit_chat_stream_assertion` (chat-completion streams) or
//! `streaming::emit_non_chat_stream_assertion` (generic streams), neither of which calls
//! `assertions::render_assertion`. A `mock.*` assertion on a streaming C# fixture that only
//! intercepted in `assertions.rs` would silently fall through to
//! `emit_non_chat_stream_assertion`'s `result_fields`-membership check and render
//! `FieldSkip::StreamingAssertionOnUnsupportedField` instead -- the same misattribution
//! `mock_assertions::ensure_capture_declared`'s own doc warns about, just reached by a second
//! path the gate itself cannot see (the gate only knows whether `mock_capture("csharp")` is
//! `Ok`, not which of a backend's renderers actually intercepts). All three call sites --
//! `assertions::render_assertion`, `streaming::emit_chat_stream_assertion`,
//! `streaming::emit_non_chat_stream_assertion` -- must call
//! [`try_render_mock_capture_assertion`] first.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// C# expression for the mock server's base URL -- the same `Environment.GetEnvironmentVariable`
/// fallback every other per-fixture mock-URL call site in this backend already uses (see
/// `csharp/http.rs`'s `render_http_test_method`). `Environment` resolves unqualified because
/// `using System;` is in every generated test file's unconditional using-list.
const MOCK_SERVER_URL_EXPR: &str =
    "Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\"";

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
    let capture = mock_capture("csharp").unwrap_or_else(|_| {
        panic!("C# e2e generator: mock.* field '{field}' has no csharp capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `int`) into the same `Assert.*` family every other
/// numeric comparison in this backend already uses -- see `csharp/assertions.rs`'s
/// `result_is_bytes` branch for the `Assert.Equal`/`Assert.True` precedent this follows. Unlike
/// Java's counterpart, no message argument is added: this backend's existing assertions never
/// pass one (xUnit's `Assert.Equal`/`Assert.True` overloads used elsewhere here take no string
/// message), so none is introduced here either.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "C# e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "C# e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "        Assert.Equal({n}, {expr});");
        }
        "not_equals" => {
            let _ = writeln!(out, "        Assert.NotEqual({n}, {expr});");
        }
        "greater_than" => {
            let _ = writeln!(out, "        Assert.True({expr} > {n});");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "        Assert.True({expr} >= {n});");
        }
        "less_than" => {
            let _ = writeln!(out, "        Assert.True({expr} < {n});");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "        Assert.True({expr} <= {n});");
        }
        other => {
            panic!("C# e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

/// Appends the once-per-suite mock-request-count helper to `fixtures_body` iff a fixture already
/// rendered into it actually calls the helper -- mirrors `java::test_file::render_test_file`'s
/// inline gate. Extracted to its own function (rather than inlined in `csharp.rs`) because
/// `csharp.rs` is already over the file-size ratchet's 1,000-line cap and must not grow.
pub(super) fn append_helper_if_referenced(fixtures_body: &mut String) {
    let Ok(capture) = mock_capture("csharp") else {
        return;
    };
    if !fixtures_body.contains(capture.helper_name) {
        return;
    }
    let Some(rendered) = crate::e2e::codegen::mock_assertions::render_helper("csharp") else {
        return;
    };
    fixtures_body.push('\n');
    fixtures_body.push_str(&rendered);
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
