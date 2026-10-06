//! Go trait-bridge registration, vtable and panic-recovery coverage, split from trait_bridge.rs.

use super::{make_api_with_type, make_config_with_bridges, make_trait_method, make_trait_type};
use alef::backends::go::GoBackend;
use alef::backends::go::trait_bridge::gen_trait_bridges_file;
use alef::core::backend::Backend;
use alef::core::config::TraitBridgeConfig;
use alef::core::ir::*;

#[test]
fn test_gen_trait_bridges_file_registration_fn_builds_vtable_and_calls_c_register() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
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
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

    assert!(
        code.contains("func RegisterOcrBackend(impl OcrBackend) error"),
        "registration function must have the correct Go signature"
    );
    assert!(
        code.contains("bridge := NewOcrBackendBridge(impl)") && code.contains("cgo.NewHandle(bridge)"),
        "registration must create a cgo.Handle for the Go bridge wrapper"
    );
    assert!(
        code.contains("C.krz_register_ocr_backend("),
        "registration must call the C FFI register function with correct name format"
    );
    assert!(
        code.contains("func UnregisterOcrBackend(name string) error"),
        "unregistration function must also be generated"
    );
    assert!(
        code.contains("C.krz_unregister_ocr_backend("),
        "unregistration must call the C FFI unregister function with correct name format"
    );
}

#[test]
fn test_gen_trait_bridges_file_registration_fn_handles_c_error_response() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),

        unregister_fn: None,

        clear_fn: None,
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
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

    assert!(
        code.contains("if rc != 0"),
        "registration must check the C return code for errors"
    );
    assert!(
        code.contains("fmt.Errorf"),
        "registration must return a Go error on C failure"
    );
    assert!(
        code.contains("handle.Delete()"),
        "registration must delete the cgo.Handle on failure to avoid leaking"
    );
    assert!(
        code.contains("C.krz_free_string(cErr)"),
        "registration/unregistration must free Rust-allocated error strings with the generated FFI free function"
    );
    assert!(
        code.contains("if old, ok := reg.handles[name]; ok {\n\t\told.Delete()\n\t}"),
        "handle registry must delete any replaced handle on duplicate registration"
    );
}

#[test]
fn test_gen_trait_bridges_file_uses_correct_vtable_struct_name() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
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
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "sample_crate", "test.h", "../ffi", "..");

    assert!(
        code.contains("static inline SAMPLE_CRATESampleCrateOcrBackendVTable* sample_crate_ocr_backend_vtable_new("),
        "must use correct cbindgen-generated VTable struct name format: {{CRATE_UPPER}}{{CratePascal}}{{TraitPascal}}VTable"
    );
    assert!(
        code.contains("vtable := C.sample_crate_ocr_backend_vtable_new("),
        "registration must allocate the VTable through the ffi-prefixed C helper"
    );
}

#[test]
fn test_gen_trait_bridges_file_cgo_preamble_forward_declares_trampolines() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![make_trait_method("analyze", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "Analyzer".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),

        unregister_fn: None,

        clear_fn: None,
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
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

    assert!(
        code.contains("extern int32_t goAnalyzerAnalyze("),
        "CGo preamble must forward-declare the analyze trampoline"
    );
    assert!(
        code.contains("import \"C\""),
        "must import C after the CGo preamble block"
    );
}

#[test]
fn test_generate_bindings_with_trait_bridge_emits_trait_bridges_go_file() {
    let backend = GoBackend;

    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
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
    };

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let result = backend.generate_bindings(&api, &config);

    assert!(
        result.is_ok(),
        "generate_bindings must succeed with trait_bridges configured"
    );
    let files = result.unwrap();

    let bridge_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("trait_bridges.go"));
    assert!(
        bridge_file.is_some(),
        "generate_bindings should emit trait_bridges.go when trait_bridges are configured"
    );

    let content = &bridge_file.unwrap().content;
    assert!(
        content.contains("type OcrBackend interface"),
        "trait_bridges.go must contain the Go interface"
    );
    assert!(
        content.contains("func RegisterOcrBackend"),
        "trait_bridges.go must contain the registration function"
    );
}
