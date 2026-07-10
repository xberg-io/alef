use super::*;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn test_gen_registration_fn_requires_register_fn_and_registry_getter() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def("MyTrait", "my_lib::MyTrait", vec![]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

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
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);
    assert!(
        out.is_empty(),
        "registration fn should be empty when register_fn is absent"
    );
}

#[test]
fn test_gen_registration_fn_validates_required_methods() {
    let generator = make_bridge_generator("my_lib");
    let required_method = make_method_def("process", vec![], TypeRef::String, false, true, false);
    let optional_method = make_method_def("describe", vec![], TypeRef::String, false, false, true);
    let trait_def = make_trait_def("Backend", "my_lib::Backend", vec![required_method, optional_method]);
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Backend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_backend".to_string()),

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
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);

    assert!(
        out.contains("\"process\""),
        "registration fn should validate required method 'process'"
    );
    assert!(
        out.contains("PyAttributeError"),
        "registration fn should raise PyAttributeError for missing methods"
    );
    assert!(
        out.contains("#[pyfunction]"),
        "registration fn should be annotated with #[pyfunction]"
    );
    assert!(
        out.contains("fn register_backend"),
        "registration fn should use the configured name"
    );
    assert!(out.contains("Arc::new(wrapper)"), "registration fn should wrap in Arc");
}

#[test]
fn test_gen_registration_fn_calls_registry_getter() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "Processor",
        "my_lib::Processor",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Processor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::registry::get_processors".to_string()),
        register_fn: Some("register_processor".to_string()),

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
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let out = generator.gen_registration_fn(&spec);

    assert!(
        out.contains("my_lib::registry::get_processors()"),
        "registration fn should call the configured registry getter"
    );
    assert!(
        out.contains("registry.register(arc)"),
        "registration fn should call registry.register"
    );
    assert!(
        out.contains("registry.write()"),
        "registration fn should acquire write lock"
    );
}

#[test]
fn test_gen_unregistration_fn_emits_typed_pyfunction_when_configured() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "TextBackend",
        "my_lib::TextBackend",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::plugins::registry::get_text_backend_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),
        unregister_fn: Some("unregister_text_backend".to_string()),
        clear_fn: Some("clear_text_backends".to_string()),
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
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };

    let unreg = generator.gen_unregistration_fn(&spec);
    assert!(unreg.contains("#[pyfunction]"), "unreg must be a pyfunction: {unreg}");
    assert!(unreg.contains("name: String"), "unreg takes name as String: {unreg}");
    assert!(
        unreg.contains("my_lib::plugins::text_backend::unregister_text_backend"),
        "unreg must call the host plugin module fn: {unreg}"
    );

    let clear = generator.gen_clear_fn(&spec);
    assert!(clear.contains("#[pyfunction]"), "clear must be a pyfunction: {clear}");
    assert!(
        clear.contains("my_lib::plugins::text_backend::clear_text_backends"),
        "clear must call the host plugin module fn: {clear}"
    );
}

#[test]
fn test_gen_unregistration_fn_returns_empty_when_unset() {
    let generator = make_bridge_generator("my_lib");
    let trait_def = make_trait_def(
        "TextBackend",
        "my_lib::TextBackend",
        vec![make_method_def("run", vec![], TypeRef::Unit, false, true, false)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: None,
        registry_getter: None,
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
    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Py",
        type_paths: HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "Error".to_string(),
        error_constructor: "Error::from({msg})".to_string(),
    };
    assert!(generator.gen_unregistration_fn(&spec).is_empty());
    assert!(generator.gen_clear_fn(&spec).is_empty());
}
