//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`body.contains(capture.helper_name)` in `render_test_file`) are
//! tested SEPARATELY everywhere else in this codebase -- these tests drive the real
//! `render_test_file` entry point so a fixture's call and the file's helper definition land in
//! ONE rendered artifact, the only way to catch the two disagreeing about `capture.helper_name`.
//! Mirrors `python::mock_capture_gate_tests` / `rust::test_file::tests::mock_capture_gate`. ~keep

use super::test_file::{GoTestFileContext, render_test_file};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// `returns_result: true, result_is_simple: true` -- the same shape
/// `import_pruning_tests::e2e_config_for_describe` uses to reach real assertion rendering:
/// `returns_result: false` (what `python`/`rust`'s own gate-test fixtures use) hits Go's
/// void-call early return (`test_function.rs`'s `!effective_returns_result || returns_void`
/// branch), which emits the call and NOTHING else -- not even a `not_error` check -- so no
/// assertion, mock or otherwise, would ever reach `render_assertion`. ~keep
fn e2e_config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "chat".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_is_simple: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
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

fn render(assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "mock_capture_fixture".to_string(),
        description: "asserts against the mock server request log".to_string(),
        input: serde_json::Value::Null,
        assertions,
        ..Fixture::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let e2e_config = e2e_config();

    render_test_file(
        "mock_capture",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/mylib",
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &errors,
            functions: &[],
            crate_facts: None,
        },
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
            "\t\tv := alefMockRequestCount(os.Getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\")\n\t\tif v != 1 {"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "\t\tv := alefMockRequestCount(os.Getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\")\n\t\tif v < 1 {"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("func alefMockRequestCount(baseURL, pathAndQuery string) int {"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
    // The helper calls `os.Getenv`, `http.Get`, `io.ReadAll` and `strconv.Atoi` -- every one of
    // those imports must be present, or `go build` fails on this exact file. ~keep
    for import in ["\"os\"", "\"net/http\"", "\"io\"", "\"strconv\""] {
        assert!(out.contains(import), "expected the {import} import, got:\n{out}");
    }
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference or define the helper,
/// and must not pull in imports the helper alone would need.
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
    assert!(
        !out.contains("\"strconv\""),
        "strconv has no other caller in this backend, got:\n{out}"
    );
}
