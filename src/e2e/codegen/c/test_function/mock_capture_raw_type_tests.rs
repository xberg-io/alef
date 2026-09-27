//! Regression coverage for the "raw C result type" bespoke assertion loop's `mock.*` (alef issue
//! #443) split-dispatch gap: `render_test_function_impl`'s success path for a function returning
//! `char*`/`int32_t`/`uintptr_t` directly (not an opaque struct handle) has its OWN assertion
//! dispatcher keyed off the bare `result_var`, never `assertion.field`. Before intercepting
//! `mock.*` there first, an `equals` assertion on `mock.requests.total` would have matched the
//! `"equals"` arm and compared `result_var` against the literal `1`, compiling a silently WRONG
//! assertion rather than skipping or failing loudly -- worse than every other gap this backend
//! needed, which either dropped the assertion or refused to render.
//!
//! ~keep New submodule of `test_function` (`#[path = ...]`) rather than growing `test_function.rs`
//! itself, already over the file-modularization cap -- mirrors `c::call_patterns`'s own
//! `mock_capture_regression_tests` split for the identical reason.

use super::*;
use crate::e2e::codegen::c::ResultTypeName;
use crate::e2e::codegen::c::assertions::{EffectiveConfigSource, FieldConfigSources};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    }
}

/// Drives `render_test_function_impl` directly (the real production entry point, same call shape
/// `c.rs::raw_result_test_function_asserts_failure_per_result_type` already uses) with a
/// `raw_c_result_type` configured and a `mock.*` assertion -- proving the interception fires
/// before the bespoke `result_var`-keyed match, not that some OTHER branch happens to skip it.
#[test]
fn a_mock_assertion_on_a_raw_result_type_call_renders_the_real_comparison_not_a_result_var_compare() {
    let fixture = Fixture {
        id: "raw_type_mock_capture".into(),
        description: "asserts against the mock server request log on a raw-result-type call".into(),
        assertions: vec![mock_assertion("mock.requests.total", "equals", serde_json::json!(3))],
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    let mut out = String::new();
    render_test_function_impl(
        &mut out,
        &fixture,
        "sample",
        "sample_parse_input",
        "result",
        &[],
        &field_resolver,
        &HashMap::new(),
        &HashSet::new(),
        &ResultTypeName::Resolved("Result".into()),
        "",
        None,
        Some("char*"),
        None,
        None,
        false,
        false,
        None,
        &[],
        &config,
        &[],
        &[],
        false,
        &FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::Global,
        },
        TargetParams::IrAbsent,
    )
    .expect("raw-result-type mock-capture fixture renders");

    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 3 && \"equals assertion failed\");"
        ),
        "expected the real mock-count comparison, got:\n{out}"
    );
    assert!(
        !out.contains("assert(result == 3"),
        "must not have compared the raw scalar result against the mock-count literal, got:\n{out}"
    );
}
