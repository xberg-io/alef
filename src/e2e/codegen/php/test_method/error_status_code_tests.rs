//! The `error.status_code` assertion of an error fixture, driven through the real
//! `render_test_method` entry point.

use super::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::error_field_checks::test_fixtures::{
    error_with_methods, status_code_errors, status_code_fixture,
};
use crate::e2e::config::E2eConfig;
use std::collections::HashMap;

fn render_status_fixture(errors: &[crate::core::ir::ErrorDef]) -> String {
    let fixture = status_code_fixture(401);
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".into();
    e2e_config.call.result_var = "result".into();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let mut trait_bridge_imports: Vec<String> = Vec::new();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "php",
        "Sample",
        "SampleClient",
        &[],
        &[],
        &[],
        &super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]),
        &HashMap::new(),
        false,
        None,
        "",
        &[],
        "",
        &config,
        errors,
        &mut trait_bridge_imports,
    );
    out
}

/// The thrown exception carries the status through a camelCase accessor, so the assertion reads
/// it instead of being named as skipped.
#[test]
fn an_error_status_code_assertion_reads_the_accessor_of_the_exception() {
    let out = render_status_fixture(&status_code_errors());

    assert!(
        out.contains("$this->assertSame(401, $caughtException->statusCode(), 'expected status_code 401');"),
        "the status must be compared with the fixture value, got:\n{out}"
    );
    assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

/// An error type without a `status_code` method has nothing to read, so the skip stays named.
#[test]
fn an_error_status_code_assertion_stays_skipped_without_the_method() {
    let out = render_status_fixture(&error_with_methods("ApiError", &[]));

    assert!(!out.contains("statusCode()"), "got:\n{out}");
    assert!(
        out.contains("has no accessor for error field error.status_code"),
        "got:\n{out}"
    );
}
