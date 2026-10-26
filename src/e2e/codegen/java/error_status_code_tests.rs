//! The `error.status_code` assertion of an error fixture, driven through the real
//! `render_test_method` entry point.

use super::test_method::render_test_method;
use crate::e2e::codegen::error_field_checks::test_fixtures::{status_code_errors, status_code_fixture};
use crate::e2e::config::{CallConfig, E2eConfig};

fn render_status_fixture(errors: &[crate::core::ir::ErrorDef]) -> String {
    let fixture = status_code_fixture(401);
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "process".to_string(),
            module: "MyLib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let mut out = String::new();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_method(
        &mut out,
        &fixture,
        "SampleClass",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &[],
        &config,
        &[],
        &[],
        &[],
        errors,
    );
    out
}

/// The generated base exception carries the status getter, so the thrown exception is cast to it
/// and compared with the fixture value instead of being named as skipped.
#[test]
fn an_error_status_code_assertion_reads_the_getter_of_the_thrown_exception() {
    let out = render_status_fixture(&status_code_errors());

    assert!(out.contains("assertThrows("), "the error block must render:\n{out}");
    assert!(
        out.contains("assertEquals(401, ((") && out.contains("ApiErrorException) thrown).getStatusCode()"),
        "the status must be compared with the fixture value, got:\n{out}"
    );
    assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

/// An error type without a `status_code` method has nothing to read, so the skip stays named.
#[test]
fn an_error_status_code_assertion_stays_skipped_without_the_method() {
    let errors = crate::e2e::codegen::error_field_checks::test_fixtures::error_with_methods("ApiError", &[]);
    let out = render_status_fixture(&errors);

    assert!(!out.contains("getStatusCode()"), "got:\n{out}");
    assert!(
        out.contains("has no accessor for error field error.status_code"),
        "got:\n{out}"
    );
}
