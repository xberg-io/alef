//! The rendered Dart test file for a fixture that expects a typed error.

use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use crate::e2e::fixture::{Assertion, Fixture};

fn api_error() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "Authentication".to_string(),
            is_unit: true,
            ..ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![MethodDef {
            name: "status_code".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::U16),
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn unauthorized_fixture() -> Fixture {
    Fixture {
        id: "unauthorized".to_string(),
        description: "unauthorized".to_string(),
        assertions: vec![
            Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::json!("Authentication")),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("error.status_code".to_string()),
                value: Some(serde_json::json!(401)),
                ..Assertion::default()
            },
        ],
        ..Fixture::default()
    }
}

fn render(fixture: &Fixture, errors: &[ErrorDef]) -> String {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "parseThing".into();
    super::test_file::render_test_file(
        "smoke",
        &[fixture],
        &e2e_config,
        "dart",
        "samplecli",
        "RustLib",
        "RustLibBridge",
        &crate::e2e::field_access::DartFirstClassMap::default(),
        &[],
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
        errors,
    )
}

/// The test asserts the generated class of the declared variant and reads the status code from
/// the caught error, so a wrong class or a wrong code fails the test.
#[test]
fn typed_error_fixture_asserts_the_class_and_the_field() {
    let _ = crate::e2e::codegen::take_skip_records();
    let output = render(&unauthorized_fixture(), &[api_error()]);

    assert!(output.contains("throwsA(equals(ApiError.authentication))"), "{output}");
    assert!(output.contains("Object? caughtError;"), "{output}");
    assert!(
        output.contains("expect(await (caughtError as ApiError).statusCode(), equals(401));"),
        "{output}"
    );
    assert!(!output.contains("error.status_code in this backend"), "{output}");
    assert!(!output.contains("throwsA(anything)"), "{output}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

/// With no error registry the same fixture keeps the message check and names the field it
/// cannot read, as before.
#[test]
fn fixture_without_an_error_registry_keeps_the_message_check() {
    let _ = crate::e2e::codegen::take_skip_records();
    let output = render(&unauthorized_fixture(), &[]);

    assert!(output.contains("throwsA(predicate("), "{output}");
    assert!(
        output.contains("has no accessor for error field error.status_code in this backend"),
        "{output}"
    );
    let _ = crate::e2e::codegen::take_skip_records();
}

#[test]
fn nullable_payload_and_introspection_assertions_are_both_rendered_once() {
    let mut error = api_error();
    error.variants[0].is_unit = false;
    error.variants[0].fields.push(crate::core::ir::FieldDef {
        name: "retry_after".into(),
        ty: TypeRef::Primitive(PrimitiveType::U16),
        optional: true,
        ..Default::default()
    });
    let mut fixture = unauthorized_fixture();
    fixture.assertions.push(Assertion {
        assertion_type: "equals".into(),
        field: Some("error.retry_after".into()),
        value: Some(serde_json::Value::Null),
        ..Default::default()
    });
    let _ = crate::e2e::codegen::take_skip_records();
    let output = render(&fixture, &[error]);
    assert_eq!(
        output
            .matches(".having((e) => e.retryAfter, 'retry_after', null)")
            .count(),
        1,
        "{output}"
    );
    assert_eq!(
        output
            .matches("expect(await (caughtError as ApiError).statusCode(), equals(401));")
            .count(),
        1,
        "{output}"
    );
    assert!(!output.contains("has no accessor for error field"), "{output}");
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}

#[test]
fn excluded_errors_report_unrendered_assertions_instead_of_assuming_a_typed_class() {
    let mut error = api_error();
    error.binding_excluded = true;
    let _ = crate::e2e::codegen::take_skip_records();
    let output = render(&unauthorized_fixture(), &[error]);
    assert_eq!(
        output.matches("declared error variant 'Authentication'").count(),
        1,
        "{output}"
    );
    assert!(output.contains("throwsA(anything)"), "{output}");
    assert!(!output.contains("ApiError.authentication"), "{output}");
    assert!(!output.contains("caughtError as ApiError"), "{output}");
    assert_eq!(crate::e2e::codegen::take_skip_records().len(), 2);
}
