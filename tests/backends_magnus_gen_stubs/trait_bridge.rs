//! Trait-bridge RBS generation coverage: registry functions, typed host interfaces, and
//! optional-method documentation for the Ruby/Magnus backend.
//!
//! Split out of `backends_magnus_gen_stubs_test.rs` (see `file-modularization` in CLAUDE.md).

use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::ir::*;

use super::{make_config_with_stubs, make_field};

#[test]
fn test_rbs_includes_trait_registry_functions() {
    let backend = MagnusBackend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }];
    // The bridged trait has to be in the surface for the RBS to declare its registry functions —
    // `gen_module_init` binds none for a bridge whose trait does not resolve. It is declared with
    // no methods so no `interface _OcrBackend` is emitted and the `backend` param stays `untyped`,
    // which is what this test pins. ~keep
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "test_lib::OcrBackend".to_string(),
            is_trait: true,
            is_opaque: true,
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("def self.register_ocr_backend: (untyped backend, String name) -> nil")
            && content.contains("def self.unregister_ocr_backend: (String name) -> nil")
            && content.contains("def self.clear_ocr_backends: () -> nil"),
        "RBS must include trait bridge registry functions:\n{content}"
    );
}

#[test]
fn test_rbs_plugin_bridge_emits_typed_interface_and_typed_register() {
    let backend = MagnusBackend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    }];

    let greeter = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![MethodDef {
            name: "process".to_string(),
            params: vec![ParamDef {
                name: "opts".to_string(),
                ty: TypeRef::Named("Opts".to_string()),
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
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("Doc".to_string()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            error_type: Some("Error".to_string()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let opts = TypeDef {
        name: "Opts".to_string(),
        rust_path: "test_lib::Opts".to_string(),
        has_serde: true,
        fields: vec![make_field("label", TypeRef::String, false)],
        ..TypeDef::default()
    };
    let doc = TypeDef {
        name: "Doc".to_string(),
        rust_path: "test_lib::Doc".to_string(),
        has_serde: true,
        is_return_type: true,
        fields: vec![make_field("text", TypeRef::String, false)],
        ..TypeDef::default()
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![greeter, opts, doc],
        ..Default::default()
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("interface _Greeter"),
        "plugin bridge must emit a host-implementable RBS interface:\n{content}"
    );
    assert!(
        content.contains("def process: (Opts opts) -> Doc"),
        "interface method must type the struct param as `Opts` and return as `Doc`:\n{content}"
    );

    assert!(
        content.contains("def self.register_greeter: (_Greeter backend, String name) -> nil"),
        "register fn must type its backend param against the interface:\n{content}"
    );
}

#[test]
fn test_rbs_plugin_interface_omits_defaulted_methods_and_documents_them() {
    let backend = MagnusBackend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    }];

    let greeter = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        is_opaque: true,
        methods: vec![
            MethodDef {
                name: "process".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..Default::default()
            },
            MethodDef {
                name: "supports_table_detection".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                has_default_impl: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![greeter],
        ..Default::default()
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("def process:"),
        "required method must stay in the interface:\n{content}"
    );
    assert!(
        !content.contains("def supports_table_detection:"),
        "Rust-defaulted method must not be a required interface member:\n{content}"
    );
    assert!(
        content.contains("Optional methods") && content.contains("supports_table_detection"),
        "defaulted method must be documented as optional:\n{content}"
    );
}
