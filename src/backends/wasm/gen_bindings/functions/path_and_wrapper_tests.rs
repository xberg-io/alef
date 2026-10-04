use super::*;

#[test]
fn nested_named_jsvalue_recovery_uses_surface_rust_path() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut func = async_function(vec![param(
        "values",
        TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Named(
            "NestedThing".to_string(),
        ))))),
    )]);
    func.is_async = false;
    func.error_type = None;
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "NestedThing".to_string(),
            rust_path: "sample_fixture::nested::NestedThing".to_string(),
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(
        out.contains("Vec<Option<sample_fixture::nested::NestedThing>>"),
        "nested named recovery must use TypeDef.rust_path: {out}"
    );
}

#[test]
fn async_vec_named_recovery_uses_surface_rust_path() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let func = async_function(vec![param(
        "values",
        TypeRef::Vec(Box::new(TypeRef::Named("NestedThing".to_string()))),
    )]);
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "NestedThing".to_string(),
            rust_path: "sample_fixture::nested::NestedThing".to_string(),
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(out.contains("Vec<sample_fixture::nested::NestedThing>"), "{out}");
}

#[test]
fn input_dto_conversion_uses_resolved_nested_type_path() {
    let type_def = crate::core::ir::TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "sample_fixture::nested::NestedConfig".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "child".to_string(),
            ty: TypeRef::Named("NestedChild".to_string()),
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    };
    let type_paths = ahash::AHashMap::from_iter([(
        "NestedChild".to_string(),
        "sample_fixture::nested::NestedChild".to_string(),
    )]);

    let (out, _) = gen_input_dto_for_type_at_path(
        "NestedConfig",
        "sample_fixture::nested::NestedConfig",
        "sample_fixture",
        &type_paths,
        &type_def,
    );

    assert!(
        out.contains("impl From<NestedConfigInput> for sample_fixture::nested::NestedConfig"),
        "{out}"
    );
    assert!(out.contains("Option<sample_fixture::nested::NestedChild>"), "{out}");
}

#[test]
fn input_dto_uses_field_wire_name_and_rebuilds_transparent_wrapper() {
    let type_def = crate::core::ir::TypeDef {
        name: "Credentials".to_string(),
        rust_path: "sample_fixture::Credentials".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "secret_value".to_string(),
            ty: TypeRef::String,
            is_boxed: true,
            serde_rename: Some("credential".to_string()),
            newtype_wrapper: Some(transparent_string_wrapper(vec![])),
            ..Default::default()
        }],
        has_default: true,
        has_serde: true,
        ..Default::default()
    };

    let (out, _) = gen_input_dto_for_type("Credentials", "sample_fixture", &type_def);

    assert!(out.contains("#[serde(rename = \"credential\")]"), "{out}");
    assert!(out.contains("let mut out = Self::default();"), "{out}");
    assert!(
        out.contains("out.secret_value = Box::new(sample_fixture::SecretString::from(v));"),
        "{out}"
    );
}

#[test]
fn sanitized_fallible_named_recovery_uses_surface_rust_path() {
    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut value = param("value", TypeRef::Named("NestedThing".to_string()));
    value.sanitized = true;
    let mut func = async_function(vec![value]);
    func.is_async = false;
    func.sanitized = true;
    let api = crate::core::ir::ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "NestedThing".to_string(),
            rust_path: "sample_fixture::nested::NestedThing".to_string(),
            has_serde: true,
            ..Default::default()
        }],
        ..empty_surface()
    };

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &api,
        &AHashSet::new(),
    );

    assert!(out.contains("sample_fixture::nested::NestedThing"), "{out}");
    assert!(!out.contains("sample_fixture::NestedThing"), "{out}");
}

#[test]
fn deep_optional_wrapper_return_serializes_as_jsvalue() {
    use crate::core::ir::NewtypeContainer::Optional;

    let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
    let mut func = async_function(vec![]);
    func.is_async = false;
    func.error_type = None;
    func.return_type = TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::String))));
    func.return_newtype_wrapper = Some(transparent_string_wrapper(vec![Optional, Optional]));

    let out = gen_function_with_emitted_dtos(
        &func,
        &mapper,
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &empty_surface(),
        &AHashSet::new(),
    );

    assert!(out.contains("-> JsValue"), "{out}");
    assert!(out.contains("serde_wasm_bindgen::to_value"), "{out}");
}

#[test]
fn nested_jsvalue_return_wrapping_recurses_through_optional_and_vec_containers() {
    let wrapped = wrap_jsvalue_mapped_return("result", "Option<Vec<Option<JsValue>>>")
        .expect("nested JsValue must require wrapping");

    assert!(
        wrapped.contains("result.map(|value| value.into_iter().map(|value| value.map(|value|"),
        "{wrapped}"
    );
    assert!(wrapped.contains("serde_wasm_bindgen::to_value"), "{wrapped}");
}
