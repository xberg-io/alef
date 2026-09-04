//! A generated `@Test` method must declare `Unit`, or JUnit 5 never runs it.

use super::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// `not_error` is the assertion that exposed this: it renders `assertNotNull(result, ...)`,
/// and `kotlin.test.assertNotNull` returns the asserted value rather than `Unit`. ~keep
fn not_error() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }
}

fn render(kotlin_android_style: bool, assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "list_validators".to_string(),
        description: "list registered validators".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "listValidators".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        kotlin_android_style,
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("render_test_method succeeds");
    out
}

/// The defect: an expression-bodied `fun x() = runBlocking { ... }` takes its return type from
/// the lambda's final expression. When that is a value-returning assertion the method is not
/// `Unit`, and JUnit 5 does not execute a non-void `@Test` -- silently, with the suite still
/// reporting BUILD SUCCESSFUL. Measured in one consumer: 18 of 97 generated
/// kotlin-android tests never ran, one class losing all five of its own. Declaring the return type on the signature fixes
/// every branch at once, where a trailing `Unit` statement only fixes the branch it is in.
#[test]
fn should_declare_unit_return_when_the_body_is_a_runblocking_expression() {
    let out = render(true, vec![not_error()]);
    assert!(
        out.contains("fun testListValidators(): Unit = runBlocking {"),
        "expression-bodied @Test must declare `: Unit` or JUnit 5 skips it; got:\n{out}"
    );
}

/// CONTROL: the block-bodied form is already `Unit` and must not grow a redundant annotation,
/// which would prove the assertion above matches the signature rather than the whole file.
#[test]
fn should_not_annotate_a_block_bodied_test_method() {
    let out = render(false, vec![not_error()]);
    assert!(
        out.contains("fun testListValidators() {"),
        "block-bodied @Test should stay unannotated; got:\n{out}"
    );
    assert!(
        !out.contains("fun testListValidators(): Unit"),
        "block body needs no return-type annotation; got:\n{out}"
    );
}
