//! The `error.status_code` assertion of an error fixture, driven through the real
//! `render_test_function_impl` entry point.

use super::*;
use crate::e2e::codegen::error_field_checks::test_fixtures::{
    error_with_methods, status_code_errors, status_code_fixture,
};

fn render_status_fixture(errors: &[crate::core::ir::ErrorDef]) -> String {
    let fixture = status_code_fixture(401);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_function_impl(
        &mut out,
        &fixture,
        "sample",
        "parse",
        "result",
        &[],
        &resolver,
        &HashMap::new(),
        &HashSet::new(),
        &ResultTypeName::Resolved("ParseResult".into()),
        "",
        None,
        None,
        None,
        None,
        false,
        false,
        None,
        &[],
        &config,
        &[],
        errors,
        false,
        &FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::Global,
        },
        TargetParams::IrAbsent,
    )
    .expect("error test function renders");
    out
}

/// The ABI records the HTTP status beside the message, so the assertion reads it through the
/// `_last_error_status_code` accessor instead of being named as skipped.
#[test]
fn an_error_status_code_assertion_reads_the_last_error_status() {
    let out = render_status_fixture(&status_code_errors());

    assert!(
        out.contains("assert(sample_last_error_status_code() == 401 && \"error status_code mismatch\");"),
        "the status must be compared with the fixture value, got:\n{out}"
    );
    assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

/// An error type without a `status_code` method has nothing to read, so the skip stays named.
#[test]
fn an_error_status_code_assertion_stays_skipped_without_the_method() {
    let out = render_status_fixture(&error_with_methods("ApiError", &[]));

    assert!(!out.contains("sample_last_error_status_code"), "got:\n{out}");
    assert!(
        out.contains("has no accessor for error field error.status_code"),
        "got:\n{out}"
    );
}
