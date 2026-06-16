use alef::backends::ffi::FfiBackend;
use alef::backends::ffi::trait_bridge::gen_trait_bridge;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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

fn make_method(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
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
        has_default_impl: has_default,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_method_with_params(name: &str, params: Vec<ParamDef>, return_type: TypeRef, has_error: bool) -> MethodDef {
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
        version: Default::default(),
    }
}

fn make_param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref,
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

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

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
    }
}

fn make_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
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

// ---------------------------------------------------------------------------
// VTable struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_vtable_is_repr_c() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = make_bridge_cfg("OcrBackend");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(code.contains("#[repr(C)]"), "vtable struct must be #[repr(C)]");
    assert!(
        code.contains("KrOcrBackendVTable"),
        "vtable struct name must follow {{PascalPrefix}}{{TraitName}}VTable pattern"
    );
}

#[test]
fn test_gen_trait_bridge_vtable_has_function_pointer_fields_for_each_method() {
    let trait_def = make_trait_def(
        "Analyzer",
        vec![
            make_method("analyze", TypeRef::String, true, false),
            make_method("status", TypeRef::Primitive(PrimitiveType::I32), false, false),
        ],
    );
    let bridge_cfg = make_bridge_cfg("Analyzer");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "lib",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("pub analyze:"),
        "vtable must have function pointer field for 'analyze'"
    );
    assert!(
        code.contains("pub status:"),
        "vtable must have function pointer field for 'status'"
    );
    assert!(
        code.contains("pub free_user_data:"),
        "vtable must have free_user_data destructor field"
    );
}

#[test]
fn test_gen_trait_bridge_vtable_fn_ptrs_are_optional_extern_c() {
    let trait_def = make_trait_def("Scanner", vec![make_method("scan", TypeRef::Unit, false, false)]);
    let bridge_cfg = make_bridge_cfg("Scanner");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "lib",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("Option<unsafe extern \"C\" fn("),
        "vtable fn pointers must be Option<unsafe extern \"C\" fn(...)>"
    );
}

#[test]
fn test_gen_trait_bridge_vtable_fn_ptrs_take_user_data_first() {
    let trait_def = make_trait_def(
        "Checker",
        vec![make_method(
            "ping",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            false,
        )],
    );
    let bridge_cfg = make_bridge_cfg("Checker");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "lib",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("user_data: *const std::ffi::c_void"),
        "every vtable fn pointer must accept user_data as first parameter"
    );
}

#[test]
fn test_gen_trait_bridge_vtable_string_param_maps_to_c_char_ptr() {
    let trait_def = make_trait_def(
        "Greeter",
        vec![make_method_with_params(
            "greet",
            vec![make_param("message", TypeRef::String, true)],
            TypeRef::Unit,
            false,
        )],
    );
    let bridge_cfg = make_bridge_cfg("Greeter");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "g",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("*const std::ffi::c_char"),
        "string parameter must map to *const c_char in vtable function pointer"
    );
}

// ---------------------------------------------------------------------------
// Registration function naming convention
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_register_fn_name_follows_prefix_register_trait_snake_pattern() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    // Convention: {prefix}_register_{trait_snake}
    assert!(
        code.contains("extern \"C\" fn kr_register_ocr_backend"),
        "register fn must follow {{prefix}}_register_{{trait_snake}} naming convention"
    );
}

#[test]
fn test_gen_trait_bridge_unregister_fn_is_generated() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    // Convention: {prefix}_unregister_{trait_snake}
    assert!(
        code.contains("extern \"C\" fn kr_unregister_ocr_backend"),
        "unregister fn must be generated alongside register fn"
    );
    assert!(
        code.contains("#[unsafe(no_mangle)]"),
        "unregister fn must carry #[unsafe(no_mangle)]"
    );
}

#[test]
fn test_gen_trait_bridge_no_exported_registration_fn_when_not_configured() {
    let trait_def = make_trait_def(
        "HtmlVisitor",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    // No register_fn — vtable is still generated but no top-level registration function
    let bridge_cfg = make_bridge_cfg("HtmlVisitor");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "lib",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    // Without register_fn, no #[unsafe(no_mangle)] exported function is generated.
    // The vtable fields still contain `extern "C" fn` pointer *types*, but no
    // stand-alone `pub unsafe extern "C" fn lib_register_*` definition.
    assert!(
        !code.contains("#[unsafe(no_mangle)]"),
        "no #[unsafe(no_mangle)] exported function should be generated when register_fn is not configured"
    );
    assert!(
        !code.contains("pub unsafe extern \"C\" fn lib_"),
        "no exported registration function should be generated when register_fn is not configured"
    );
}

// ---------------------------------------------------------------------------
// Super-trait ("Plugin") support
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_with_super_trait_plugin_generates_vtable_lifecycle_fields() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    // When super_trait = "Plugin", the vtable must include Plugin lifecycle fn pointers
    assert!(
        code.contains("name_fn:"),
        "vtable must include name_fn for Plugin super-trait"
    );
    assert!(
        code.contains("version_fn:"),
        "vtable must include version_fn for Plugin super-trait"
    );
    assert!(
        code.contains("initialize_fn:"),
        "vtable must include initialize_fn for Plugin super-trait"
    );
    assert!(
        code.contains("shutdown_fn:"),
        "vtable must include shutdown_fn for Plugin super-trait"
    );
}

#[test]
fn test_gen_trait_bridge_with_super_trait_plugin_generates_plugin_impl() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl my_lib::Plugin for KrOcrBackendBridge"),
        "must generate Plugin impl for bridge struct"
    );
    assert!(code.contains("fn name(&self)"), "Plugin impl must contain name()");
    assert!(
        code.contains("fn initialize(&self)"),
        "Plugin impl must contain initialize()"
    );
    assert!(
        code.contains("fn shutdown(&self)"),
        "Plugin impl must contain shutdown()"
    );
}

// ---------------------------------------------------------------------------
// Bridge struct and safety
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_bridge_struct_holds_vtable_and_user_data() {
    let trait_def = make_trait_def("Runner", vec![make_method("run", TypeRef::Unit, true, false)]);
    let bridge_cfg = make_bridge_cfg("Runner");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "rn",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("vtable: RnRunnerVTable"),
        "bridge struct must hold its vtable"
    );
    assert!(
        code.contains("user_data: *const std::ffi::c_void"),
        "bridge struct must hold opaque user_data pointer"
    );
    assert!(
        code.contains("cached_name: String"),
        "bridge struct must cache the plugin name"
    );
}

#[test]
fn test_gen_trait_bridge_bridge_struct_is_send_sync() {
    let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
    let bridge_cfg = make_bridge_cfg("Worker");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "wk",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("unsafe impl Send for WkWorkerBridge"),
        "bridge must implement Send"
    );
    assert!(
        code.contains("unsafe impl Sync for WkWorkerBridge"),
        "bridge must implement Sync"
    );
}

#[test]
fn test_gen_trait_bridge_safety_comments_present() {
    let trait_def = make_trait_def("Processor", vec![make_method("run", TypeRef::String, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Processor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_processor".to_string()),

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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("// SAFETY:"),
        "generated code must contain SAFETY comments for all unsafe blocks"
    );
}

#[test]
fn test_gen_trait_bridge_drop_impl_calls_free_user_data() {
    let trait_def = make_trait_def("Plugin", vec![make_method("tick", TypeRef::Unit, false, false)]);
    let bridge_cfg = make_bridge_cfg("Plugin");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "p",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl Drop for PPluginBridge"),
        "bridge must implement Drop"
    );
    assert!(code.contains("free_user_data"), "Drop impl must invoke free_user_data");
}

// ---------------------------------------------------------------------------
// Trait impl generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_generates_trait_impl() {
    let trait_def = make_trait_def("Scanner", vec![make_method("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_bridge_cfg("Scanner");
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "sc",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl my_lib::Scanner for ScScannerBridge"),
        "must generate the trait impl for the bridge struct"
    );
    assert!(code.contains("fn scan("), "trait impl must include all trait methods");
}

#[test]
fn test_gen_trait_bridge_register_fn_validates_required_fn_ptrs() {
    let trait_def = make_trait_def(
        "Transform",
        vec![
            make_method("transform", TypeRef::String, true, false), // required
            make_method("describe", TypeRef::String, false, true),  // optional (has default)
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Transform".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_transform".to_string()),

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
    let api = make_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "Error",
        "Error::from({msg})",
        None,
        &api,
    );

    // Required method fn ptr must be null-checked; optional need not be
    assert!(
        code.contains("vtable_ref.transform.is_none()"),
        "register fn must validate required fn pointers are non-null"
    );
}

// ---------------------------------------------------------------------------
// build.rs Go header copy (regression: downstream go get compatibility)
// ---------------------------------------------------------------------------

#[test]
fn test_build_rs_contains_go_header_copy_when_go_is_configured() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.go]
module = "github.com/example/mylib"

[crates.output]
ffi = "crates/mylib-ffi/src/"
go = "packages/go/"
"#,
    );
    let api = make_empty_api();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let build_rs = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();

    assert!(
        build_rs.content.contains("go_include_dir"),
        "build.rs must contain go_include_dir when Go is configured"
    );
    assert!(
        build_rs.content.contains("std::fs::copy"),
        "build.rs must copy the header into the Go include dir"
    );
    assert!(
        build_rs.content.contains("packages/go/include"),
        "build.rs must target the correct Go include destination"
    );
}

#[test]
fn test_build_rs_has_no_go_copy_when_go_is_not_configured() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );
    let api = make_empty_api();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let build_rs = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();

    assert!(
        !build_rs.content.contains("go_include_dir"),
        "build.rs must not contain Go copy step when Go is not configured"
    );
    assert!(
        !build_rs.content.contains("std::fs::copy"),
        "build.rs must not copy header when Go is not configured"
    );
}

// ---------------------------------------------------------------------------
// _len() companion generation
// ---------------------------------------------------------------------------

/// A function returning `Option<&'static str>` (Optional<String> + returns_ref=true) must
/// emit both the primary `*mut c_char` wrapper AND a sibling `_len() -> usize` companion.
#[test]
fn test_c_char_returning_function_emits_len_companion() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "detect_language".to_string(),
            rust_path: "mylib::detect_language".to_string(),
            original_rust_path: String::new(),
            params: vec![make_param("extension", TypeRef::String, true)],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: true,
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

    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    // Primary function must be present and return *mut c_char
    assert!(
        code.contains("pub unsafe extern \"C\" fn ml_detect_language("),
        "primary ml_detect_language function must be emitted"
    );

    // _len companion must be present
    assert!(
        code.contains("pub unsafe extern \"C\" fn ml_detect_language_len("),
        "_len companion ml_detect_language_len must be emitted"
    );
    assert!(
        code.contains("static LAST_RETURN_LEN: RefCell<usize>"),
        "FFI module must record the primary C-string return length"
    );
    assert!(
        code.contains("set_last_return_len(cs.as_bytes().len());"),
        "primary C-string return must record its byte length before into_raw"
    );

    // Locate and inspect the _len companion body
    let len_fn_start = code
        .find("pub unsafe extern \"C\" fn ml_detect_language_len(")
        .expect("_len companion not found");
    let len_fn_end = code[len_fn_start..]
        .find("\n}")
        .map(|i| len_fn_start + i + 2)
        .unwrap_or(code.len().min(len_fn_start + 1200));
    let len_fn_snippet = &code[len_fn_start..len_fn_end];

    assert!(
        len_fn_snippet.contains("-> usize"),
        "_len companion signature must declare usize return type"
    );
    assert!(
        len_fn_snippet.contains("*const std::ffi::c_char"),
        "_len companion must accept the same *const c_char extension param"
    );
    // Body must not allocate a CString
    assert!(
        !len_fn_snippet.contains("CString::new"),
        "_len companion must not allocate a CString"
    );
    assert!(
        !len_fn_snippet.contains("mylib::detect_language"),
        "_len companion must not re-execute the wrapped Rust function"
    );
    assert!(
        !len_fn_snippet.contains("clear_last_error"),
        "_len companion must not clear the primary call's error state"
    );
    assert!(
        len_fn_snippet.contains("last_return_len()"),
        "_len companion must read the length recorded by the primary call"
    );
}

#[test]
fn test_error_and_clear_paths_reset_last_return_len() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    let api = make_empty_api();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    let set_error = code
        .split("fn set_last_error(code: i32, message: &str) {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("set_last_error body");
    assert!(
        set_error.contains("LAST_RETURN_LEN.with_borrow_mut(|c| *c = 0);"),
        "set_last_error must clear stale C-string success length: {set_error}"
    );

    let clear_error = code
        .split("fn clear_last_error() {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("clear_last_error body");
    assert!(
        clear_error.contains("LAST_RETURN_LEN.with_borrow_mut(|c| *c = 0);"),
        "clear_last_error must clear stale C-string success length: {clear_error}"
    );
}

// ---------------------------------------------------------------------------
// clippy::manual_unwrap_or regression
// ---------------------------------------------------------------------------

/// A function returning `Option<f64>` must emit `.unwrap_or(0.0)` instead of the
/// manual `match result { Some(val) => val, None => 0.0 }` pattern that clippy
/// flags as `manual_unwrap_or`.
#[test]
fn test_optional_primitive_return_emits_unwrap_or_not_manual_match() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    // `completion_cost(model: &str) -> Option<f64>` is the canonical example that
    // triggered clippy::manual_unwrap_or in sample-llm-ffi.
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "cost".to_string(),
            rust_path: "mylib::cost".to_string(),
            original_rust_path: String::new(),
            params: vec![make_param("model", TypeRef::String, true)],
            return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
            is_async: false,
            error_type: None,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    // The function must exist.
    assert!(
        code.contains("fn ml_cost("),
        "ml_cost function must be emitted; got:\n{code}"
    );

    // Must emit `.unwrap_or(0.0)` — the idiomatic form clippy expects.
    assert!(
        code.contains(".unwrap_or(0.0)"),
        "Option<f64> return must emit .unwrap_or(0.0), not a manual match; got:\n{code}"
    );

    // Must NOT emit the manual-match form that clippy::manual_unwrap_or flags.
    assert!(
        !code.contains("Some(val) => val") && !code.contains("Some(val) => {\n"),
        "must not emit manual Some(val) => val match for passthrough primitive; got:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Opaque type configured in [workspace.opaque_types] regression
// ---------------------------------------------------------------------------

/// A type declared in [workspace.opaque_types] must have its _new() and _free()
/// FFI functions emitted, even though it is not defined in the API surface.
/// This enables return values like `fn get_language() -> Language` to work.
/// Regression: alef 0.23.0 excluded all opaque_types from FFI emission, causing
/// cgo compilation to fail with "could not determine what C.language_free refers to".
#[test]
fn test_opaque_type_configured_in_workspace_emits_ffi_allocators() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]
opaque_types = { Language = "mylib::Language" }

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    // Language is an opaque type, so it appears in config but not in the API surface.
    // However, it may be returned by functions, so we need to emit the wrapper.
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "mylib::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
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
        }],
        functions: vec![FunctionDef {
            name: "get_language".to_string(),
            rust_path: "mylib::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![make_param("name", TypeRef::String, true)],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    // The function must exist and return the opaque type.
    assert!(
        code.contains("pub unsafe extern \"C\" fn ml_get_language("),
        "ml_get_language function must be emitted; got:\n{code}"
    );

    // Most importantly, the _free() function for the opaque type must be emitted.
    // Without this fix, the cgo compilation fails: "could not determine what C.language_free refers to".
    assert!(
        code.contains("pub unsafe extern \"C\" fn ml_language_free("),
        "opaque type Language must emit ml_language_free() for cgo compatibility; got:\n{code}"
    );
}

#[test]
fn test_opaque_type_filter_simple_newtype_not_excluded() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]
opaque_types = { Language = "mylib::Language" }

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    // Simple newtype opaque (no generics) should have free function emitted
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "mylib::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Simple opaque language type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    // Simple newtype opaques should have free function emitted
    assert!(
        code.contains("pub unsafe extern \"C\" fn ml_language_free("),
        "opaque type Language must emit ml_language_free() for simple newtype opaque (no generics in path)"
    );
}

#[test]
fn test_opaque_type_filter_generic_path_excluded() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]
opaque_types = { Handler = "std::sync::Arc<std::sync::Mutex<dyn mylib::Handler>>" }

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.output]
ffi = "crates/mylib-ffi/src/"
"#,
    );

    // Generic-path opaque should be excluded from FFI emission
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handler".to_string(),
            rust_path: "mylib::Handler".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Generic opaque handler type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let code = &lib_rs.content;

    // Generic-path opaques should NOT have free function (excluded)
    assert!(
        !code.contains("pub unsafe extern \"C\" fn ml_handler_free("),
        "opaque type Handler must NOT emit ml_handler_free() for generic-path opaque (contains '<')"
    );
}
