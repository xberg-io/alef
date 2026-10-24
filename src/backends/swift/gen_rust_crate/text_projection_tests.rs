use super::*;
use crate::core::ir::{NewtypeWrapper, NewtypeWrapperMetadata};

#[test]
fn configured_text_projection_has_one_return_shape_authority() {
    let configured = HashSet::from(["BodyValue"]);
    for (ty, optional, expected) in [
        (TypeRef::Named("BodyValue".into()), false, Some("String")),
        (TypeRef::Named("BodyValue".into()), true, Some("Option<String>")),
        (
            TypeRef::Optional(Box::new(TypeRef::Named("BodyValue".into()))),
            false,
            Some("Option<String>"),
        ),
        (TypeRef::Named("OtherValue".into()), true, None),
        (TypeRef::String, false, None),
    ] {
        let field = FieldDef {
            ty,
            optional,
            ..Default::default()
        };
        assert_eq!(text_projected_getter_type(&field, &configured), expected);
        assert_eq!(text_projected_getter_type(&field, &HashSet::new()), None);
    }
}

#[test]
fn explicit_newtype_conversion_keeps_its_existing_transport_precedence() {
    let metadata = NewtypeWrapperMetadata::transparent_string(
        "sample::BodyValue",
        "sample::BodyValue::from",
        "sample::BodyValue::into_inner",
        vec![],
    );
    let field = FieldDef {
        ty: TypeRef::Named("BodyValue".into()),
        newtype_wrapper: Some(NewtypeWrapper::encode_explicit(&[metadata])),
        ..Default::default()
    };
    assert_eq!(text_projected_getter_type(&field, &HashSet::from(["BodyValue"])), None);
}

fn reply_type(optional: bool) -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: "Reply".into(),
        rust_path: "sample::Reply".into(),
        has_serde: true,
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Named("BodyValue".into()),
            optional,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn reply_extern_block(optional: bool, configured: &HashSet<&str>) -> String {
    let all = HashSet::from(["BodyValue"]);
    let unit = HashSet::new();
    crate::backends::swift::gen_rust_crate::extern_block::emit_extern_block_for_type(
        &reply_type(optional),
        &HashSet::new(),
        &std::collections::HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        crate::backends::swift::gen_rust_crate::default_construction::EnumKinds::new(&all, &unit),
        configured,
        &HashSet::new(),
    )
}

#[test]
fn json_getter_is_declared_beside_the_text_getter_with_the_same_optionality() {
    let configured = HashSet::from(["BodyValue"]);
    let optional = reply_extern_block(true, &configured);
    assert!(optional.contains("fn content(&self) -> Option<String>;"), "{optional}");
    assert!(
        optional.contains(
            "#[swift_bridge(swift_name = \"contentJson\")]\n        fn content_json(&self) -> Option<String>;"
        ),
        "{optional}"
    );
    let required = reply_extern_block(false, &configured);
    assert!(required.contains("fn content(&self) -> String;"), "{required}");
    assert!(
        required.contains("#[swift_bridge(swift_name = \"contentJson\")]\n        fn content_json(&self) -> String;"),
        "{required}"
    );
}

#[test]
fn json_getter_is_not_declared_for_a_field_outside_the_configured_text_types() {
    let block = reply_extern_block(true, &HashSet::new());
    assert!(!block.contains("content_json"), "{block}");
}
