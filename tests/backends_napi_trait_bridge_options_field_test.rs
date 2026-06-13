use alef::backends::napi::trait_bridge::gen_options_field_bridge_function;
use alef::codegen::generators::{AsyncPattern, RustBindingConfig};
use alef::codegen::type_mapper::IdentityMapper;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn options_field_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("visitor".to_string()),
        context_type: None,
        result_type: None,
    }
}

fn render_document_function() -> FunctionDef {
    FunctionDef {
        name: "render_document".to_string(),
        rust_path: "sample_core::render_document".to_string(),
        params: vec![ParamDef {
            name: "options".to_string(),
            ty: TypeRef::Named("RenderOptions".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::String,
        error_type: Some("SampleError".to_string()),
        ..FunctionDef::default()
    }
}

fn binding_config() -> RustBindingConfig<'static> {
    RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &[],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[napi]",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "sample_core",
        async_pattern: AsyncPattern::NapiNativeAsync,
        has_serde: true,
        type_name_prefix: "Js",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
    }
}

#[test]
fn options_field_bridge_body_injects_visitor_handle() {
    let code = gen_options_field_bridge_function(
        &ApiSurface::default(),
        &render_document_function(),
        0,
        &options_field_bridge_config(),
        &IdentityMapper,
        &binding_config(),
        &ahash::AHashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains(
            "let visitor_handle: Option<sample_core::VisitorHandle> = options.as_ref().and_then(|o| o.visitor.as_ref()).and_then(|v|"
        )
    );
    assert!(code.contains("let mut result: sample_core::RenderOptions = o.into();"));
    assert!(code.contains("result.visitor = visitor_handle.clone();"));
    assert!(code.contains("visitor: visitor_handle.clone(),"));
    assert!(code.contains("sample_core::render_document(options_core).map(|val| val.into())"));
}
