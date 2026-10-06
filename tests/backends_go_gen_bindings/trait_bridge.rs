use alef::backends::go::GoBackend;
use alef::backends::go::trait_bridge::gen_trait_bridges_file;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::*;

use super::{make_field, resolved_one};

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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
        cfg: None,
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
    config.replace_trait_bridges(bridge_configs);
    config
}

fn make_api_with_type(trait_type: TypeDef) -> ApiSurface {
    ApiSurface {
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
        unresolved_modules: Vec::new(),
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
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
    assert!(binding.contains("var cOptions C.KRZAlefHandle"));
    assert!(binding.contains("cOptions = C.krz_render_options_from_json(tmpStr)"));
    assert!(binding.contains("if cOptions == 0"));
    assert!(binding.contains("ptr := C.krz_render(cDocument, cOptions)"));
    assert!(binding.contains("defer C.krz_render_output_free(ptr)"));
    assert!(binding.contains("jsonPtr := C.krz_render_output_to_json(ptr)"));
    assert!(!binding.contains("convertWithVisitorHelper"));
    assert!(!binding.contains("HTMConversionOptions"));
    assert!(!binding.contains("ConversionResult"));
}

#[test]
fn test_gen_trait_bridges_file_produces_go_interface() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

    assert!(
        code.contains("type OcrBackend interface"),
        "should generate Go interface for the trait"
    );
}

#[test]
fn excluded_go_bridge_emits_no_trampoline_or_interface_reference() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let mut bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        register_fn: Some("register_ocr_backend".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    };
    let api = make_api_with_type(trait_type);

    let active = gen_trait_bridges_file(
        &api,
        &make_config_with_bridges(vec![bridge_cfg.clone()]),
        "testlib",
        "krz",
        "test.h",
        "../ffi",
        "..",
    );
    assert!(
        active.contains("goOcrBackendProcess") && active.contains("type OcrBackend interface"),
        "the control must exercise both the trampoline and interface emitters:\n{active}"
    );

    bridge_cfg.exclude_languages = vec!["go".to_string()];
    let excluded = gen_trait_bridges_file(
        &api,
        &make_config_with_bridges(vec![bridge_cfg]),
        "testlib",
        "krz",
        "test.h",
        "../ffi",
        "..",
    );
    assert!(
        !excluded.contains("OcrBackend") && !excluded.contains("goOcrBackendProcess"),
        "an excluded bridge must leave no Go reference behind:\n{excluded}"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_plugin_lifecycle_methods() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        exclude_functions: Vec::new(),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

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
        exclude_functions: Vec::new(),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

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
            make_trait_method("analyze", vec![], TypeRef::String, true),
            make_trait_method("ping", vec![], TypeRef::Unit, true),
        ],
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
        code.contains("(string, error)"),
        "method with non-unit return and error must produce (T, error) return type"
    );
    assert!(
        code.contains("Ping() error"),
        "method with unit return and error must produce 'error' return type"
    );
}

#[test]
fn test_gen_trait_bridges_file_generates_exported_trampolines() {
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
        code.contains("//export goOcrBackendProcess"),
        "trampoline for 'process' must be exported as goOcrBackendProcess"
    );
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
        code.contains("cResult := C.CString(callbackResult)"),
        "string callback returns must cross the FFI boundary as raw UTF-8, not JSON: {code}"
    );
    assert!(
        !code.contains("json.Marshal(callbackResult)\n\tcResult := C.CString(string(jsonBytes))"),
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
        exclude_functions: Vec::new(),
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

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..");

    assert!(
        code.contains("C.GoString(message)"),
        "trampoline must convert *C.char parameter to Go string via C.GoString"
    );
}

#[path = "trait_bridge/registration_tests.rs"]
mod registration_tests;

#[path = "trait_bridge/typed_params.rs"]
mod typed_params;

#[test]
fn test_gen_trait_bridges_file_trampolines_recover_host_panics() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![
            make_trait_method("process", vec![], TypeRef::String, true),
            make_trait_method(
                "count_tokens",
                vec![],
                TypeRef::Primitive(alef::core::ir::PrimitiveType::Usize),
                false,
            ),
        ],
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
        code.contains("defer func() {") && code.contains("if r := recover(); r != nil {"),
        "trampolines must recover host panics:\n{code}"
    );
    assert!(
        code.contains("host 'Process' panicked") && code.contains("*outError = C.CString(fmt.Sprint(r))"),
        "fallible trampoline must marshal the panic through outError:\n{code}"
    );
    assert!(
        code.contains("host 'CountTokens' panicked; returning default"),
        "infallible trampoline must log the panic and return the default:\n{code}"
    );
    assert!(
        code.contains("(ret C.int32_t)"),
        "fallible trampolines must use a named int32 status return:\n{code}"
    );
    assert!(
        code.contains("(ret C.uintptr_t)"),
        "usize trampolines must use a named primitive return:\n{code}"
    );
    assert!(
        code.contains("called with an invalid handle"),
        "invalid-handle paths must log:\n{code}"
    );
    assert!(
        code.contains("host 'Name' called with an invalid handle")
            && code.contains("*outError = C.CString(\"invalid handle\")"),
        "lifecycle invalid-handle paths must marshal outError:\n{code}"
    );
    assert!(
        code.contains("host 'Name' panicked"),
        "plugin Name trampoline must recover:\n{code}"
    );
}
