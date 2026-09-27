//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`assertion_mock_capture::append_helper_if_referenced`) are tested
//! SEPARATELY in `assertion_mock_capture/tests.rs` -- these tests drive the real
//! `render_test_file` entry point so a fixture's call and the file's helper definition land in
//! ONE rendered artifact, the only way to catch the two disagreeing about `capture.helper_name`.
//! Mirrors `java::mock_capture_gate_tests` / `python::mock_capture_gate_tests`. ~keep

use super::*;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

fn minimal_fixture(id: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "mock capture smoke test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render(fixture: &Fixture) -> String {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "SampleCall".to_string(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
        &std::collections::HashSet::new(),
    );
    let config = ResolvedCrateConfig::default();
    let empty_enum_fields: HashMap<String, String> = HashMap::new();
    let empty_nested_types: HashMap<String, String> = HashMap::new();
    let fixtures: Vec<&Fixture> = vec![fixture];

    render_test_file(
        "mock_capture",
        &fixtures,
        "Example.Tests",
        "ExampleLib",
        "SampleCall",
        "ExampleException",
        "result",
        "MockCaptureTests",
        &e2e_config.call.args,
        &field_resolver,
        false,
        false,
        &e2e_config,
        &empty_enum_fields,
        &empty_enum_fields,
        &empty_nested_types,
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
    let fixture = minimal_fixture(
        "mock_capture_fixture",
        vec![
            mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
            mock_assertion(
                r#"mock.requests["POST /v1/chat"]"#,
                "greater_than_or_equal",
                serde_json::json!(1),
            ),
        ],
    );
    let out = render(&fixture);

    assert!(
        out.contains(
            "Assert.Equal(1, alefMockRequestCount(Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\", \"/__alef/requests/total?prefix=\"));"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "Assert.True(alefMockRequestCount(Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? \"http://localhost:8080\", \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 1);"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("private static int alefMockRequestCount(string baseUrl, string pathAndQuery)"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let fixture = minimal_fixture(
        "mock_capture_absent",
        vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
    );
    let out = render(&fixture);
    assert!(
        !out.contains("alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
