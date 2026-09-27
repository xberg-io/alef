//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`fixtures_body.contains(capture.helper_name)` in
//! `test_file::render_test_file`) are tested SEPARATELY everywhere else in this codebase --
//! these tests drive the real `render_test_file` entry point so a fixture's call and the file's
//! helper definition land in ONE rendered artifact, the only way to catch the two disagreeing
//! about `capture.helper_name`. Mirrors `python::mock_capture_gate_tests` /
//! `rust::test_file::tests::mock_capture_gate`. ~keep

use super::test_file::render_test_file;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render(assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "mock_capture_fixture".to_string(),
        description: "asserts against the mock server request log".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call: CallConfig::default(),
        ..E2eConfig::default()
    };

    render_test_file(
        "mock_capture",
        &[&fixture],
        "TestClass",
        "testFunc",
        "com.example",
        "com.example.binding",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &[],
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
        &[],
        false,
    )
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same fixture -- cheap here since both share one rendered file. ~keep
#[test]
fn a_mock_assertion_gets_both_its_call_and_the_helper_definition_in_one_file() {
    let out = render(vec![
        mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
        mock_assertion(
            r#"mock.requests["POST /v1/chat"]"#,
            "greater_than_or_equal",
            serde_json::json!(1),
        ),
    ]);

    assert!(
        out.contains(
            "assertEquals(1, alefMockRequestCount(System.getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\"), \"expected mock.requests.total == 1\");"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "assertTrue(alefMockRequestCount(System.getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 1, \"expected mock.requests[\\\"POST /v1/chat\\\"] >= 1\");"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("private static int alefMockRequestCount(String baseUrl, String pathAndQuery)"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let out = render(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);
    assert!(
        !out.contains("alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
