use alef::backends::go::GoBackend;
use alef::backends::go::trait_bridge::gen_trait_bridges_file;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::*;

use super::{make_field, resolved_one};

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_trait_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, has_error: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: format!("{name} method."),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        trait_source: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_trait_param(name: &str, ty: TypeRef) -> ParamDef {
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
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_config_with_bridges(bridge_configs: Vec<TraitBridgeConfig>) -> ResolvedCrateConfig {
    let mut config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "krz"
visitor_callbacks = true

[crates.go]
module = "github.com/test/test-lib"
"#,
    );
    config.trait_bridges = bridge_configs;
    config
}

fn make_api_with_type(trait_type: TypeDef) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_options_field_visitor_wrapper_uses_bridge_config_not_convert_names() {
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        type_alias: Some("RendererHandle".to_string()),
        param_name: Some("renderer".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        ..TraitBridgeConfig::default()
    };
    let mut config = make_config_with_bridges(vec![bridge_cfg]);
    config.go.as_mut().unwrap().functional_options = vec![];

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Renderer".to_string(),
                rust_path: "my_lib::Renderer".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![make_trait_method(
                    "visit_text",
                    vec![make_trait_param("text", TypeRef::String)],
                    TypeRef::Unit,
                    false,
                )],
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
                version: Default::default(),
            },
            TypeDef {
                name: "RenderOptions".to_string(),
                rust_path: "my_lib::RenderOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("renderer", TypeRef::Named("RendererHandle".to_string()), true),
                    make_field("visitor", TypeRef::Named("AuditVisitor".to_string()), true),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "RenderOutput".to_string(),
                rust_path: "my_lib::RenderOutput".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("html", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "my_lib::render".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_trait_param("document", TypeRef::String),
                ParamDef {
                    optional: true,
                    ..make_trait_param(
                        "settings",
                        TypeRef::Optional(Box::new(TypeRef::Named("RenderOptions".to_string()))),
                    )
                },
            ],
            return_type: TypeRef::Named("RenderOutput".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Render a document.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go must be generated")
        .content
        .as_str();

    assert!(binding.contains("func Render(document string, settings *RenderOptions) (*RenderOutput, error)"));
    assert!(binding.contains("Renderer Visitor `json:\"-\"`"));
    assert!(binding.contains("Visitor *json.RawMessage `json:\"visitor,omitempty\"`"));
    assert!(binding.contains("if settings != nil && settings.Renderer != nil"));
    assert!(binding.contains("return renderWithVisitorHelper(document, settings, settings.Renderer)"));
    assert!(binding.contains("var cOptions *C.KRZRenderOptions"));
    assert!(binding.contains("cOptions = C.krz_render_options_from_json(tmpStr)"));
    assert!(binding.contains("ptr := C.krz_render(cDocument, cOptions)"));
    assert!(binding.contains("defer C.krz_render_output_free(ptr)"));
    assert!(binding.contains("jsonPtr := C.krz_render_output_to_json(ptr)"));
    assert!(!binding.contains("convertWithVisitorHelper"));
    assert!(!binding.contains("HTMConversionOptions"));
    assert!(!binding.contains("ConversionResult"));
}

// ---------------------------------------------------------------------------
// Go interface generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_produces_go_interface() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_ocr_registry".to_string()),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("type OcrBackend interface"),
        "should generate Go interface for the trait"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_plugin_lifecycle_methods() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_scanner_registry".to_string()),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Plugin lifecycle methods must always be present in the interface
    assert!(
        code.contains("Name() string"),
        "Go interface must include Name() string"
    );
    assert!(
        code.contains("Version() string"),
        "Go interface must include Version() string"
    );
    assert!(
        code.contains("Initialize() error"),
        "Go interface must include Initialize() error"
    );
    assert!(
        code.contains("Shutdown() error"),
        "Go interface must include Shutdown() error"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_trait_methods_in_pascal_case() {
    let trait_type = make_trait_type(
        "ImageProcessor",
        vec![
            make_trait_method("process_image", vec![], TypeRef::String, true),
            make_trait_method(
                "get_format",
                vec![make_trait_param("path", TypeRef::String)],
                TypeRef::String,
                false,
            ),
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "ImageProcessor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_image_processor".to_string()),

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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("ProcessImage("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
    assert!(
        code.contains("GetFormat("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_method_with_error_returns_tuple_or_error() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![
            make_trait_method("analyze", vec![], TypeRef::String, true), // (string, error)
            make_trait_method("ping", vec![], TypeRef::Unit, true),      // error
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("(string, error)"),
        "method with non-unit return and error must produce (T, error) return type"
    );
    // Unit return with error: just "error"
    assert!(
        code.contains("Ping() error") || code.contains("Ping()"),
        "method with unit return and error must produce 'error' return type"
    );
}

// ---------------------------------------------------------------------------
// Trampoline generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_generates_exported_trampolines() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Each trait method must have a //export trampoline
    assert!(
        code.contains("//export goOcrBackendProcess"),
        "trampoline for 'process' must be exported as goOcrBackendProcess"
    );
    // Plugin lifecycle trampolines
    assert!(
        code.contains("//export goOcrBackendName"),
        "plugin Name trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendInitialize"),
        "plugin Initialize trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendFreeUserData"),
        "free_user_data trampoline must be exported"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampolines_retrieve_go_object_via_cgo_handle() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("cgo.Handle(uintptr(unsafe.Pointer(userData)))"),
        "trampolines must retrieve the Go object via cgo.Handle from userData"
    );
    assert!(
        code.contains("runtime/cgo"),
        "must import runtime/cgo for cgo.Handle support"
    );
}

#[test]
fn test_trait_bridge_string_return_is_not_json_quoted() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("cResult := C.CString(result)"),
        "string callback returns must cross the FFI boundary as raw UTF-8, not JSON: {code}"
    );
    assert!(
        !code.contains("json.Marshal(result)\n\tcResult := C.CString(string(jsonBytes))"),
        "string callback return must not be JSON-quoted before Rust decodes it: {code}"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampoline_converts_string_param_from_c() {
    let trait_type = make_trait_type(
        "Greeter",
        vec![make_trait_method(
            "greet",
            vec![make_trait_param("message", TypeRef::String)],
            TypeRef::Unit,
            false,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_greeter".to_string()),

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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("C.GoString(message)"),
        "trampoline must convert *C.char parameter to Go string via C.GoString"
    );
}

// ---------------------------------------------------------------------------
// Registration function
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_registration_fn_builds_vtable_and_calls_c_register() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

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

// ---------------------------------------------------------------------------
// VTable struct name derivation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_uses_correct_vtable_struct_name() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    // With crate_name="sample_crate", the VTable struct should be SAMPLE_CRATESampleCrateOcrBackendVTable
    let code = gen_trait_bridges_file(
        &api,
        &config,
        "testlib",
        "sample_crate",
        "test.h",
        "../ffi",
        "..",
        "sample_crate",
    );

    assert!(
        code.contains("static inline SAMPLE_CRATESampleCrateOcrBackendVTable* sample_crate_ocr_backend_vtable_new("),
        "must use correct cbindgen-generated VTable struct name format: {{CRATE_UPPER}}{{CratePascal}}{{TraitPascal}}VTable"
    );
    assert!(
        code.contains("vtable := C.sample_crate_ocr_backend_vtable_new("),
        "registration must allocate the VTable through the ffi-prefixed C helper"
    );
}

// ---------------------------------------------------------------------------
// CGo preamble
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_cgo_preamble_forward_declares_trampolines() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![make_trait_method("analyze", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // CGo preamble must forward-declare all exported Go functions
    assert!(
        code.contains("extern int32_t goAnalyzerAnalyze("),
        "CGo preamble must forward-declare the analyze trampoline"
    );
    assert!(
        code.contains("import \"C\""),
        "must import C after the CGo preamble block"
    );
}

// ---------------------------------------------------------------------------
// via generate_bindings (end-to-end)
// ---------------------------------------------------------------------------

#[test]
fn test_generate_bindings_with_trait_bridge_emits_trait_bridges_go_file() {
    let backend = GoBackend;

    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
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
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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

#[path = "trait_bridge/typed_params.rs"]
mod typed_params;
