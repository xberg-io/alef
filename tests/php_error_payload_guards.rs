use alef::codegen::error_gen::{
    gen_php_error_converter_with_errors, gen_php_error_stubs, gen_php_error_types, php_error_class_names,
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
fn should_reject_shared_variant_fields_with_conflicting_types_at_every_class_entrypoint() {
    let api = surface(vec![
        error("ErrorA", vec![field("status", TypeRef::String)]),
        error("ErrorB", vec![field("status", TypeRef::Primitive(PrimitiveType::U16))]),
    ]);
    let results = [
        gen_php_error_types(&api, "Sample", None).map(|_| ()),
        gen_php_error_stubs(&api, None).map(|_| ()),
        php_error_class_names(&api, None).map(|_| ()),
    ];
    for result in results {
        let message = result.unwrap_err().to_string();
        for expected in ["RejectedError", "ErrorA::Rejected", "ErrorB::Rejected", "status", "u16"] {
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
        assert!(gen_php_error_types(&api, "Sample", None).is_err());
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
    let types = gen_php_error_types(&surface(vec![a, b]), "Sample", None).unwrap();
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
    let message = gen_php_error_types(&surface(vec![a, b]), "Sample", None)
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
    let types = gen_php_error_types(&surface(vec![a, b]), "Sample", None).unwrap();
    assert_eq!(types.matches("pub struct PhpRejectedError").count(), 1);
    assert!(types.contains("#[php(extends(PhpErrorB))]"));
}

#[test]
fn should_reject_variant_payloads_that_collide_with_a_base_exception_class() {
    let a = error("ErrorA", vec![field("vendor", TypeRef::String)]);
    let mut b = error("RejectedError", vec![]);
    b.variants[0].name = "Other".into();
    b.variants[0].is_unit = true;
    let message = gen_php_error_types(&surface(vec![a, b]), "Sample", None)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("RejectedError") && message.contains("ErrorA::Rejected"),
        "{message}"
    );
}

#[test]
fn should_make_payload_fields_nullable_on_a_class_shared_with_a_unit_variant() {
    let a = error("ErrorA", vec![field("vendor", TypeRef::String)]);
    let mut b = error("ErrorB", vec![]);
    b.variants[0].is_unit = true;
    let api = surface(vec![a, b]);
    let types = gen_php_error_types(&api, "Sample", None).unwrap();
    assert_eq!(types.matches("pub struct PhpRejectedError").count(), 1);
    assert!(types.contains("pub vendor: Option<String>"), "{types}");
    assert!(types.contains("#[php(extends(PhpErrorA))]\n#[php(implements(PhpInterfaceErrorBInterface))]"));
    let unit_converter = gen_php_error_converter_with_errors(&api.errors[1], &api.errors, "sample");
    assert!(unit_converter.contains("vendor: None,"), "{unit_converter}");
    let payload_converter = gen_php_error_converter_with_errors(&api.errors[0], &api.errors, "sample");
    assert!(
        payload_converter.contains("vendor: Some(ToString::to_string(vendor)),"),
        "{payload_converter}"
    );
}

#[test]
fn should_omit_cfg_gated_fields_from_native_classes_stubs_and_converter_patterns() {
    let mut status = field("status", TypeRef::Primitive(PrimitiveType::U16));
    status.cfg = Some("feature = \"http\"".into());
    let error = error("ConversionError", vec![field("reason", TypeRef::String), status]);
    let api = surface(vec![error.clone()]);
    let types = gen_php_error_types(&api, "Sample", None).unwrap();
    let stubs = gen_php_error_stubs(&api, None).unwrap();
    let converter = gen_php_error_converter_with_errors(&error, std::slice::from_ref(&error), "sample");
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

#[test]
fn should_give_a_family_base_declared_as_another_family_variant_its_marker_interface() {
    let mut app = error("AppError", vec![]);
    app.variants[0].name = "GraphQL".into();
    app.variants[0].is_unit = true;
    let mut graphql = error("GraphQLError", vec![]);
    graphql.variants[0].name = "Parse".into();
    graphql.variants[0].is_unit = true;
    let types = gen_php_error_types(&surface(vec![app, graphql]), "Sample", None).unwrap();
    assert_eq!(types.matches("pub struct PhpGraphQLError {").count(), 1, "{types}");
    assert!(
        types.contains(
            "#[php(extends(PhpAppError))]\n#[php(implements(PhpInterfaceGraphQLErrorInterface))]\n\
             #[derive(Default)]\npub struct PhpGraphQLError {"
        ),
        "{types}"
    );
}
