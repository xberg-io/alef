//! `mock.*` request-count virtual field assertion rendering for Swift e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `kotlin::assertion_mock_capture`: `mock.*` fields (see
//! `mock_assertions::model`) resolve against the generated mock server's own request log through
//! the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `swift::test_file::render_test_file`), not a struct field, so interception happens before every
//! other branch in `assertions::render_assertion` -- including the streaming-virtual-field
//! interception at the very TOP of that function, which resolves against the collected `chunks`
//! array. A streaming fixture may still assert a `mock.*` count against the mock server's own
//! request log rather than the stream's collected chunks, so `mock.*` must win even there; without
//! this ordering a `mock.*` assertion on a streaming fixture would be misrouted into the streaming
//! resolver, which has no entry for it and would fall through to a skip marker. The helper is a
//! private INSTANCE method of the generated `XCTestCase` subclass (deliberately not `static` --
//! see `swift/mock_request_helper.swift.jinja`'s doc: Swift, unlike Java/Kotlin, refuses an
//! unqualified call to a `static` member from instance context), so every call site here still
//! reads a bare (unqualified) call. ~keep
//!
//! The helper is `throws` (see `swift/mock_request_helper.swift.jinja`'s doc for why), so every
//! call site emitted here is prefixed with `try` -- `XCTAssertEqual`/`XCTAssertGreaterThan`/etc.
//! all accept `@autoclosure() throws -> T` arguments, so a `try` expression nested inside one of
//! those calls type-checks and, on failure, fails only that one test rather than aborting the run.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

use super::values::escape_swift;

/// Swift expression for the mock server's base URL -- the same accessor every per-fixture mock-URL
/// call site in this backend (`args.rs`, `test_method.rs`) already reads, and the one `swift/
/// project.rs` emits unconditionally into every generated test target.
const MOCK_SERVER_URL_EXPR: &str = "AlefE2EMockServer.baseURL";

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
    let capture = mock_capture("swift").unwrap_or_else(|_| {
        panic!(
            "Swift e2e generator: mock.* field '{field}' has no swift capture-table entry (mock_assertions::snippets)"
        )
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!(
        "try {}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")",
        capture.helper_name
    );
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `Int`) into the same `XCTAssert*` family every other
/// numeric comparison in this backend already uses.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Swift e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Swift e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion message below, inside a Swift string literal --
    // `mock.requests["POST /v1/chat"]` carries literal `"` characters that must be escaped there.
    let escaped_field = escape_swift(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(
                out,
                "        XCTAssertEqual({expr}, {n}, \"expected {escaped_field} == {n}\")"
            );
        }
        "not_equals" => {
            let _ = writeln!(
                out,
                "        XCTAssertNotEqual({expr}, {n}, \"expected {escaped_field} != {n}\")"
            );
        }
        "greater_than" => {
            let _ = writeln!(
                out,
                "        XCTAssertGreaterThan({expr}, {n}, \"expected {escaped_field} > {n}\")"
            );
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "        XCTAssertGreaterThanOrEqual({expr}, {n}, \"expected {escaped_field} >= {n}\")"
            );
        }
        "less_than" => {
            let _ = writeln!(
                out,
                "        XCTAssertLessThan({expr}, {n}, \"expected {escaped_field} < {n}\")"
            );
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "        XCTAssertLessThanOrEqual({expr}, {n}, \"expected {escaped_field} <= {n}\")"
            );
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
