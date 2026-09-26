//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertions::mock_capture::render`) and the helper-definition gate
//! (`super::snippet::references_identifier(&fixtures_body, capture.helper_name)` in
//! `render_test_file`) are tested SEPARATELY everywhere else in this codebase -- this drives the
//! real `render_test_file` entry point so a fixture's call and the file's helper definition land
//! in ONE rendered artifact, the only way to catch the two disagreeing about
//! `capture.helper_name`. `mock_capture(lang)` only has a `"node"` entry (`"wasm"` is a deliberate
//! gap -- see `mock_capture.rs`), so every case here renders with `lang = "node"`. ~keep

use super::*;

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn fixture(assertions: Vec<crate::e2e::fixture::Assertion>) -> Fixture {
    Fixture {
        id: "mock_capture_fixture".to_string(),
        category: Some("mock_capture".to_string()),
        description: "asserts against the mock server request log".to_string(),
        assertions,
        ..Default::default()
    }
}

fn render(fixture: &Fixture) -> String {
    let e2e_config = E2eConfig::default();
    render_test_file(
        "node",
        "mock_capture",
        &[fixture],
        "",
        "sample-bindings",
        "chat",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
    )
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same fixture -- cheap here since both share one rendered file. ~keep
#[test]
fn a_mock_assertion_gets_both_its_call_and_the_helper_definition_in_one_file() {
    let fixture = fixture(vec![
        mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
        mock_assertion(
            r#"mock.requests["POST /v1/chat"]"#,
            "greater_than_or_equal",
            serde_json::json!(1),
        ),
    ]);
    let out = render(&fixture);

    assert!(
        out.contains(
            "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/total?prefix=\")).toBe(1);"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\")).toBeGreaterThanOrEqual(1);"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("async function alefMockRequestCount(baseUrl: string, pathAndQuery: string): Promise<number> {"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let fixture = fixture(vec![crate::e2e::fixture::Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);
    let out = render(&fixture);
    assert!(
        !out.contains("alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
