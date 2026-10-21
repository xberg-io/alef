use super::*;

#[test]
fn should_preserve_named_scalar_error_fields_as_python_attributes() {
    let mut error = sample_error();
    let mut status = named_field("status");
    status.ty = TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U16)));
    let mut hidden = named_field("secret");
    hidden.sensitive = true;
    let mut wrapped = named_field("wrapped");
    wrapped.core_wrapper = CoreWrapper::Arc;
    error.variants = vec![ErrorVariant {
        name: "Rejected".to_string(),
        fields: vec![
            named_field("vendor"),
            status,
            named_field("body"),
            hidden,
            named_field("args"),
            named_field("type"),
            wrapped,
        ],
        ..Default::default()
    }];
    let output = gen_pyo3_error_converter(&error, "sample_markup_rs");
    assert!(
        output.contains("value.setattr(\"vendor\", Clone::clone(vendor))?;"),
        "{output}"
    );
    assert!(
        output.contains("value.setattr(\"status\", Clone::clone(status))?;"),
        "{output}"
    );
    assert!(
        output.contains("value.setattr(\"body\", Clone::clone(body))?;"),
        "{output}"
    );
    assert!(!output.contains("value.setattr(\"secret\""), "{output}");
    assert!(!output.contains("value.setattr(\"args\""), "{output}");
    assert!(!output.contains("value.setattr(\"wrapped\""), "{output}");
    assert!(output.contains("RejectedError::new_err(msg)"), "{output}");
    assert!(!output.contains(".clone()"), "{output}");
    assert!(
        output.contains("let sample_markup_rs::ConversionError::Rejected"),
        "{output}"
    );
    assert!(
        output.contains("if let sample_markup_rs::ConversionError::Rejected"),
        "{output}"
    );
    syn::parse_file(&output).expect("generated converter is valid Rust");
}

#[test]
fn should_match_a_single_payload_variant_without_a_single_arm_match() {
    let mut error = sample_error();
    error.variants[0].is_tuple = false;
    error.variants[0].fields = vec![named_field("vendor")];
    let output = gen_pyo3_error_converter(&error, "sample_markup_rs");
    assert!(
        output.contains("if let sample_markup_rs::ConversionError::ParseError"),
        "{output}"
    );
    syn::parse_file(&output).expect("generated converter is valid Rust");
}
