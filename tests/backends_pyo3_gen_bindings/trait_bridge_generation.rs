use super::*;

// ---------------------------------------------------------------------------
// gen_trait_bridge (the main entry point)
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridge_produces_non_empty_output_for_plugin_pattern() {
    let method = make_method_def("process", vec![], TypeRef::String, false, true, false);
    let trait_def = make_trait_def("TextBackend", "my_lib::TextBackend", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
        &[],
    )
    .expect("trait bridge generation should succeed");

    assert!(!code.code.is_empty(), "gen_trait_bridge must produce non-empty output");
    assert!(
        code.code.contains("PyTextBackendBridge"),
        "output should define the bridge wrapper struct"
    );
    assert!(
        code.imports.iter().any(|i| i.contains("pyo3::prelude")),
        "output should import pyo3 prelude"
    );
    assert!(
        code.code.contains("fn process"),
        "output should include the trait method"
    );
}

#[test]
fn test_gen_trait_bridge_wrapper_struct_has_required_fields() {
    let method = make_method_def("run", vec![], TypeRef::Unit, false, true, false);
    let trait_def = make_trait_def("Worker", "my_lib::Worker", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Worker".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_workers".to_string()),
        register_fn: Some("register_worker".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
        &[],
    )
    .expect("trait bridge generation should succeed");

    // The wrapper struct must hold the Python object and a cached name field
    assert!(
        code.code.contains("inner: Py<PyAny>"),
        "wrapper struct must hold inner Py<PyAny>"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "wrapper struct must hold cached_name"
    );
}

#[test]
fn test_gen_trait_bridge_generates_registration_fn_when_configured() {
    let method = make_method_def("infer", vec![], TypeRef::String, false, true, false);
    let trait_def = make_trait_def("InferenceBackend", "my_lib::InferenceBackend", vec![method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "InferenceBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_inference_registry".to_string()),
        register_fn: Some("register_inference_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
        &[],
    )
    .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("fn register_inference_backend"),
        "should generate registration function with configured name"
    );
    assert!(
        code.code.contains("#[pyfunction]"),
        "registration function should carry #[pyfunction] attribute"
    );
}

#[test]
fn test_gen_trait_bridge_with_sync_and_async_required_methods() {
    // A trait with one sync and one async required method — exercises both code paths
    let sync_method = make_method_def(
        "validate",
        vec![],
        TypeRef::Primitive(PrimitiveType::Bool),
        false,
        false,
        false,
    );
    let async_method = make_method_def("process", vec![], TypeRef::String, true, true, false);
    let trait_def = make_trait_def(
        "HybridBackend",
        "my_lib::HybridBackend",
        vec![sync_method, async_method],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "HybridBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_hybrid_registry".to_string()),
        register_fn: Some("register_hybrid_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    };
    let api = make_api_surface();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "my_lib",
        "Error",
        "Error::from({msg})",
        &api,
        &[],
    )
    .expect("trait bridge generation should succeed");

    assert!(!code.code.is_empty(), "output must not be empty");
    // Sync method body uses Python::attach (no spawn_blocking)
    assert!(
        code.code.contains("fn validate"),
        "sync method should be present in trait impl"
    );
    // Async method body uses spawn_blocking
    assert!(
        code.code.contains("fn process"),
        "async method should be present in trait impl"
    );
    assert!(
        code.code.contains("spawn_blocking"),
        "async method body should use spawn_blocking"
    );
    // Both methods are required — registration fn should validate both
    assert!(
        code.code.contains("\"validate\"") || code.code.contains("\"process\""),
        "registration fn should validate required method names"
    );
}
