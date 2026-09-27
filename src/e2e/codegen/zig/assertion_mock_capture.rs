//! `mock.*` request-count virtual field assertion rendering for Zig e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `ruby::assertion_mock_capture`: `mock.*` fields (see
//! `mock_assertions::model`) resolve against the generated mock server's own request log through
//! the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `zig::test_file::render_test_file`), not a struct field or JSON key.
//!
//! ~keep Unlike every backend before this one in the #443 wave, Zig has TWO independent
//! assertion-rendering entry points, not one: `assertions::render_json_assertion` (the
//! JSON-struct and streaming result paths) and `assertions::render_assertion` (the typed/simple
//! result path). `zig::visitor::emit_visitor_test_body` calls `render_json_assertion` directly,
//! so it needs no separate interception -- but both `render_json_assertion` and
//! `render_assertion` themselves must call `try_render_mock_capture_assertion` first, before
//! every other branch (including `result_is_option`/`result_is_simple`/the various synthetic-
//! field special cases), or a `mock.*` fixture routed through whichever function is NOT
//! intercepted silently falls through to the ordinary field oracle -- exactly the
//! wrong-backend-wins-the-guard trap `mock_assertions::ensure_capture_declared` exists to
//! prevent.
//!
//! The emitted Zig helper manages its own `std.heap.c_allocator` internally (see
//! `zig/mock_request_helper.zig.jinja`) rather than taking an allocator parameter, and panics
//! instead of returning an error union, so the call site here stays the same two-argument shape
//! (`base_url`, `path_and_query`) every other backend's call site uses, with no `try` and no
//! GPA/allocator threading required.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// Zig expression for the mock server's base URL -- read the same way `http.rs`'s `render_call`
/// already reads it for every other per-fixture HTTP call in this backend
/// (`std.c.getenv("MOCK_SERVER_URL")`), with the identical localhost fallback.
const MOCK_SERVER_URL_EXPR: &str =
    "if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"";

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
    let capture = mock_capture("zig").unwrap_or_else(|_| {
        panic!("Zig e2e generator: mock.* field '{field}' has no zig capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (a `u64`) into the same `std.testing` family every other
/// numeric comparison in this backend already uses.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Zig e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Zig e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    try testing.expectEqual(@as(u64, {n}), {expr});");
        }
        "not_equals" => {
            let _ = writeln!(out, "    try testing.expect({expr} != @as(u64, {n}));");
        }
        "greater_than" => {
            let _ = writeln!(out, "    try testing.expect({expr} > @as(u64, {n}));");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "    try testing.expect({expr} >= @as(u64, {n}));");
        }
        "less_than" => {
            let _ = writeln!(out, "    try testing.expect({expr} < @as(u64, {n}));");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "    try testing.expect({expr} <= @as(u64, {n}));");
        }
        other => {
            panic!("Zig e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
