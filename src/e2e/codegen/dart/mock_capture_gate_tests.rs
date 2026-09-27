//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`fixture_bodies.contains(capture.helper_name)` in
//! `test_file::render_test_file`) are tested SEPARATELY everywhere else in this codebase -- these
//! tests drive the real `render_test_file` entry point so a fixture's call and the file's helper
//! definition land in ONE rendered artifact, the only way to catch the two disagreeing about
//! `capture.helper_name`. Mirrors `java::mock_capture_gate_tests` / `ruby::mock_capture_gate_tests`.
//! ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

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
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "chat".to_string();
    let config = ResolvedCrateConfig::default();
    let dart_first_class_map = crate::e2e::field_access::DartFirstClassMap::default();

    super::test_file::render_test_file(
        "mock_capture",
        &[&fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &dart_first_class_map,
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
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
            "    expect(await _alefMockRequestCount(_sutUrl(), \"/__alef/requests/total?prefix=\"), equals(1), reason: 'expected mock.requests.total == 1');"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "    expect(await _alefMockRequestCount(_sutUrl(), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\"), greaterThanOrEqualTo(1), reason: 'expected mock.requests[\"POST /v1/chat\"] >= 1');"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("Future<int> _alefMockRequestCount(String baseUrl, String pathAndQuery) async {"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
    // The helper must be declared at module level, before `void main() {` opens -- Dart forbids
    // top-level function definitions inside another function body.
    let helper_pos = out
        .find("Future<int> _alefMockRequestCount")
        .expect("helper definition must be present");
    let main_pos = out.find("void main() {").expect("void main() must be present");
    assert!(
        helper_pos < main_pos,
        "helper must be declared before void main(), got:\n{out}"
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
        !out.contains("_alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
