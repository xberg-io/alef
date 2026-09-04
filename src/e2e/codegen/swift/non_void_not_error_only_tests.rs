//! Regression coverage for the non-void `not_error`-only vacuous-test defect.
//!
//! ~keep A non-void call whose ONLY declared assertion is `not_error` bound `result` but never
//! asserted anything real against it: `render_not_error_assertion` (see its doc) deliberately
//! renders just a comment for a non-void call, since an `XCTAssertNotNil` there would be
//! tautological (Swift auto-promotes the declared-non-optional return type to `Optional` at the
//! call site, so the assertion could never fail). `body_buffer` therefore held only that comment,
//! `inert_example::inert_verdict` found no executable line, and the whole example was refused as
//! `RenderedNothing` — dropped from the generated suite entirely, even though the call genuinely
//! throwing on failure IS the check the fixture asked for (`list_validators`, `format_pptx`, and
//! 17 siblings in one consumer's suite). The fix reuses `test_method.rs`'s existing
//! `void_not_error` machinery — the same `XCTAssertNoThrow`/do-catch templates — gated by the new
//! `non_void_not_error_only` flag, which fires only when every declared assertion is `not_error`.
//!
//! Lives in its own file rather than growing `test_method.rs`: that file is close to the repo's
//! 1,000-line cap (see `file-modularization` in CLAUDE.md), matching the precedent set by
//! `void_not_error_call_tests.rs` itself.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::inert_example::take_inert_examples;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::SwiftFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Assertion::default()
    }
}

fn field_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!("x")),
        ..Assertion::default()
    }
}

fn render_call(is_async: bool, assertions: Vec<Assertion>) -> String {
    let call_config = CallConfig {
        function: "list_validators".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        r#async: is_async,
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call: call_config,
        // Only needed by the control test below (an assertion targeting `content` must resolve
        // rather than being excluded), but harmless for the not_error-only fixtures too.
        result_fields: std::collections::HashSet::from(["content".to_string()]),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let fixture = Fixture {
        id: "list_validators".to_string(),
        description: "List validators".to_string(),
        assertions,
        ..Fixture::default()
    };

    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "",
        "",
        &[],
        false,
        None,
        &SwiftFirstClassMap::default(),
        "Sample",
        &config,
        &[],
        &[],
        &[],
        &[],
    );
    out
}

/// The regression this file exists for, synchronous case: before the fix, a non-void
/// `not_error`-only fixture rendered `try XCTSkipIf(true, ...)` and was refused entirely — the
/// fixture never shipped as a test at all.
#[test]
fn non_void_not_error_only_sync_wraps_the_call_in_xctassert_no_throw() {
    let _ = take_inert_examples();

    let out = render_call(false, vec![not_error_assertion()]);

    assert!(
        out.contains("XCTAssertNoThrow(try Sample.listValidators())"),
        "expected the non-void call wrapped in XCTAssertNoThrow, got:\n{out}"
    );
    assert!(
        !out.contains("XCTSkipIf"),
        "must not fall back to an unconditional skip, got:\n{out}"
    );
    assert!(
        !out.contains("let result ="),
        "a not_error-only fixture has no follow-on assertion to use `result`, so it must not be \
         bound, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "the example must now be published, not refused as inert"
    );
}

/// Async case: mirrors `void_not_error`'s async branch — `XCTAssertNoThrow` has no async-aware
/// overload, so the call is wrapped in a do/catch that fails the test via `XCTFail`.
#[test]
fn non_void_not_error_only_async_wraps_the_call_in_a_do_catch_that_fails_on_error() {
    let _ = take_inert_examples();

    let out = render_call(true, vec![not_error_assertion()]);

    assert!(out.contains("do {"), "expected a do/catch wrapping the async call, got:\n{out}");
    assert!(
        out.contains("try await Sample.listValidators()"),
        "expected the async call inside the do block, got:\n{out}"
    );
    assert!(out.contains("XCTFail("), "expected the catch block to fail the test, got:\n{out}");
    assert!(
        !out.contains("XCTSkipIf"),
        "must not fall back to an unconditional skip, got:\n{out}"
    );
    assert!(take_inert_examples().is_empty());
}

/// CONTROL, asserted first per the fix's own constraint: a fixture that pairs `not_error` with a
/// real field assertion must keep binding `result` exactly as before — the field assertion still
/// needs it — and must not be rerouted through the `XCTAssertNoThrow` wrap.
#[test]
fn not_error_paired_with_a_real_assertion_keeps_the_result_binding() {
    let _ = take_inert_examples();

    let out = render_call(false, vec![not_error_assertion(), field_assertion("content")]);

    assert!(
        out.contains("let result = try Sample.listValidators()"),
        "the field assertion still needs `result` bound, got:\n{out}"
    );
    assert!(
        !out.contains("XCTAssertNoThrow"),
        "a fixture with a real assertion besides not_error must not be rerouted through the \
         not_error-only wrap, got:\n{out}"
    );
    assert!(
        out.contains("XCTAssertEqual"),
        "the real field assertion must still be rendered, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "a fixture with a real assertion must never be refused"
    );
}
