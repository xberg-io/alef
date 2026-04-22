use alef_backend_ffi::trait_bridge::gen_trait_bridge;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::*;

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
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
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
    }
}

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "kr", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "lib", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "lib", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "lib", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "g", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "kr", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "kr", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "lib", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "kr", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "kr", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "rn", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "wk", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "ml", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "p", "my_lib", "Error", "Error::from({msg})", &api);

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

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "sc", "my_lib", "Error", "Error::from({msg})", &api);

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
        type_alias: None,
        param_name: None,
    };
    let api = make_api();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "ml", "my_lib", "Error", "Error::from({msg})", &api);

    // Required method fn ptr must be null-checked; optional need not be
    assert!(
        code.contains("vtable.transform.is_none()"),
        "register fn must validate required fn pointers are non-null"
    );
}
