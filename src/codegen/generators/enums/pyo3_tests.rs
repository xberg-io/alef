use super::*;
use crate::core::ir::{NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, TypeRef};

fn wrapper(paths: Vec<Vec<NewtypeContainer>>) -> String {
    NewtypeWrapper::encode_explicit(
        &paths
            .into_iter()
            .map(|containers| {
                NewtypeWrapperMetadata::transparent_string("toolkit::SecretString", "from", "into_inner", containers)
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn factory_preserves_transparent_wrappers_and_boxes() {
    let required = ParamDef {
        name: "value".to_string(),
        ty: TypeRef::String,
        newtype_wrapper: Some(wrapper(vec![vec![]])),
        ..ParamDef::default()
    };
    assert_eq!(
        pyo3_variant_field_init(&required, false, true),
        "Box::new(toolkit::SecretString::from(value))"
    );
    let optional = ParamDef {
        optional: true,
        newtype_wrapper: Some(wrapper(vec![vec![NewtypeContainer::Optional]])),
        ..required.clone()
    };
    assert_eq!(
        pyo3_variant_field_init(&optional, false, true),
        "(value).map(|value| toolkit::SecretString::from(value)).map(Box::new)"
    );
    assert_eq!(
        pyo3_variant_field_init(&required, true, true),
        "Box::new(toolkit::SecretString::from(value.unwrap_or_default()))"
    );
}

#[test]
fn factory_converts_nested_wrapper_paths_and_preserves_plain_fields() {
    let wrapped = ParamDef {
        name: "values".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        newtype_wrapper: Some(wrapper(vec![
            vec![NewtypeContainer::MapKey],
            vec![NewtypeContainer::MapValue],
        ])),
        ..ParamDef::default()
    };
    assert_eq!(
        pyo3_variant_field_init(&wrapped, false, false),
        "(values).into_iter().map(|(key, value)| (toolkit::SecretString::from(key), toolkit::SecretString::from(value))).collect()"
    );
    let plain = ParamDef {
        name: "value".to_string(),
        ty: TypeRef::String,
        ..ParamDef::default()
    };
    assert_eq!(
        pyo3_variant_field_init(&plain, false, false),
        variant_field_init(&plain, false, false, false, false)
    );
}

#[test]
fn factory_composes_non_serde_wrapper_keys_with_dto_value_coercion() {
    let param = ParamDef {
        name: "values".to_string(),
        ty: TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("Config".to_string())),
        ),
        newtype_wrapper: Some(wrapper(vec![vec![NewtypeContainer::MapKey]])),
        ..ParamDef::default()
    };
    let coerced = crate::codegen::generators::dto_coercion::coercible_field_init(
        "values",
        "Config",
        crate::codegen::generators::dto_coercion::CoercibleShape::Map,
        false,
        false,
    );
    assert_eq!(
        pyo3_variant_field_init_with_coercion(&param, false, false, Some(coerced)),
        "(__alef_coerce_dto_map(py, values, __ALEF_WIRE_CONFIG)?).into_iter().map(|(key, value)| (toolkit::SecretString::from(key), value)).collect()"
    );
}
