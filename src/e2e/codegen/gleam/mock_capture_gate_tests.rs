//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`out[fixtures_start..].contains(capture.helper_name)` in
//! `test_file::render_test_file`) are tested separately elsewhere in this codebase -- these tests
//! drive the real `render_test_file` entry point so a fixture's call and the file's helper
//! definition land in ONE rendered artifact, the only way to catch the two disagreeing about
//! `capture.helper_name`. Mirrors `java::mock_capture_gate_tests` / `kotlin::mock_capture_gate_tests`.

use super::test_file::render_test_file;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render_gleam_with_call(assertions: Vec<Assertion>, call: CallConfig) -> String {
    let fixture = Fixture {
        id: "mock_capture_fixture".to_string(),
        description: "asserts against the mock server request log".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..E2eConfig::default()
    };
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_file(
        "mock_capture",
        &[&fixture],
        &e2e_config,
        "sample_crate",
        "process",
        "result",
        &[],
        &[],
        None,
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
        &[],
    )
}

fn render_gleam(assertions: Vec<Assertion>) -> String {
    render_gleam_with_call(
        assertions,
        CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
    )
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same fixture -- cheap here since both share one rendered file.
#[test]
fn a_mock_assertion_gets_both_its_call_and_the_helper_definition_in_one_gleam_file() {
    let out = render_gleam(vec![
        mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
        mock_assertion(
            r#"mock.requests["POST /v1/chat"]"#,
            "greater_than_or_equal",
            serde_json::json!(1),
        ),
    ]);

    assert!(
        out.contains("alef_mock_request_count(mock_base_url__, \"/__alef/requests/total?prefix=\") |> should.equal(1)"),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "alef_mock_request_count(mock_base_url__, \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") |> fn(n__) { n__ >= 1 } |> should.equal(True)"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("fn alef_mock_request_count(base_url: String, path_and_query: String) -> Int"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
    assert!(
        out.contains("import gleam/httpc")
            && out.contains("import gleam/http/request")
            && out.contains("import gleam/int"),
        "expected the helper's imports to be present, got:\n{out}"
    );
}

/// `result_is_simple` gates every OTHER field-access assertion behind
/// `FieldSkip::NotAccessibleOnSimpleResultType` in `test_case.rs`, before `render_assertion` (and
/// therefore this backend's mock-capture interception) ever runs. `mock.*` is not a field of the
/// result type at all -- it must still resolve against the mock server helper even when the call's
/// result is "simple", so this fixture pins that `test_case.rs`'s own gate exempts it rather than
/// swallowing it as a false `AuthoringGap`. This is the exact "early return upstream of the
/// interception" shape a naive `render_assertion`-only fix would have missed.
#[test]
fn a_mock_assertion_still_resolves_when_the_result_is_simple() {
    let out = render_gleam_with_call(
        vec![mock_assertion("mock.requests.total", "equals", serde_json::json!(2))],
        CallConfig {
            function: "process".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_is_simple: true,
            ..CallConfig::default()
        },
    );
    assert!(
        out.contains("alef_mock_request_count(mock_base_url__, \"/__alef/requests/total?prefix=\") |> should.equal(2)"),
        "a result_is_simple fixture must still get the mock-capture call, got:\n{out}"
    );
    assert!(
        !out.contains("not accessible on"),
        "a mock.* field must never be misclassified as NotAccessibleOnSimpleResultType, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper,
/// and must not carry the helper-only imports either.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let out = render_gleam(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);
    assert!(
        !out.contains("alef_mock_request_count"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
    assert!(
        !out.contains("import gleam/httpc"),
        "a file with no mock.* assertion must not import the helper's httpc dependency, got:\n{out}"
    );
}
