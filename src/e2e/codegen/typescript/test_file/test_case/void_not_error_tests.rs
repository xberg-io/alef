use super::*;
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Assertion;

/// Regression coverage for the void `not_error` defect: before this fix, a fixture whose
/// only assertion was `not_error` on a `returns_void` call rendered `expect(result)
/// .toBeDefined()` — but napi-rs maps a void call's `Ok(())` to JS `undefined`, so that
/// assertion FAILED every successful call, not just an unsuccessful one. Worse than the
/// vacuous body it replaced.
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
        module: "myLib".to_string(),
        result_var: "result".to_string(),
        returns_void: true,
        ..Default::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<TypeDef> = Vec::new();
    let enums: Vec<EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &type_defs,
        &enums,
        &[],
        "",
        &config,
        &mut referenced_enums,
        &errors,
    );
    out
}

/// The regression this test exists for: before this earlier fix, a void `not_error`-only
/// fixture rendered `const result = await prefetchLanguages(); expect(result).toBeDefined();`
/// — an assertion that fails on every successful call, since a void call resolves `undefined`.
/// `CallConfig::default()` is synchronous, so the wrapper shape asserted here is the sync one
/// (`expect(() => ...)`, no Promise); see `void_not_error_call_tests.rs` for the sibling
/// async-shape coverage and the sync/async selection defect this split guards against.
#[test]
fn void_not_error_wraps_the_call_without_asserting_tobedefined() {
    let out = render_void_call(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);

    assert!(
        out.contains("expect(() => prefetchLanguages()).not.toThrow();"),
        "expected the sync void call wrapped in expect(() => ...).not.toThrow(), got:\n{out}"
    );
    assert!(
        !out.contains("toBeDefined()"),
        "must not assert toBeDefined() on a void call's always-undefined result, got:\n{out}"
    );
}

/// A void `not_error` fixture over a *synchronous* call renders its wrapper expression
/// (`expect(() => ...).not.toThrow()`, see `void_not_error_call_tests.rs`) inside the `it(..)`
/// callback whose `async` keyword is frozen into `async_kw` by `test_is_async` two lines above
/// `void_not_error`'s use site. `test_is_async` forces `async` whenever `void_not_error` is set
/// (regardless of `call_is_async`) so that other backends' analogous async-body content, or a
/// future template change, can never reintroduce an `await` stranded in a non-async callback —
/// that would not be a weak assertion, it would be a TS/JS syntax error, and it aborted the
/// entire formatting phase of `alef all` (taking every other language's formatting with it)
/// rather than failing one test. `CallConfig::default()` is synchronous, which is exactly the
/// shape that reproduces it.
#[test]
fn sync_void_not_error_marks_the_test_callback_async() {
    let out = render_void_call(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);

    assert!(
        out.contains("async () => {"),
        "a body containing `await` must live in an async callback, got:\n{out}"
    );
    let awaits_in_body = out.contains("await ");
    let callback_is_async = out.contains("async () => {");
    assert!(
        !awaits_in_body || callback_is_async,
        "`await` outside an async function is a syntax error, got:\n{out}"
    );
}

/// A void fixture with no `not_error` assertion at all must keep emitting a bare call —
/// wrapping every void call regardless of what it asserts would be a different, unrequested
/// behavior change.
#[test]
fn void_call_without_not_error_stays_a_bare_statement() {
    let out = render_void_call(vec![]);

    assert!(
        out.contains("prefetchLanguages();"),
        "expected a bare call statement, got:\n{out}"
    );
    assert!(!out.contains("resolves.not.toThrow"), "got:\n{out}");
}
