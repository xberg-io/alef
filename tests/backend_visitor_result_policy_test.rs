use alef::core::config::TraitBridgeConfig;
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
    }
}

fn method() -> MethodDef {
    MethodDef {
        name: "inspect".to_string(),
        params: vec![param("context", TypeRef::Named("VisitContext".to_string()))],
        return_type: TypeRef::Named("FlowDecision".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn type_def(name: &str, rust_path: &str, is_trait: bool, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    }
}

fn result_enum() -> EnumDef {
    EnumDef {
        name: "FlowDecision".to_string(),
        rust_path: "my_lib::visitor::FlowDecision".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Proceed".to_string(),
                is_default: true,
                serde_rename: Some("go_on".to_string()),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "DropNode".to_string(),
                serde_rename: Some("drop_node".to_string()),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "ReplaceWith".to_string(),
                fields: vec![field("value", TypeRef::String)],
                serde_rename: Some("swap".to_string()),
                is_tuple: true,
                ..EnumVariant::default()
            },
        ],
        doc: String::new(),
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            type_def("VisitContext", "my_lib::observer::VisitContext", false, vec![]),
            type_def("TreeObserver", "my_lib::observer::TreeObserver", true, vec![method()]),
        ],
        functions: vec![],
        enums: vec![result_enum()],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
}
}

fn bridge_cfg() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "TreeObserver".to_string(),
        type_alias: Some("TreeObserverHandle".to_string()),
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("observer".to_string()),
        context_type: Some("VisitContext".to_string()),
        result_type: Some("FlowDecision".to_string()),
        ..TraitBridgeConfig::default()
    }
}

fn assert_metadata_driven(code: &str) {
    assert!(code.contains("::Proceed"), "default fallback must use Proceed:\n{code}");
    assert!(
        code.contains("\"go_on\""),
        "unit wire name must use serde rename:\n{code}"
    );
    assert!(!code.contains("::Continue"), "must not hardcode Continue:\n{code}");
    assert!(
        !code.contains("\"continue\""),
        "must not hardcode continue wire string:\n{code}"
    );
}

fn assert_payload_wire_name(code: &str) {
    assert!(
        code.contains("\"swap\""),
        "payload wire name must use serde rename:\n{code}"
    );
}

#[test]
fn visitor_result_policy_is_metadata_driven_for_napi_wasm_pyo3_magnus_extendr_and_rustler() {
    let api = api();
    let trait_def = api.types.iter().find(|typ| typ.name == "TreeObserver").unwrap();
    let bridge_cfg = bridge_cfg();

    let napi = alef::backends::napi::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&napi.code);
    assert_payload_wire_name(&napi.code);

    let wasm = alef::backends::wasm::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&wasm.code);
    assert_payload_wire_name(&wasm.code);

    let pyo3 = alef::backends::pyo3::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&pyo3.code);
    assert_payload_wire_name(&pyo3.code);

    let magnus = alef::backends::magnus::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&magnus);
    assert_payload_wire_name(&magnus);

    let extendr = alef::backends::extendr::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&extendr.code);
    assert_payload_wire_name(&extendr.code);

    let rustler = alef::backends::rustler::trait_bridge::gen_trait_bridge(
        trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
    );
    assert_metadata_driven(&rustler.code);
}
