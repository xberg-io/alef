//! Regression coverage for Go declared-error-value assertion rendering: message/type
//! matching, string-literal escaping, and coded-vs-uncoded known error variants.
//!
//! Split out of `test_function.rs`, which is over the 1000-line cap and may not grow.

use super::declared_error_assertion::emit_declared_error_value_assertion;
use crate::core::ir::{ErrorDef, ErrorVariant};
use crate::e2e::fixture::{Assertion, Fixture};

fn fixture_with_declared_error(value: &str) -> Fixture {
    Fixture {
        id: "declares_error".to_string(),
        assertions: vec![Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn error_def_with(variant_name: &str, error_code: Option<u32>) -> Vec<ErrorDef> {
    vec![ErrorDef {
        exception_base: None,
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: variant_name.to_string(),
            error_code,
            is_unit: true,
            ..ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }]
}

#[test]
fn declared_value_emits_message_or_type_check() {
    let mut out = String::new();
    let fixture = fixture_with_declared_error("SomeExpectedError");

    emit_declared_error_value_assertion(&mut out, &fixture, &[]);

    assert_eq!(
        out,
        "\tif err != nil {\n\
         \t\tif !strings.Contains(err.Error(), `SomeExpectedError`) && !strings.Contains(fmt.Sprintf(\"%T\", err), `SomeExpectedError`) {\n\
         \t\t\tt.Errorf(\"expected error to match %s, got message=%q type=%T\", `SomeExpectedError`, err.Error(), err)\n\
         \t\t}\n\
         \t}\n"
    );
}

#[test]
fn no_declared_value_emits_nothing() {
    let mut out = String::new();
    let fixture = Fixture::default();

    emit_declared_error_value_assertion(&mut out, &fixture, &[]);

    assert_eq!(out, "", "no declared error value must leave output unchanged");
    assert!(!out.contains("strings."), "must not reference strings package");
    assert!(!out.contains("fmt."), "must not reference fmt package");
}

#[test]
fn declared_value_is_escaped_for_go_string_literal() {
    let mut out = String::new();
    let fixture = fixture_with_declared_error("contains \"quotes\" and a backtick `");

    emit_declared_error_value_assertion(&mut out, &fixture, &[]);

    assert!(
        out.contains("\"contains \\\"quotes\\\" and a backtick `\""),
        "expected escaped double-quoted literal, got: {out}"
    );
}

/// A CODED known variant still asserts.
#[test]
fn coded_known_variant_still_asserts() {
    let mut out = String::new();
    let fixture = fixture_with_declared_error("Authentication");
    let errors = error_def_with("Authentication", Some(100));

    emit_declared_error_value_assertion(&mut out, &fixture, &errors);

    assert!(out.contains("strings.Contains(err.Error()"), "got: {out}");
    assert!(out.contains("`Authentication`"), "got: {out}");
}

/// The defect this fix closes: a declared value naming a real `ErrorVariant` with no
/// `error_code` must render the registered skip, not an assertion that can never pass.
#[test]
fn uncoded_known_variant_renders_the_skip() {
    let mut out = String::new();
    let fixture = fixture_with_declared_error("Authentication");
    let errors = error_def_with("Authentication", None);

    emit_declared_error_value_assertion(&mut out, &fixture, &errors);

    assert_eq!(
        out,
        "\t// skipped: declared error variant 'Authentication' not substantiated by this backend's generated \
         error type\n"
    );
    assert!(
        !out.contains("strings.Contains"),
        "must not render an assertion, got: {out}"
    );
}

/// The status assertion of an error fixture is rendered through the generated `*Error`, not
/// skipped, and the funnel for the same fixture then names nothing.
#[test]
fn status_code_assertion_is_rendered_against_the_generated_error() {
    use super::declared_error_assertion::emit_field_checks;
    use crate::e2e::codegen::error_field_checks::test_fixtures::{status_code_errors, status_code_fixture};

    let mut out = String::new();
    let fixture = status_code_fixture(401);
    let reachable = emit_field_checks(&mut out, &fixture, &status_code_errors(), "pkg", "pkg");

    assert!(out.contains("var fieldError *pkg.ApiError"), "got: {out}");
    assert!(out.contains("fieldError.StatusCode != 401"), "got: {out}");
    assert!(
        crate::e2e::codegen::error_path_assertions::render_with_errors(&fixture, "\t// ", "go", &reachable).is_empty(),
        "the rendered assertion must not also be named as skipped"
    );
}

/// An error type without a `status_code` method has nothing to read, so the skip stays.
#[test]
fn status_code_assertion_stays_skipped_without_the_method() {
    use super::declared_error_assertion::emit_field_checks;
    use crate::e2e::codegen::error_field_checks::test_fixtures::{error_with_methods, status_code_fixture};

    let mut out = String::new();
    let fixture = status_code_fixture(401);
    let reachable = emit_field_checks(&mut out, &fixture, &error_with_methods("ApiError", &[]), "pkg", "pkg");

    assert_eq!(out, "");
    assert!(
        crate::e2e::codegen::error_path_assertions::render_with_errors(&fixture, "\t// ", "go", &reachable)
            .contains("no accessor for error field error.status_code")
    );
}
