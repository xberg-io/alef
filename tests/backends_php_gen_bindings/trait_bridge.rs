use super::*;

fn assert_php_callback_generation_is_disabled(code: &str) {
    assert!(code.contains("compile_error!"), "expected an explicit failure: {code}");
    for forbidden in [
        "Zval",
        "ZendCallable",
        "unsafe impl Send",
        "unsafe impl Sync",
        "Arc<dyn",
        "Arc::new",
    ] {
        assert!(!code.contains(forbidden), "unsafe token `{forbidden}` in: {code}");
    }
}

#[test]
fn raw_trait_bridge_generator_fails_closed_for_every_bridge_shape() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "OcrBackend",
        vec![make_method_php("process", TypeRef::String, true, false)],
    );
    let api = make_api_php();
    let mut direct = make_plugin_bridge_cfg_php("OcrBackend");
    direct.registry_getter = None;
    let mut function_param = make_plugin_bridge_cfg_php("OcrBackend");
    function_param.bind_via = alef::core::config::BridgeBinding::FunctionParam;
    let mut options_field = make_plugin_bridge_cfg_php("OcrBackend");
    options_field.bind_via = alef::core::config::BridgeBinding::OptionsField;
    let visitor = make_visitor_bridge_cfg_php("OcrBackend", "OcrBackend");

    for bridge in [
        make_plugin_bridge_cfg_php("OcrBackend"),
        direct,
        function_param,
        options_field,
        visitor,
    ] {
        let output = gen_trait_bridge(&trait_def, &bridge, "my_lib", "Error", "Error::from({msg})", &api);
        assert!(output.imports.is_empty());
        assert_php_callback_generation_is_disabled(&output.code);
    }
}

#[test]
fn php_backend_entrypoints_reject_an_active_trait_bridge() {
    let backend = PhpBackend;
    let mut config = make_config();
    config.replace_trait_bridges(vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }]);
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        types: vec![make_trait_def_php(
            "OcrBackend",
            vec![make_method_php("process", TypeRef::String, true, false)],
        )],
        ..make_api_php()
    };

    for result in [
        backend.generate_bindings(&api, &config),
        backend.generate_public_api(&api, &config),
        backend.generate_type_stubs(&api, &config),
        backend.generate_service_api(&api, &config),
    ] {
        let error = result.expect_err("every PHP output entrypoint must fail closed");
        assert!(
            error.to_string().contains("PHP trait bridge `OcrBackend` is disabled"),
            "the backend gate must identify the unsafe bridge: {error}"
        );
    }
    assert!(
        backend.trait_bridge_registration_surface(&api, &config).is_empty(),
        "an unsafe bridge must not advertise callable registration symbols"
    );
}

#[test]
fn php_excluded_trait_bridge_emits_no_zend_thread_escape() {
    let backend = PhpBackend;
    let mut config = make_config();
    config.replace_trait_bridges(vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        exclude_languages: vec!["php".to_string()],
        ..Default::default()
    }]);
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        types: vec![make_trait_def_php(
            "OcrBackend",
            vec![make_method_php("process", TypeRef::String, true, false)],
        )],
        ..make_api_php()
    };

    let mut files = backend.generate_bindings(&api, &config).unwrap();
    files.extend(backend.generate_public_api(&api, &config).unwrap());
    files.extend(backend.generate_type_stubs(&api, &config).unwrap());
    files.extend(backend.generate_service_api(&api, &config).unwrap());
    let output = files.into_iter().map(|file| file.content).collect::<String>();

    for forbidden in [
        "unsafe impl Send for PhpOcrBackendBridge",
        "unsafe impl Sync for PhpOcrBackendBridge",
        "Arc<dyn my_lib::OcrBackend>",
        "inner: ext_php_rs::types::Zval",
    ] {
        assert!(
            !output.contains(forbidden),
            "a PHP-excluded bridge must not emit `{forbidden}`"
        );
    }
}

/// A non-opaque serde struct DTO (qualifies for native-object marshalling).
fn make_serde_struct(name: &str) -> TypeDef {
    let mut t = make_node_context_php();
    t.name = name.to_string();
    t.rust_path = format!("my_lib::{name}");
    t.fields = vec![make_field("label", TypeRef::String, false)];
    t
}

/// An opaque/handle type (must NOT be native-marshalled).
fn make_opaque_type(name: &str) -> TypeDef {
    let mut t = make_serde_struct(name);
    t.is_opaque = true;
    t
}

fn make_named_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        ..ParamDef::default()
    }
}

/// `Greeter` plugin trait: `greet(opts: &Opts, mood: &Mood, handle: &Handle, hidden: &Hidden) -> Doc`.
/// `Opts`/`Doc` are serde structs; `Mood` is an enum; `Handle` is opaque; `Hidden` is excluded.
fn make_greeter_api() -> (TypeDef, ApiSurface) {
    let greet = MethodDef {
        params: vec![
            make_named_param("opts", "Opts"),
            make_named_param("mood", "Mood"),
            make_named_param("handle", "Handle"),
            make_named_param("hidden", "Hidden"),
        ],
        ..make_method_php("greet", TypeRef::Named("Doc".to_string()), true, false)
    };
    let trait_def = make_trait_def_php("Greeter", vec![greet]);

    let mut hidden = make_serde_struct("Hidden");
    hidden.binding_excluded = true;

    let mut mood_enum = make_visit_result_php();
    mood_enum.name = "Mood".to_string();
    mood_enum.rust_path = "my_lib::Mood".to_string();

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        types: vec![
            make_serde_struct("Opts"),
            make_serde_struct("Doc"),
            make_opaque_type("Handle"),
            hidden,
        ],
        enums: vec![mood_enum],
        ..make_api_php()
    };
    (trait_def, api)
}

#[test]
fn test_php_typed_interface_emitted_for_plugin_bridge() {
    use alef::backends::php::trait_bridge::gen_registration_interface;

    let (trait_def, api) = make_greeter_api();
    let bridge_cfg = make_plugin_bridge_cfg_php("Greeter");
    let iface = gen_registration_interface(
        &trait_def,
        &bridge_cfg,
        "My\\Ns",
        &std::collections::HashMap::new(),
        &api,
    );

    assert!(
        iface.contains("interface Greeter"),
        "plugin interface must be emitted:\n{iface}"
    );
    assert!(
        iface.contains("public function greet(Opts $opts, mixed $mood, mixed $handle, mixed $hidden): Doc;"),
        "interface must type the serde struct param as `Opts`, the return as `Doc`, and leave \
         enum/opaque/excluded params as mixed:\n{iface}"
    );
    assert!(
        iface.contains("@param Opts $opts"),
        "PHPDoc must type the serde struct param:\n{iface}"
    );
    assert!(iface.contains("@return Doc"), "PHPDoc must type the return:\n{iface}");
}
