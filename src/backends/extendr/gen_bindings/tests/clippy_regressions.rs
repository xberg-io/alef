use super::*;
use crate::backends::extendr::gen_bindings::{bridges, options};

#[test]
fn options_external_pointer_returns_the_binding_without_identity_conversion() {
    let api = make_api_surface();
    let output = options::gen_options_rs(&api, &api.types[0], "test_lib");
    assert!(output.contains("return Ok((*ext).clone());"), "{output}");
}

#[test]
fn optional_numbers_filter_null_before_decoding() {
    for primitive in [
        PrimitiveType::U64,
        PrimitiveType::I64,
        PrimitiveType::Usize,
        PrimitiveType::Isize,
        PrimitiveType::F64,
    ] {
        let mut api = make_api_surface();
        api.types[0].fields = vec![make_field(
            "limit",
            TypeRef::Optional(Box::new(TypeRef::Primitive(primitive))),
            true,
        )];
        let output = options::gen_options_rs(&api, &api.types[0], "test_lib");
        assert!(
            output.contains("list_get(&list, \"limit\").filter(|v| !v.is_null())"),
            "{output}"
        );
    }
}

#[test]
fn flat_string_enum_payloads_pass_through_without_identity_conversion() {
    let definition = EnumDef {
        name: "Action".to_string(),
        rust_path: "test_lib::Action".to_string(),
        variants: vec![EnumVariant {
            name: "Custom".to_string(),
            is_tuple: true,
            fields: vec![make_field("_0", TypeRef::String, false)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let from = bridges::gen_extendr_flat_data_enum_from_core(&definition, "test_lib");
    let to = bridges::gen_extendr_flat_data_enum_to_core(&definition, "test_lib");
    assert!(from.contains("custom: Some(_0),"), "{from}");
    assert!(to.contains("Self::Custom(val.custom.unwrap_or_default()),"), "{to}");
}

#[test]
fn flat_wrapped_string_payloads_keep_required_conversion() {
    let definition = EnumDef {
        name: "Action".to_string(),
        rust_path: "test_lib::Action".to_string(),
        variants: vec![EnumVariant {
            name: "Custom".to_string(),
            is_tuple: true,
            fields: vec![FieldDef {
                core_wrapper: CoreWrapper::Cow,
                ..make_field("_0", TypeRef::String, false)
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let from = bridges::gen_extendr_flat_data_enum_from_core(&definition, "test_lib");
    let to = bridges::gen_extendr_flat_data_enum_to_core(&definition, "test_lib");
    assert!(from.contains("custom: Some(_0.into()),"), "{from}");
    assert!(
        to.contains("Self::Custom(val.custom.unwrap_or_default().into()),"),
        "{to}"
    );
}

#[test]
fn optional_flagged_numbers_filter_null_before_decoding() {
    for primitive in [
        PrimitiveType::U64,
        PrimitiveType::I64,
        PrimitiveType::Usize,
        PrimitiveType::Isize,
        PrimitiveType::F32,
        PrimitiveType::F64,
    ] {
        let mut api = make_api_surface();
        api.types[0].fields = vec![make_field("max_depth", TypeRef::Primitive(primitive), true)];
        let output = options::gen_options_rs(&api, &api.types[0], "test_lib");
        assert!(
            output.contains("list_get(&list, \"max_depth\").filter(|v| !v.is_null())"),
            "{output}"
        );
    }
}

#[test]
fn options_field_bridge_maps_decode_errors_with_the_variant_constructor() {
    use crate::codegen::generators::trait_bridge::BridgeFieldMatch;
    use crate::core::config::TraitBridgeConfig;
    let api = make_api_surface();
    let field = make_field("visitor", TypeRef::Named("VisitorHandle".to_string()), true);
    let bridge = TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        ..Default::default()
    };
    let bridge_match = BridgeFieldMatch {
        param_index: 0,
        param_name: "options".to_string(),
        options_type: "Config".to_string(),
        param_is_optional: true,
        field_name: "visitor".to_string(),
        field: &field,
        bridge: &bridge,
    };
    let output = bridges::gen_extendr_bridge_field_function(&api, &api.functions[0], &bridge_match, "test_lib");
    assert!(
        output.contains("decode_options(options)\n        .map_err(extendr_api::Error::Other)?"),
        "{output}"
    );
}
