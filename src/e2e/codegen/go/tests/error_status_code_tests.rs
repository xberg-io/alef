//! The `error.status_code` assertion of an error fixture, driven through the whole-file
//! rendering entry point, which reaches the per-fixture call site that emits the check.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ErrorDef;
use crate::e2e::codegen::error_field_checks::test_fixtures::{
    error_with_methods, status_code_errors, status_code_fixture,
};
use crate::e2e::codegen::go::{GoTestFileContext, render_test_file};
use crate::e2e::config::{CallConfig, E2eConfig};

fn render_status_fixture(errors: &[ErrorDef]) -> String {
    let mut fixture = status_code_fixture(401);
    fixture.mock_response = Some(crate::e2e::fixture::MockResponse {
        status: 401,
        body: Some(serde_json::Value::Null),
        stream_chunks: None,
        headers: std::collections::BTreeMap::new(),
        delay_ms: None,
        stream_chunk_delay_ms: None,
    });
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat".to_string(),
            module: "github.com/example/sample".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_file(
        "auth",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/sample",
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &[],
            enums: &[],
            errors,
            functions: &[],
            crate_facts: None,
        },
    )
}

/// The generated `*Error` carries `StatusCode`, so the returned error is unwrapped to it and
/// compared with the fixture value instead of being named as skipped.
#[test]
fn an_error_status_code_assertion_reads_the_field_of_the_typed_error() {
    let out = render_status_fixture(&status_code_errors());

    assert!(
        out.contains("expected an error, but call succeeded"),
        "the error block must render: {out}"
    );
    assert!(out.contains("var fieldError *pkg.ApiError"), "{out}");
    assert!(out.contains("fieldError.StatusCode != 401"), "{out}");
    assert!(!out.contains("has no accessor for error field"), "{out}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

/// An error type without a `status_code` method has nothing to read, so the skip stays named.
#[test]
fn an_error_status_code_assertion_stays_skipped_without_the_method() {
    let out = render_status_fixture(&error_with_methods("ApiError", &[]));

    assert!(!out.contains("StatusCode"), "{out}");
    assert!(
        out.contains("has no accessor for error field error.status_code"),
        "{out}"
    );
}
