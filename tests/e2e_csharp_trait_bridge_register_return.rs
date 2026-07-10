//! Test that C# trait bridge Register methods return IntPtr for e2e test stub assignments.
//!
//! This tests the fix for an e2e C# test compilation failure:
//! `var result = RendererBridge.Register(new TestStub_...)` was invalid when Register
//! returned void. Now Register returns IntPtr and is assignable to a variable.

use alef::backends::csharp::trait_bridge::gen_trait_bridges_file;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::TypeDef;
use std::collections::HashSet;

fn make_trait_def(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{}", name),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
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
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        param_name: None,
        type_alias: None,
        exclude_languages: vec![],
        super_trait: super_trait.map(|s| s.to_string()),
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        register_extra_args: None,
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

#[test]
fn test_renderer_bridge_register_returns_intptr() {
    let trait_def = make_trait_def("Renderer");
    let bridge_cfg = make_bridge_cfg("Renderer", Some("Plugin"));
    let bridges = vec![("Renderer".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["Renderer"].into_iter().collect();

    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static IntPtr Register(IRenderer impl)"));
    assert!(!content.contains("public static void Register(IRenderer impl)"));

    assert!(content.contains("return userData;"));
}

#[test]
fn test_validator_bridge_register_returns_intptr() {
    let trait_def = make_trait_def("Validator");
    let bridge_cfg = make_bridge_cfg("Validator", Some("Plugin"));
    let bridges = vec![("Validator".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["Validator"].into_iter().collect();

    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static IntPtr Register(IValidator impl)"));
    assert!(!content.contains("public static void Register(IValidator impl)"));

    assert!(content.contains("return userData;"));
}

#[test]
fn test_ocr_backend_bridge_register_returns_intptr() {
    let trait_def = make_trait_def("OcrBackend");
    let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
    let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["OcrBackend"].into_iter().collect();

    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static IntPtr Register(IOcrBackend impl)"));
    assert!(!content.contains("public static void Register(IOcrBackend impl)"));

    assert!(content.contains("return userData;"));
}

#[test]
fn test_register_without_super_trait_also_returns_intptr() {
    let trait_def = make_trait_def("PostProcessor");
    let bridge_cfg = make_bridge_cfg("PostProcessor", None);
    let bridges = vec![("PostProcessor".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["PostProcessor"].into_iter().collect();

    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static IntPtr Register(IPostProcessor impl, string name)"));
    assert!(!content.contains("public static void Register(IPostProcessor impl, string name)"));

    assert!(content.contains("return userData;"));
}
