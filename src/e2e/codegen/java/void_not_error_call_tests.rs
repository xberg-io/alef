//! Regression coverage for the void `not_error` vacuous-test defect.
//!
//! ~keep A fixture whose only assertion is `{"type": "not_error"}` on a `returns_void` call
//! bound no `result_var` (`java/test_method.jinja`'s `{% if returns_void %}` branch calls
//! without assigning), so `assertions.rs`'s `not_error` arm left the assertion unrendered on
//! the theory that the call's `throws Exception` clause already covers it. That reasoning is
//! the same vacuous-test shape the Kotlin `not_error` fix (`kotlin/not_error.rs`) closed for
//! non-void calls: an uncaught exception does fail the `@Test` method, but `not_error` must
//! still leave a real, visible assertion — not rely on the surrounding method signature being
//! declared `throws`. The fix wraps `call_expr` itself in JUnit 5's `assertDoesNotThrow(() ->
//! ...)` via `test_method.rs`'s `void_not_error` flag, since there is no `result_var` to
//! assert on the way the non-void case does.
//!
//! This lives in its own file rather than growing `test_method.rs`'s existing test module: that
//! file is already over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::test_method::render_test_method;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn void_fixture(id: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
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
        client_options: None,
        preserve_input_urls: false,
        assertions,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn render_void_call(assertions: Vec<Assertion>) -> String {
    let fixture = void_fixture("prefetch_languages", assertions);
    let call = CallConfig {
        function: "prefetchLanguages".to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        returns_void: true,
        ..Default::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

    let mut out = String::new();
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
        &type_defs,
        &[],
        &[],
        &[],
    );
    out
}

/// The regression this file exists for: before the fix, a void `not_error`-only fixture
/// rendered `prefetchLanguages();` with no assertion anywhere in the method body — green
/// because it checked nothing.
#[test]
fn void_not_error_wraps_the_call_in_assert_does_not_throw() {
    let out = render_void_call(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);

    assert!(
        out.contains("assertDoesNotThrow(() -> SampleClass.prefetchLanguages());"),
        "expected the void call wrapped in assertDoesNotThrow, got:\n{out}"
    );
    assert!(
        !out.contains("        SampleClass.prefetchLanguages();\n"),
        "the call must not also appear as a bare, unasserted statement, got:\n{out}"
    );
}

/// A void fixture with no `not_error` assertion at all (e.g. one whose only checks are on
/// declared error paths, handled elsewhere) must keep emitting the bare call — wrapping every
/// void call regardless of what it asserts would be a different, unrequested behavior change.
#[test]
fn void_call_without_not_error_stays_a_bare_statement() {
    let out = render_void_call(vec![]);

    assert!(
        out.contains("        SampleClass.prefetchLanguages();\n"),
        "expected a bare call statement, got:\n{out}"
    );
    assert!(
        !out.contains("assertDoesNotThrow"),
        "must not wrap the call when no not_error assertion is present, got:\n{out}"
    );
}
