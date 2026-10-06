use super::*;
use crate::core::ir::{ErrorVariant, FieldDef, MethodDef, TypeRef};

fn field(name: &str, sensitive: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        sensitive,
        ..Default::default()
    }
}

fn error_with(template: &str, fields: Vec<FieldDef>) -> ErrorDef {
    ErrorDef {
        name: "AuthError".to_string(),
        rust_path: "core::AuthError".to_string(),
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
        variants: vec![ErrorVariant {
            name: "Rejected".to_string(),
            message_template: Some(template.to_string()),
            fields,
            is_tuple: true,
            ..Default::default()
        }],
    }
}

#[test]
fn sensitive_tuple_field_is_redacted_in_the_exception_message() {
    let error = error_with("rejected {0} for {1}", vec![field("_0", true), field("_1", false)]);
    let mut out = String::new();
    emit_error_type_with_imports(&error, &mut out, &mut BTreeSet::new());

    assert!(out.contains("rejected <redacted> for $field1"), "{out}");
    assert!(
        !out.contains("$field0"),
        "a sensitive field must not reach the message: {out}"
    );
}

#[test]
fn non_sensitive_tuple_field_is_still_interpolated() {
    let error = error_with("rejected {0}", vec![field("_0", false)]);
    let mut out = String::new();
    emit_error_type_with_imports(&error, &mut out, &mut BTreeSet::new());

    assert!(out.contains("rejected $field0"), "{out}");
}

#[test]
fn variant_field_matching_an_error_accessor_overrides_it() {
    let mut error = error_with("rate limited: {message}", vec![]);
    error.variants[0].is_tuple = false;
    error.variants[0].fields = vec![
        field("message", false),
        FieldDef {
            name: "retry_after".to_string(),
            ty: TypeRef::Duration,
            optional: true,
            ..Default::default()
        },
    ];
    error.methods = vec![MethodDef {
        name: "retry_after".to_string(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Duration)),
        ..Default::default()
    }];
    let mut out = String::new();
    emit_error_type_with_imports(&error, &mut out, &mut BTreeSet::new());

    assert!(out.contains("override val retryAfter: Duration?"), "{out}");
    assert!(out.contains("open val retryAfter: Duration? = null"), "{out}");
}
