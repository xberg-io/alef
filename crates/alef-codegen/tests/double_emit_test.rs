//! Test that demonstrates the fix for trait-bridge double-emission bug.
//! When collect_trait_bridge_registration_fn_names returns the set of registration function names,
//! backends should exclude them from gen_function to prevent compile errors.

use alef_codegen::generators::trait_bridge::collect_trait_bridge_registration_fn_names;
use alef_core::config::{BridgeBinding, TraitBridgeConfig};

#[test]
fn test_double_emit_fix_collects_all_registration_fns() {
    // Simulate the trait bridges configured in kreuzberg's alef.toml
    let bridges = vec![
        TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("get_ocr_backend_registry".to_string()),
            register_fn: Some("register_ocr_backend".to_string()),
            type_alias: None,
            param_name: Some("backend".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
        TraitBridgeConfig {
            trait_name: "PostProcessor".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("get_post_processor_registry".to_string()),
            register_fn: Some("register_post_processor".to_string()),
            type_alias: None,
            param_name: Some("processor".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
        TraitBridgeConfig {
            trait_name: "Validator".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("get_validator_registry".to_string()),
            register_fn: Some("register_validator".to_string()),
            type_alias: None,
            param_name: Some("validator".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
        TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("get_embedding_backend_registry".to_string()),
            register_fn: Some("register_embedding_backend".to_string()),
            type_alias: None,
            param_name: Some("backend".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
    ];

    let reg_fns = collect_trait_bridge_registration_fn_names(&bridges);

    // All 4 trait bridges have register_fn configured, so all 4 should be collected
    assert_eq!(reg_fns.len(), 4, "should collect all 4 registration functions");

    // Verify each expected registration function name is in the set
    assert!(reg_fns.contains("register_ocr_backend"), "missing register_ocr_backend");
    assert!(
        reg_fns.contains("register_post_processor"),
        "missing register_post_processor"
    );
    assert!(reg_fns.contains("register_validator"), "missing register_validator");
    assert!(
        reg_fns.contains("register_embedding_backend"),
        "missing register_embedding_backend"
    );
}

#[test]
fn test_double_emit_fix_handles_mixed_bridges() {
    // Some bridges have register_fn, some don't (per-call bridge pattern)
    let bridges = vec![
        TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: Some("register_ocr_backend".to_string()),
            type_alias: None,
            param_name: Some("backend".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
        TraitBridgeConfig {
            trait_name: "SomeOtherTrait".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None, // Per-call bridge pattern: no registration function
            type_alias: None,
            param_name: Some("trait_obj".to_string()),
            register_extra_args: None,
            bind_via: BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            exclude_languages: vec![],
        },
    ];

    let reg_fns = collect_trait_bridge_registration_fn_names(&bridges);

    // Only 1 bridge has register_fn configured
    assert_eq!(reg_fns.len(), 1, "should collect only 1 registration function");
    assert!(reg_fns.contains("register_ocr_backend"));
    assert!(!reg_fns.contains("register_some_other_trait"));
}
