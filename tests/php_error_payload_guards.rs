use alef::codegen::error_gen::{
    gen_php_error_converter, gen_php_error_stubs, gen_php_error_types, php_error_class_names,
};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FieldDef, PrimitiveType, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        ..Default::default()
    }
}

fn error(name: &str, fields: Vec<FieldDef>) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: name.into(),
        variants: vec![ErrorVariant {
            name: "Rejected".into(),
            fields,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn surface(errors: Vec<ErrorDef>) -> ApiSurface {
    ApiSurface {
        errors,
        ..Default::default()
    }
}

#[test]
fn should_reject_incompatible_shared_variant_fields_at_every_class_entrypoint() {
    let api = surface(vec![
        error("ErrorA", vec![field("vendor", TypeRef::String)]),
        error("ErrorB", vec![field("status", TypeRef::Primitive(PrimitiveType::U16))]),
    ]);
    let results = [
        gen_php_error_types(&api, "Sample").map(|_| ()),
        gen_php_error_stubs(&api).map(|_| ()),
        php_error_class_names(&api).map(|_| ()),
    ];
    for result in results {
        let message = result.unwrap_err().to_string();
        for expected in [
            "RejectedError",
            "ErrorA::Rejected",
            "ErrorB::Rejected",
            "vendor",
            "status",
        ] {
            assert!(message.contains(expected), "{message}");
        }
    }
}

#[test]
fn should_reject_shared_payload_fields_with_different_types_or_optionality() {
    for ty in [
        TypeRef::Primitive(PrimitiveType::U16),
        TypeRef::Optional(Box::new(TypeRef::String)),
    ] {
        let api = surface(vec![
            error("ErrorA", vec![field("value", TypeRef::String)]),
            error("ErrorB", vec![field("value", ty)]),
        ]);
        assert!(gen_php_error_types(&api, "Sample").is_err());
    }
}

#[test]
fn should_keep_compatible_shared_payloads_when_field_order_differs() {
    let a = error(
        "ErrorA",
        vec![
            field("vendor", TypeRef::String),
            field("status", TypeRef::Primitive(PrimitiveType::U16)),
        ],
    );
    let b = error(
        "ErrorB",
        vec![
            field("status", TypeRef::Primitive(PrimitiveType::U16)),
            field("vendor", TypeRef::String),
        ],
    );
    let types = gen_php_error_types(&surface(vec![a, b]), "Sample").unwrap();
    assert_eq!(types.matches("pub struct PhpRejectedError").count(), 1);
    assert_eq!(types.matches("pub vendor: String").count(), 1);
    assert_eq!(types.matches("pub status: u16").count(), 1);
    assert!(types.contains("#[php(extends(PhpErrorA))]"));
}

#[test]
fn should_validate_original_payloads_before_hierarchy_variant_ownership_filters_them() {
    let a = error("ErrorA", vec![field("vendor", TypeRef::String)]);
    let mut b = error("ErrorB", vec![field("status", TypeRef::Primitive(PrimitiveType::U16))]);
    b.exception_base = Some("ErrorA".into());
    let message = gen_php_error_types(&surface(vec![a, b]), "Sample")
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("ErrorA::Rejected") && message.contains("ErrorB::Rejected"),
        "{message}"
    );
}

#[test]
fn should_keep_compatible_unit_classes_with_an_explicit_hierarchy() {
    let mut a = error("ErrorA", vec![]);
    a.variants[0].is_unit = true;
    let mut b = error("ErrorB", vec![]);
    b.variants[0].is_unit = true;
    b.exception_base = Some("ErrorA".into());
    let types = gen_php_error_types(&surface(vec![a, b]), "Sample").unwrap();
    assert_eq!(types.matches("pub struct PhpRejectedError").count(), 1);
    assert!(types.contains("#[php(extends(PhpErrorB))]"));
}

#[test]
fn should_reject_variant_payloads_that_collide_with_a_base_exception_class() {
    let a = error("ErrorA", vec![field("vendor", TypeRef::String)]);
    let mut b = error("RejectedError", vec![]);
    b.variants[0].name = "Other".into();
    b.variants[0].is_unit = true;
    let message = gen_php_error_types(&surface(vec![a, b]), "Sample")
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("RejectedError") && message.contains("ErrorA::Rejected"),
        "{message}"
    );
}

#[test]
fn should_reject_unit_and_payload_variants_sharing_one_exception_class() {
    let a = error("ErrorA", vec![field("vendor", TypeRef::String)]);
    let mut b = error("ErrorB", vec![]);
    b.variants[0].is_unit = true;
    assert!(gen_php_error_types(&surface(vec![a, b]), "Sample").is_err());
}

#[test]
fn should_omit_cfg_gated_fields_from_native_classes_stubs_and_converter_patterns() {
    let mut status = field("status", TypeRef::Primitive(PrimitiveType::U16));
    status.cfg = Some("feature = \"http\"".into());
    let error = error("ConversionError", vec![field("reason", TypeRef::String), status]);
    let api = surface(vec![error.clone()]);
    let types = gen_php_error_types(&api, "Sample").unwrap();
    let stubs = gen_php_error_stubs(&api).unwrap();
    let converter = gen_php_error_converter(&error, "sample");
    assert!(types.contains("pub reason: String"));
    assert!(!types.contains("pub status:"), "{types}");
    assert!(stubs.contains("$reason"));
    assert!(!stubs.contains("$status"), "{stubs}");
    assert!(
        converter.contains("ConversionError::Rejected { reason, .. }"),
        "{converter}"
    );
    assert!(!converter.contains("status: *status"), "{converter}");
}
