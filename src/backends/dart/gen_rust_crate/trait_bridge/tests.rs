use super::methods::emit_trait_bridge_method;
use super::*;
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn empty_type_def(name: &str, is_trait: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn api_surface(types: Vec<TypeDef>, excluded_paths: Vec<(&str, &str)>, excluded_traits: Vec<&str>) -> ApiSurface {
    ApiSurface {
        types,
        excluded_type_paths: excluded_paths
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        excluded_trait_names: excluded_traits.into_iter().map(String::from).collect(),
        services: vec![],
        handler_contracts: vec![],
        ..ApiSurface::default()
    }
}

#[test]
fn return_type_references_in_surface_trait() {
    let api = api_surface(vec![empty_type_def("MyTrait", true)], vec![], vec![]);
    let ret = TypeRef::Optional(Box::new(TypeRef::Named("MyTrait".into())));
    assert!(return_type_references_trait(&ret, &api));
}

#[test]
fn return_type_references_excluded_trait_is_detected() {
    let api = api_surface(
        vec![],
        vec![("SyncExtractor", "demo::extractors::SyncExtractor")],
        vec!["SyncExtractor"],
    );
    let ret = TypeRef::Optional(Box::new(TypeRef::Named("SyncExtractor".into())));
    assert!(return_type_references_trait(&ret, &api));
}

#[test]
fn return_type_with_excluded_struct_is_not_detected() {
    let api = api_surface(
        vec![],
        vec![("HiddenDocument", "demo::types::hidden::HiddenDocument")],
        vec![],
    );
    let ret = TypeRef::Named("HiddenDocument".into());
    assert!(!return_type_references_trait(&ret, &api));
}

#[test]
fn return_type_with_unrelated_named_is_not_detected() {
    let api = api_surface(vec![empty_type_def("MyStruct", false)], vec![], vec![]);
    let ret = TypeRef::Optional(Box::new(TypeRef::Named("MyStruct".into())));
    assert!(!return_type_references_trait(&ret, &api));
}

#[test]
fn excluded_named_result_return_deserializes_with_error_mapping() {
    let method = MethodDef {
        name: "extract".to_string(),
        params: vec![],
        return_type: TypeRef::Named("HiddenDocument".to_string()),
        is_async: true,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut out = String::new();
    let type_paths = std::collections::HashMap::from([(
        "HiddenDocument".to_string(),
        "demo::types::hidden::HiddenDocument".to_string(),
    )]);
    let excluded_type_paths = type_paths.clone();

    emit_trait_bridge_method(
        &mut out,
        &method,
        "DemoBridge",
        "demo",
        &type_paths,
        &excluded_type_paths,
        &std::collections::HashSet::new(),
    );

    assert!(
        out.contains("serde_json::from_str(&__ret_bridge.json)?;"),
        "Result-returning excluded types must propagate JSON decode errors, got:\n{out}",
    );
    assert!(
        !out.contains("expect(\"deserialize excluded Dart trait bridge value\")"),
        "Result-returning excluded types must not panic on JSON decode, got:\n{out}",
    );
}

#[test]
fn excluded_named_result_param_serializes_with_error_mapping() {
    let method = MethodDef {
        name: "render".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "document".to_string(),
            ty: TypeRef::Named("HiddenDocument".to_string()),
            optional: false,
            is_ref: true,
            ..Default::default()
        }],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mut out = String::new();
    let type_paths = std::collections::HashMap::from([(
        "HiddenDocument".to_string(),
        "demo::types::hidden::HiddenDocument".to_string(),
    )]);
    let excluded_type_paths = type_paths.clone();

    emit_trait_bridge_method(
        &mut out,
        &method,
        "DemoBridge",
        "demo",
        &type_paths,
        &excluded_type_paths,
        &std::collections::HashSet::new(),
    );

    assert!(
        out.contains("serde_json::to_string(&document)?"),
        "Result-returning excluded params must propagate JSON encode errors, got:\n{out}",
    );
    assert!(
        !out.contains("expect(\"serialize excluded Dart trait bridge value\")"),
        "Result-returning excluded params must not panic on JSON encode, got:\n{out}",
    );
}
