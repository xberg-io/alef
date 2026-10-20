use super::*;

#[cfg(test)]
mod vacuous_assertion_fallback_tests {
    use super::has_real_dart_assertion;

    #[test]
    fn has_real_dart_assertion_is_false_for_comment_only_body() {
        let body = "    // skipped: field 'foo' not available on dart result type\n";
        assert!(
            !has_real_dart_assertion(body),
            "comment-only body must not count as asserting"
        );
    }

    #[test]
    fn has_real_dart_assertion_is_false_for_empty_body() {
        assert!(!has_real_dart_assertion(""));
        assert!(!has_real_dart_assertion("   \n  \n"));
    }

    #[test]
    fn has_real_dart_assertion_is_true_when_a_real_statement_is_present() {
        let body = "    // skipped: field 'foo' not available on dart result type\n    expect(result.ok, isTrue);\n";
        assert!(
            has_real_dart_assertion(body),
            "a real expect(...) line must count as asserting"
        );
    }
}

#[cfg(test)]
mod dart_error_matcher_tests {
    use super::dart_error_matcher;
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

    fn coded_error_def(variant_name: &str) -> ErrorDef {
        ErrorDef {
            exception_base: None,
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn no_declared_value_renders_throws_anything_with_no_comment() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &[]);
        assert_eq!(matcher, "throwsA(anything)");
        assert_eq!(out, "", "no declared value must leave the output buffer untouched");
    }

    /// With no `errors` IR supplied, a value cannot be recognised as a known variant name, so it
    /// renders exactly like a message-style value always did before this fix.
    #[test]
    fn message_style_value_renders_the_message_or_type_predicate() {
        let fixture = fixture_with_declared_error("BadRequest");
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &[]);
        assert_eq!(
            matcher,
            "throwsA(predicate((e) => e.toString().contains('BadRequest') || e.runtimeType.toString().contains('BadRequest')))"
        );
        assert_eq!(out, "");
    }

    /// A declared value that names a variant of a mirrored error must assert that variant's
    /// generated class. `throwsA(anything)` passes for any thrown value, so no fixture could
    /// fail on a wrong error class.
    #[test]
    fn declared_value_naming_a_known_variant_asserts_the_generated_class() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = vec![coded_error_def("Authentication")];
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &errors);
        assert_eq!(matcher, "throwsA(isA<ApiError_Authentication>())");
        assert_eq!(out, "", "a substantiated variant records no skip");
    }

    /// The class is the same for a variant with no `error_code`: its identity comes from the
    /// bridge, not from the C ABI taxonomy.
    #[test]
    fn variant_without_an_error_code_still_asserts_its_class() {
        let fixture = fixture_with_declared_error("Authentication");
        let mut errors = vec![coded_error_def("Authentication")];
        errors[0].variants[0].error_code = None;
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &errors);
        assert_eq!(matcher, "throwsA(isA<ApiError_Authentication>())");
    }

    /// An error excluded from the binding has no Dart class, so the variant cannot be asserted:
    /// the matcher falls back to "the call must fail" and the skip is recorded.
    #[test]
    fn variant_of_an_error_excluded_from_the_binding_records_a_skip() {
        let fixture = fixture_with_declared_error("Authentication");
        let mut errors = vec![coded_error_def("Authentication")];
        errors[0].binding_excluded = true;
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &errors);
        assert_eq!(matcher, "throwsA(anything)");
        assert_eq!(
            out,
            "    // skipped: declared error variant 'Authentication' not substantiated by this backend's generated \
             error type\n"
        );
    }
}

#[cfg(test)]
mod dart_error_field_check_tests {
    use super::super::super::error_assertions::field_checks;
    use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, PrimitiveType, ReceiverKind, TypeRef};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn error_with_status_code() -> ErrorDef {
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

    fn fixture_asserting_status(value: serde_json::Value) -> Fixture {
        Fixture {
            id: "unauthorized".to_string(),
            assertions: vec![
                Assertion {
                    assertion_type: "error".to_string(),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("error.status_code".to_string()),
                    value: Some(value),
                    ..Assertion::default()
                },
            ],
            ..Fixture::default()
        }
    }

    /// A field a fixture names is read from the caught error through its generated method, and
    /// compared with the fixture value, so a wrong value fails the test.
    #[test]
    fn error_field_assertion_reads_the_field_from_the_caught_error() {
        let fixture = fixture_asserting_status(serde_json::json!(401));
        let mut out = String::new();
        let checks = field_checks(&mut out, "    ", &fixture, &[error_with_status_code()]);
        assert_eq!(
            checks,
            vec!["    expect(await (caughtError as ApiError).statusCode(), equals(401));".to_string()]
        );
        assert_eq!(out, "");
    }

    /// A value Dart cannot compare with `equals` is named as a skip, never dropped.
    #[test]
    fn error_field_value_without_a_dart_literal_records_a_skip() {
        let fixture = fixture_asserting_status(serde_json::json!([401]));
        let mut out = String::new();
        let checks = field_checks(&mut out, "    ", &fixture, &[error_with_status_code()]);
        assert!(checks.is_empty());
        assert!(out.contains("has no accessor for error field error.status_code"), "{out}");
    }

    /// A field that no bridged method answers renders nothing here; the shared marker names it.
    #[test]
    fn unknown_error_field_renders_no_check() {
        let mut fixture = fixture_asserting_status(serde_json::json!(401));
        fixture.assertions[1].field = Some("error.retry_after".to_string());
        let mut out = String::new();
        let checks = field_checks(&mut out, "    ", &fixture, &[error_with_status_code()]);
        assert!(checks.is_empty());
        assert_eq!(out, "");
    }
}
