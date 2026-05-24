use alef::backends::rustler::trait_bridge::gen_trait_bridge;
use alef::core::config::TraitBridgeConfig;
use alef::core::ir::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool, has_error: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: if has_error {
            Some("std::error::Error".to_string())
        } else {
            None
        },
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
    }
}

fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
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
    }
}

fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some("demo::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        unregister_fn: Some(format!("unregister_{}", trait_name.to_lowercase())),
        clear_fn: Some(format!("clear_{}_backends", trait_name.to_lowercase())),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

// ---------------------------------------------------------------------------
// Snapshot tests
// ---------------------------------------------------------------------------

#[test]
fn test_rustler_trait_bridge_snapshot_with_sync_method() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![make_method(
            "validate",
            vec![make_param("language", TypeRef::String)],
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            true,
        )],
    );
    let cfg = make_plugin_bridge_cfg("OcrBackend");
    let output = gen_trait_bridge(&trait_def, &cfg, "demo", "Error", "Error::new({msg})", &make_api());

    insta::assert_snapshot!("rustler_trait_bridge_sync_method", output.code);
}

#[test]
fn test_rustler_trait_bridge_snapshot_with_async_method() {
    let trait_def = make_trait_def(
        "DocumentExtractor",
        vec![make_method(
            "extract",
            vec![
                make_param("content", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            TypeRef::String,
            true,
            true,
        )],
    );
    let cfg = make_plugin_bridge_cfg("DocumentExtractor");
    let output = gen_trait_bridge(&trait_def, &cfg, "demo", "Error", "Error::new({msg})", &make_api());

    insta::assert_snapshot!("rustler_trait_bridge_async_method", output.code);
}

#[test]
fn test_rustler_trait_bridge_snapshot_with_multiple_methods() {
    let trait_def = make_trait_def(
        "Processor",
        vec![
            make_method("initialize", vec![], TypeRef::Unit, false, true),
            make_method(
                "process",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                true,
                true,
            ),
            make_method("shutdown", vec![], TypeRef::Unit, false, false),
        ],
    );
    let cfg = make_plugin_bridge_cfg("Processor");
    let output = gen_trait_bridge(&trait_def, &cfg, "demo", "Error", "Error::new({msg})", &make_api());

    insta::assert_snapshot!("rustler_trait_bridge_multiple_methods", output.code);
}
