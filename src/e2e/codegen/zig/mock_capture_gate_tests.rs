//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`out.contains(capture.helper_name)` in `test_file::render_test_file`)
//! are tested SEPARATELY everywhere else in this codebase -- these tests drive the real
//! `render_test_file` entry point so a fixture's call and the file's helper definition land in
//! ONE rendered artifact, the only way to catch the two disagreeing about `capture.helper_name`.
//! Mirrors `java::mock_capture_gate_tests` / `ruby::mock_capture_gate_tests`.
//!
//! The third test below is Zig-specific: unlike every other backend in this wave, Zig gates
//! whether a fixture's assertions get rendered AT ALL on `assertion_emits_code` (to avoid
//! binding an unused `result`/`allocator` local, which Zig 0.16 rejects) -- and a `mock.*`
//! field is never `is_valid_for_result`, so a fixture whose ONLY assertion is `mock.*` used to
//! be silently dropped by that same gate before `test_file::render_test_fn`'s
//! `has_mock_assertions` fix landed alongside it. ~keep

use std::collections::HashSet;

use super::test_file::render_test_file;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::call_ir::CallIr;
use crate::e2e::config::{CallOverride, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render(fixture: Fixture, e2e_config: &E2eConfig) -> String {
    render_test_file(
        "mock_capture",
        &[&fixture],
        e2e_config,
        "chat",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        CallIr::default(),
        &[],
    )
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same fixture -- cheap here since both share one rendered file. ~keep
#[test]
fn a_mock_assertion_gets_both_its_call_and_the_helper_definition_in_one_file() {
    let fixture = Fixture {
        id: "mock_capture_fixture".to_string(),
        description: "asserts against the mock server request log".to_string(),
        assertions: vec![
            mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
            mock_assertion(
                r#"mock.requests["POST /v1/chat"]"#,
                "greater_than_or_equal",
                serde_json::json!(1),
            ),
        ],
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chat".to_string();

    let out = render(fixture, &e2e_config);

    assert!(
        out.contains(
            "    try testing.expectEqual(@as(u64, 1), alefMockRequestCount(if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\", \"/__alef/requests/total?prefix=\"));"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "    try testing.expect(alefMockRequestCount(if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\", \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= @as(u64, 1));"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("fn alefMockRequestCount(baseUrl: []const u8, pathAndQuery: []const u8) u64 {"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let fixture = Fixture {
        id: "no_mock".to_string(),
        description: "no mock assertions at all".to_string(),
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chat".to_string();

    let out = render(fixture, &e2e_config);

    assert!(
        !out.contains("alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}

/// The Zig-specific defect this wave fixes: on the JSON-struct result path, a fixture whose ONLY
/// assertion is `mock.*` must still render its call. `result_fields` is deliberately non-empty
/// and excludes "mock" so `is_valid_for_result` -- permissive when `result_fields` is EMPTY --
/// actually returns `false` for the mock field, reproducing the real-world config shape where
/// `assertion_emits_code` (which does not know about `mock.*`) would otherwise report no
/// assertion emits code, dropping the loop that calls `render_json_assertion` entirely. The
/// fixture must ALSO come out with no unused JSON-parse/GPA locals, since a mock-only assertion
/// never references `result`. ~keep
#[test]
fn a_mock_only_assertion_on_the_json_struct_path_still_renders_and_binds_nothing_unused() {
    let fixture = Fixture {
        id: "mock_only_json_struct".to_string(),
        description: "asserts only against the mock server request log".to_string(),
        assertions: vec![mock_assertion("mock.requests.total", "count_min", serde_json::json!(1))],
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chat".to_string();
    e2e_config.result_fields = HashSet::from(["summary".to_string()]);
    e2e_config.call.overrides.insert(
        "zig".to_string(),
        CallOverride {
            result_is_json_struct: true,
            ..CallOverride::default()
        },
    );

    let out = render(fixture, &e2e_config);

    assert!(
        out.contains(">= @as(u64, 1)"),
        "expected the mock assertion to render even though it is the fixture's only assertion, got:\n{out}"
    );
    assert!(
        !out.contains("_parsed"),
        "a mock-only fixture must not bind the unused JSON-parse locals, got:\n{out}"
    );
    assert!(
        !out.contains("std.heap.DebugAllocator"),
        "a mock-only fixture must not bind the unused GPA allocator, got:\n{out}"
    );
}

/// A second Zig-specific defect this wave fixes: the bytes-result path
/// (`call_result_is_bytes && client_factory.is_some()`, e.g. an audio/file-bytes client-factory
/// method) inline-matches `assertion_type` itself rather than delegating to
/// `render_json_assertion`/`render_assertion` -- a THIRD assertion-rendering site this backend's
/// split-dispatch audit missed on the first pass. Before its own interception, a `mock.*`
/// assertion here fell through to the bytes match's `_ =>` arm, rendering only a
/// "not implemented for zig bytes" comment -- and when `mock.*` was the fixture's ONLY
/// assertion, `has_bytes_assertions` was false too, so the loop never even ran. ~keep
#[test]
fn a_mock_only_assertion_on_the_bytes_result_path_still_renders() {
    let fixture = Fixture {
        id: "mock_only_bytes".to_string(),
        description: "asserts only against the mock server request log".to_string(),
        assertions: vec![mock_assertion("mock.requests.total", "count_min", serde_json::json!(1))],
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "speech".to_string();
    e2e_config.call.overrides.insert(
        "zig".to_string(),
        CallOverride {
            result_is_bytes: true,
            client_factory: Some("Client.create".to_string()),
            ..CallOverride::default()
        },
    );

    let out = render(fixture, &e2e_config);

    assert!(
        out.contains(">= @as(u64, 1)"),
        "expected the mock assertion to render on the bytes-result path, got:\n{out}"
    );
    assert!(
        !out.contains("not implemented for zig bytes"),
        "must not fall through to the bytes-assertion catch-all, got:\n{out}"
    );
}
