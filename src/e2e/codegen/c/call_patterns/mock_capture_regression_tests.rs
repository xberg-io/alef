//! Regression coverage for the two `mock.*` (alef issue #443) split-dispatch gaps unique to
//! `call_patterns.rs`: `render_engine_factory_test_function`'s `has_active_assertions` pre-check,
//! and `render_bytes_test_function`'s bespoke, `assertion.field`-blind assertion loop. Neither is
//! the streaming gap alef issue #443's own intel table names for `c` -- both were found by
//! auditing every `fixture.assertions` reference in this backend, per `assertion_mock_capture`'s
//! module doc.
//!
//! ~keep New submodule of `call_patterns` rather than growing `call_patterns.rs` itself (already
//! near the file-modularization cap once its own doc comments are counted) -- mirrors
//! `batch_url_regression_tests`'s own rationale for the same split.

use std::collections::{HashMap, HashSet};

use crate::e2e::codegen::c::assertions::{EffectiveConfigSource, FieldConfigSources};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

/// A resolver with a REAL, non-empty `result_fields` allow-list that does not name `mock`.
///
/// ~keep `is_valid_for_result` is permissive-by-default: with an EMPTY `result_fields` set it
/// returns `true` for any name at all, `mock.requests.total` included -- so a test built on an
/// empty-everything resolver cannot tell the `is_mock_virtual_field` fix apart from having no fix
/// at all; `is_valid_for_result` alone already says "active" either way. Caught empirically:
/// sabotaging the fix and rerunning `a_mock_only_fixture_still_takes_the_active_assertion_path`
/// against an all-empty resolver still passed. A non-empty, mock-excluding `result_fields` is what
/// makes `is_valid_for_result` actually answer `false` for `mock.requests.total`, which is the
/// realistic shape (`[e2e].result_fields` configured) `has_active_assertions`'s own fix exists for.
fn resolver_with_a_real_result_field() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::from(["status".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn global_sources() -> FieldConfigSources {
    FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::Global,
    }
}

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    }
}

/// Before the `is_mock_virtual_field` arm on `has_active_assertions`: a fixture whose ONLY
/// assertion is `mock.*` measured zero active assertions (`mock.requests.total` is never "valid
/// for result type"), took the soft-null-guard branch, and rendered NO assertions at all -- a
/// silent full drop, not even the `FieldSkip::NotAvailableOnResultType` misattribution
/// `ensure_capture_declared`'s own doc warns about. This test FAILS against that code: the
/// soft-guard branch never emits "expected call to succeed" and never calls
/// `render_assertion`/`try_render_mock_capture_assertion` at all.
#[test]
fn a_mock_only_fixture_still_takes_the_active_assertion_path() {
    let fixture = Fixture {
        id: "engine_factory_mock_capture".into(),
        description: "asserts against the mock server request log".into(),
        assertions: vec![mock_assertion("mock.requests.total", "equals", serde_json::json!(1))],
        ..Fixture::default()
    };
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "scrape",
        "result",
        &resolver_with_a_real_result_field(),
        &HashMap::new(),
        &HashSet::new(),
        "CrawlResult",
        "CrawlConfig",
        false,
        None,
        &[],
        &global_sources(),
    )
    .expect("engine-factory mock-only fixture renders");

    assert!(
        out.contains("assert(result != 0 && \"expected call to succeed\");"),
        "a mock.*-only fixture must take the active-assertion path, not the soft null guard:\n{out}"
    );
    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 1 && \"equals assertion failed\");"
        ),
        "expected the mock-count assertion to actually render:\n{out}"
    );
}

/// Control: a fixture with no assertions at all must still take the soft-null-guard branch --
/// the fix only widens what counts as "active", it does not make every fixture active.
#[test]
fn a_fixture_with_no_assertions_still_takes_the_soft_guard_path() {
    let fixture = Fixture {
        id: "engine_factory_no_assertions".into(),
        description: "no assertions at all".into(),
        assertions: vec![],
        ..Fixture::default()
    };
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "scrape",
        "result",
        &resolver_with_a_real_result_field(),
        &HashMap::new(),
        &HashSet::new(),
        "CrawlResult",
        "CrawlConfig",
        false,
        None,
        &[],
        &global_sources(),
    )
    .expect("engine-factory no-assertion fixture renders");

    assert!(
        !out.contains("expected call to succeed"),
        "a fixture with no assertions at all must still take the soft null-guard branch:\n{out}"
    );
    assert!(
        out.contains("if (result != 0) sample_crawl_result_free(result);"),
        "expected the soft null-guard free, got:\n{out}"
    );
}

/// Covers the field-EXTRACTION loop's own `is_mock_virtual_field` guard, a SEPARATE fix from
/// `has_active_assertions`'s above -- exercised here with an all-empty (permissive) resolver, the
/// case where `is_valid_for_result("mock.requests.total")` returns `true` (empty `result_fields`
/// default-allows everything), so the field reaches the extraction loop's own condition and only
/// `is_mock_virtual_field` keeps it out of `emit_nested_accessor`. Without that guard this fixture
/// bails generation entirely (no IR type named `CrawlResult` to walk "mock" against), well before
/// ever reaching a rendered assertion.
#[test]
fn a_mock_assertion_survives_extraction_even_when_is_valid_for_result_is_permissive() {
    let fixture = Fixture {
        id: "engine_factory_mock_capture_permissive".into(),
        description: "asserts against the mock server request log, no result_fields configured".into(),
        assertions: vec![mock_assertion("mock.requests.total", "equals", serde_json::json!(1))],
        ..Fixture::default()
    };
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "scrape",
        "result",
        &FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        ),
        &HashMap::new(),
        &HashSet::new(),
        "CrawlResult",
        "CrawlConfig",
        false,
        None,
        &[],
        &global_sources(),
    )
    .expect("engine-factory mock-only fixture renders even with a permissive resolver");

    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 1 && \"equals assertion failed\");"
        ),
        "expected the mock-count assertion to actually render:\n{out}"
    );
    assert!(
        !out.contains("_mock("),
        "must not have emitted a bogus accessor call for the 'mock' segment, got:\n{out}"
    );
}

/// Before intercepting `mock.*` first: `render_bytes_test_function`'s bespoke assertion loop
/// only understands `not_error`/`not_empty`/`not_null`; every other assertion type fell into
/// the catch-all "not meaningful on raw byte buffer" comment. This test FAILS against that code.
#[test]
fn a_mock_assertion_on_a_bytes_returning_call_renders_the_real_comparison() {
    let fixture = Fixture {
        id: "bytes_mock_capture".into(),
        description: "asserts against the mock server request log on a bytes-returning call".into(),
        assertions: vec![mock_assertion(
            "mock.requests.total",
            "greater_than_or_equal",
            serde_json::json!(1),
        )],
        ..Fixture::default()
    };
    let mut out = String::new();

    super::render_bytes_test_function(
        &mut out,
        &fixture,
        "sample",
        "speech",
        "result",
        &[],
        "",
        "SpeechResponse",
        "default_client_new",
        "Client",
        false,
        &[],
        false,
    );

    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") >= 1 && \"expected greater than or equal\");"
        ),
        "expected the real mock-count comparison, got:\n{out}"
    );
    assert!(
        !out.contains("not meaningful on raw byte buffer"),
        "a mock.* assertion must not be treated as an unsupported byte-buffer field, got:\n{out}"
    );
}
