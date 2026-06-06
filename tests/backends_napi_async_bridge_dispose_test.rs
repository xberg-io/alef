//! Test that async trait bridges expose a dispose() method for explicit cleanup.
//!
//! Block B11: async trait bridge tests must await bridge cleanup to avoid
//! tokio task leaks at process shutdown.

use alef::backends::napi::trait_bridge::gen_trait_bridge;
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn fixture_async_trait() -> (TypeDef, alef::core::config::TraitBridgeConfig) {
    let trait_type = TypeDef {
        name: "Foo".to_string(),
        rust_path: "sample_core::Foo".to_string(),
        original_rust_path: String::new(),
        methods: vec![MethodDef {
            name: "bar".to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::I32),
            is_async: true,
            is_static: false,
            error_type: None,
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
        }],
        fields: vec![],
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    let bridge = alef::core::config::TraitBridgeConfig {
        trait_name: "Foo".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("FooRegistry::get()".to_string()),
        register_fn: Some("register_foo".to_string()),
        unregister_fn: Some("unregister_foo".to_string()),
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: vec![],
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };

    (trait_type, bridge)
}

#[test]
fn async_trait_bridge_exposes_dispose_method() {
    let (trait_type, bridge) = fixture_async_trait();
    let api = ApiSurface::default();

    let output = gen_trait_bridge(
        &trait_type,
        &bridge,
        "sample_core",
        "SampleError",
        "SampleError::Message { message: {msg} }",
        &api,
    )
    .expect("bridge generates");

    // Verify the struct includes cancellation_token field
    assert!(
        output
            .code
            .contains("cancellation_token: Arc<tokio_util::sync::CancellationToken>"),
        "bridge struct should include cancellation_token field"
    );

    // Verify the dispose() method exists
    assert!(
        output.code.contains("pub fn dispose(&self)"),
        "bridge should expose a dispose() method"
    );

    // Verify dispose returns a Promise<()>
    assert!(
        output
            .code
            .contains("-> napi::Result<napi::bindgen_prelude::Promise<()>>"),
        "dispose() should return Promise<()>"
    );

    // Verify dispose signals cancellation
    assert!(
        output.code.contains("token.cancel()"),
        "dispose() should call token.cancel()"
    );

    // Verify the custom struct is emitted
    assert!(output.code.contains("use std::sync::Arc"), "should include Arc import");
}
