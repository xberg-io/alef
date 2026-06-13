use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, TypeRef};

#[test]
fn visitor_bridge_uses_configured_context_and_result_metadata() {
    let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
    let output = crate::backends::rustler::trait_bridge::gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
    )
    .expect("visitor bridge should generate");

    crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
    assert!(output.code.contains("\"display_name\""));
}

#[test]
fn options_field_bridge_renders_visitor_setup_template() {
    let api = ApiSurface {
        crate_name: "sample".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
        ..Default::default()
    };
    let func = FunctionDef {
        name: "render".to_string(),
        rust_path: "sample::render".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "html".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("RenderOptions".to_string()),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::String,
        is_async: false,
        error_type: Some("RenderError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let field = FieldDef {
        name: "renderer".to_string(),
        ty: TypeRef::Named("RenderVisitorHandle".to_string()),
        ..FieldDef::default()
    };
    let bridge = TraitBridgeConfig {
        trait_name: "RenderVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some("RenderVisitorHandle".to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: vec![],
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        context_type: None,
        result_type: None,
    };
    let bridge_match = crate::codegen::generators::trait_bridge::BridgeFieldMatch {
        param_index: 1,
        param_name: "options".to_string(),
        options_type: "RenderOptions".to_string(),
        param_is_optional: false,
        field_name: "renderer".to_string(),
        field: &field,
        bridge: &bridge,
    };
    let code = crate::backends::rustler::trait_bridge::gen_bridge_field_function(
        &api,
        &func,
        &bridge_match,
        &bridge,
        &crate::backends::rustler::type_map::RustlerMapper,
        &ahash::AHashSet::new(),
        &ahash::AHashSet::new(),
        "sample",
    );

    let expected_options_setup = concat!(
        "let mut options_core: sample::options::RenderOptions = options.map(|s| ",
        "serde_json::from_str::<sample::options::RenderOptions>(&s).unwrap_or_default())",
        ".unwrap_or_default();"
    );
    assert!(code.contains(expected_options_setup));
    assert!(code.contains("let bridge = ElixirRenderVisitorBridge::new(env, pid, visitor_term);"));
    assert!(code.contains(
            "options_core.renderer = Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as sample::RenderVisitorHandle);"
        ));
}
