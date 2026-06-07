use super::*;

#[test]
fn test_vtable_struct_is_repr_c() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = sample_bridge_cfg("OcrBackend");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(code.contains("#[repr(C)]"), "vtable must be #[repr(C)]");
    assert!(
        code.contains("MlOcrBackendVTable"),
        "vtable name must include prefix + trait name"
    );
}

#[test]
fn test_vtable_has_method_fn_ptrs() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![
            make_method("process", TypeRef::String, true, false),
            make_method("status", TypeRef::Primitive(PrimitiveType::I32), false, true),
        ],
    );
    let bridge_cfg = sample_bridge_cfg("OcrBackend");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(code.contains("pub process:"), "vtable must have fn ptr for 'process'");
    assert!(code.contains("pub status:"), "vtable must have fn ptr for 'status'");
    assert!(
        code.contains("pub free_user_data:"),
        "vtable must have free_user_data destructor"
    );
}

#[test]
fn test_vtable_fn_ptrs_take_user_data() {
    let trait_def = make_trait_def(
        "Checker",
        vec![make_method(
            "ping",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            false,
        )],
    );
    let bridge_cfg = sample_bridge_cfg("Checker");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "lib",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("user_data: *const std::ffi::c_void"),
        "every vtable fn pointer must accept user_data as first param"
    );
}

#[test]
fn test_bridge_struct_fields() {
    let trait_def = make_trait_def("Runner", vec![make_method("run", TypeRef::Unit, true, false)]);
    let bridge_cfg = sample_bridge_cfg("Runner");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "my_lib",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(code.contains("vtable: MyLibRunnerVTable"), "bridge must hold vtable");
    assert!(
        code.contains("user_data: *const std::ffi::c_void"),
        "bridge must hold user_data"
    );
    assert!(code.contains("cached_name: String"), "bridge must hold cached_name");
}

#[test]
fn test_bridge_is_send_sync() {
    let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
    let bridge_cfg = sample_bridge_cfg("Worker");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "w",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("unsafe impl Send for WWorkerBridge"),
        "bridge must be Send"
    );
    assert!(
        code.contains("unsafe impl Sync for WWorkerBridge"),
        "bridge must be Sync"
    );
}

#[test]
fn test_bridge_has_drop_impl_for_free_user_data() {
    let trait_def = make_trait_def("Plugin", vec![make_method("tick", TypeRef::Unit, false, false)]);
    let bridge_cfg = sample_bridge_cfg("Plugin");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "p",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl Drop for PPluginBridge"),
        "bridge must implement Drop"
    );
    assert!(code.contains("free_user_data"), "Drop impl must call free_user_data");
}

#[test]
fn test_super_trait_generates_plugin_impl() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "sample_crate",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl sample_crate::Plugin for KrOcrBackendBridge"),
        "must generate Plugin impl"
    );
    assert!(code.contains("fn name(&self)"), "Plugin impl must have name()");
    assert!(code.contains("fn version(&self)"), "Plugin impl must have version()");
    assert!(
        code.contains("fn initialize(&self)"),
        "Plugin impl must have initialize()"
    );
    assert!(code.contains("fn shutdown(&self)"), "Plugin impl must have shutdown()");
}

#[test]
fn test_register_fn_generates_extern_c() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("sample_crate::registry::get_ocr".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "sample_crate",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("extern \"C\" fn kr_register_ocr_backend"),
        "register fn must be extern C with correct name"
    );
    assert!(
        code.contains("extern \"C\" fn kr_unregister_ocr_backend"),
        "unregister fn must be extern C with correct name"
    );
    assert!(code.contains("#[unsafe(no_mangle)]"), "register fn must be no_mangle");
}

#[test]
fn test_register_fn_validates_name_null() {
    let trait_def = make_trait_def("MyTrait", vec![make_method("do_thing", TypeRef::Unit, true, false)]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_my_trait".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // Null name check must be present in register fn
    assert!(
        code.contains("if name.is_null()"),
        "register fn must check for null name"
    );
}

#[test]
fn test_register_fn_validates_required_fn_ptrs() {
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
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // Required method fn pointer must be validated; optional one need not be
    assert!(
        code.contains("vtable_ref.transform.is_none()"),
        "required fn ptr must be validated non-null"
    );
}

#[test]
fn test_safety_comments_present() {
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
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("// SAFETY:"),
        "generated code must contain SAFETY comments"
    );
    assert!(
        code.contains("unsafe"),
        "generated code must use unsafe for raw pointer ops"
    );
}

#[test]
fn test_trait_impl_generated() {
    let trait_def = make_trait_def("Scanner", vec![make_method("scan", TypeRef::String, true, false)]);
    let bridge_cfg = sample_bridge_cfg("Scanner");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "sc",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("impl my_lib::Scanner for ScScannerBridge"),
        "must generate trait impl"
    );
    assert!(code.contains("fn scan("), "trait impl must contain the method");
}

#[test]
fn test_string_param_marshalled_to_c_char() {
    let trait_def = make_trait_def(
        "Greeter",
        vec![MethodDef {
            name: "greet".to_string(),
            params: vec![ParamDef {
                name: "message".to_string(),
                ty: TypeRef::String,
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
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Unit,
            is_async: false,
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
    );
    let bridge_cfg = sample_bridge_cfg("Greeter");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "g",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // The vtable fn pointer for 'greet' must accept *const c_char for the message param
    assert!(
        code.contains("*const std::ffi::c_char"),
        "string param must map to *const c_char in vtable"
    );
}

#[test]
fn test_c_param_type_mappings() {
    assert_eq!(
        FfiBridgeGenerator::c_param_type(&TypeRef::String),
        "*const std::ffi::c_char"
    );
    assert_eq!(FfiBridgeGenerator::c_param_type(&TypeRef::Bytes), "*const u8");
    assert_eq!(
        FfiBridgeGenerator::c_param_type(&TypeRef::Primitive(PrimitiveType::Bool)),
        "i32"
    );
    assert_eq!(FfiBridgeGenerator::c_param_type(&TypeRef::Duration), "u64");
}

#[test]
fn test_c_return_convention_unit_fallible() {
    let (out_params, ret) = FfiBridgeGenerator::c_return_convention(&TypeRef::Unit, true);
    assert_eq!(ret, "i32");
    assert_eq!(out_params.len(), 1);
    assert!(out_params[0].contains("out_error"));
}

#[test]
fn test_c_return_convention_string_infallible() {
    let (out_params, ret) = FfiBridgeGenerator::c_return_convention(&TypeRef::String, false);
    // Infallible string is a complex return: it always carries out_result AND out_error
    // (the latter for stack alignment / C# FFI compatibility), and returns i32.
    assert_eq!(out_params.len(), 2);
    assert!(out_params[0].contains("out_result"));
    assert!(out_params.iter().any(|p| p.contains("out_error")));
    assert_eq!(ret, "i32");
}
