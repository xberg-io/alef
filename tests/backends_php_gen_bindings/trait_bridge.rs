use super::*;

#[test]
fn test_php_visitor_bridge_produces_visitor_struct() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "HtmlVisitor",
        vec![make_method_php("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_php("HtmlVisitor", "HtmlVisitor");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("PhpHtmlVisitorBridge"),
        "PHP visitor bridge struct must be named Php{{TraitName}}Bridge"
    );
    assert!(
        code.code.contains("impl my_lib::HtmlVisitor for PhpHtmlVisitorBridge"),
        "PHP visitor bridge must implement the trait"
    );
}

#[test]
fn test_php_visitor_bridge_has_php_obj_field() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "HtmlVisitor",
        vec![make_method_php("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_php("HtmlVisitor", "HtmlVisitor");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("php_obj: *mut ext_php_rs::types::ZendObject"),
        "PHP visitor bridge must store a raw ZendObject pointer in 'php_obj'"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "PHP visitor bridge must cache the plugin name"
    );
}

#[test]
fn test_php_plugin_bridge_produces_wrapper_struct_with_inner_and_cached_name() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "OcrBackend",
        vec![make_method_php("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_php("OcrBackend");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("pub struct PhpOcrBackendBridge"),
        "PHP plugin bridge wrapper struct must be PhpOcrBackendBridge"
    );
    assert!(
        code.code.contains("inner:"),
        "PHP plugin bridge wrapper must have an 'inner' field"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "PHP plugin bridge wrapper must have a 'cached_name: String' field"
    );
}

#[test]
fn test_php_plugin_bridge_generates_super_trait_impl() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "OcrBackend",
        vec![make_method_php("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_php("OcrBackend");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::Plugin for PhpOcrBackendBridge"),
        "PHP plugin bridge must implement Plugin super-trait"
    );
    assert!(code.code.contains("fn name("), "Plugin impl must contain name()");
    assert!(
        code.code.contains("fn initialize("),
        "Plugin impl must contain initialize()"
    );
    assert!(
        code.code.contains("fn shutdown("),
        "Plugin impl must contain shutdown()"
    );
}

#[test]
fn test_php_plugin_bridge_generates_trait_impl_with_forwarded_methods() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "OcrBackend",
        vec![make_method_php("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_php("OcrBackend");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::OcrBackend for PhpOcrBackendBridge"),
        "PHP plugin bridge must implement the trait itself"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must forward the 'process' method"
    );
}

#[test]
fn test_php_plugin_bridge_generates_registration_fn_with_php_function_attribute() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "OcrBackend",
        vec![make_method_php("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_php("OcrBackend");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("#[php_function]"),
        "PHP registration function must carry the #[php_function] attribute"
    );
    assert!(
        code.code.contains("pub fn register_ocrbackend("),
        "PHP registration function must use the configured name"
    );
}

#[test]
fn test_php_trait_registry_methods_use_matching_native_facade_and_stub_names() {
    let backend = PhpBackend;
    let mut config = make_config();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }];
    let api = ApiSurface {
        types: vec![make_trait_def_php(
            "OcrBackend",
            vec![make_method_php("process", TypeRef::String, true, false)],
        )],
        ..make_api_php()
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");
    assert!(
        lib.content
            .contains("#[php(name = \"registerOcrBackend\")]\n    pub fn register_ocr_backend(")
            && lib
                .content
                .contains("#[php(name = \"unregisterOcrBackend\")]\n    pub fn unregister_ocr_backend(")
            && lib
                .content
                .contains("#[php(name = \"clearOcrBackends\")]\n    pub fn clear_ocr_backends("),
        "native Api class methods must expose the same camelCase names used by the facade:\n{}",
        lib.content
    );

    let public = backend.generate_public_api(&api, &config).unwrap();
    let facade = &public[0].content;
    assert!(
        facade.contains("public static function registerOcrBackend(\nOcrBackend $backend) : void")
            && facade.contains("\\Test\\Lib\\TestLibApi::registerOcrBackend($backend)")
            && facade.contains("\\Test\\Lib\\TestLibApi::unregisterOcrBackend($name)")
            && facade.contains("\\Test\\Lib\\TestLibApi::clearOcrBackends()"),
        "facade methods must call the native Api class public names:\n{facade}"
    );

    let stubs = backend.generate_type_stubs(&api, &config).unwrap();
    let stub = &stubs[0].content;
    assert!(
        stub.contains("public static function registerOcrBackend(\\Test\\Lib\\OcrBackend $backend): void")
            && stub.contains("public static function unregisterOcrBackend(string $name): void")
            && stub.contains("public static function clearOcrBackends(): void"),
        "extension stubs must expose registry methods on the native Api class:\n{stub}"
    );
}

#[test]
fn test_php_plugin_bridge_validates_required_methods() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "Analyzer",
        vec![
            make_method_php("analyze", TypeRef::String, true, false), // required
            make_method_php("describe", TypeRef::String, false, true), // optional
        ],
    );
    let bridge_cfg = alef::core::config::TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: Some("Plugin".to_string()),
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
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    // Registration fn must null-check the required method "analyze" via get_property
    assert!(
        code.code.contains("\"analyze\""),
        "PHP registration fn must validate required method 'analyze'"
    );
    assert!(
        code.code.contains("try_call_method"),
        "PHP registration fn must check method presence via try_call_method"
    );
}

#[test]
fn test_php_sync_method_body_uses_try_call_method() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php("Scanner", vec![make_method_php("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_plugin_bridge_cfg_php("Scanner");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("try_call_method"),
        "PHP sync method body must use try_call_method to dispatch to PHP"
    );
}

#[test]
fn test_php_async_method_body_uses_box_pin() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php("Processor", vec![make_async_method_php("run", TypeRef::Unit)]);
    let bridge_cfg = make_plugin_bridge_cfg_php("Processor");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("WORKER_RUNTIME.block_on(async"),
        "PHP async method body must use WORKER_RUNTIME.block_on(async {{ ... }})"
    );
}

#[test]
fn test_php_visitor_bridge_has_send_sync_impls() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "HtmlVisitor",
        vec![make_method_php("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_php("HtmlVisitor", "HtmlVisitor");
    let api = make_api_php();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("unsafe impl Send for PhpHtmlVisitorBridge"),
        "PHP visitor bridge must implement Send"
    );
    assert!(
        code.code.contains("unsafe impl Sync for PhpHtmlVisitorBridge"),
        "PHP visitor bridge must implement Sync"
    );
}

// ---------------------------------------------------------------------------
// Native-object trait-callback args + typed host interface (neutral fixtures)
// ---------------------------------------------------------------------------

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
fn test_php_sync_struct_param_marshalled_as_native_object_not_json() {
    use alef::backends::php::trait_bridge::gen_trait_bridge;

    let (trait_def, api) = make_greeter_api();
    let bridge_cfg = make_plugin_bridge_cfg_php("Greeter");
    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    // (a) serde struct param `opts` is built as the binding's native PHP object via From<core::T>.
    assert!(
        code.code.contains("Zval::try_from(Opts::from((*opts).clone()))"),
        "serde struct param must be marshalled as the native PHP object, not a JSON string:\n{}",
        code.code
    );
    // (b) the struct param must NOT be JSON-serialized.
    assert!(
        !code.code.contains("serde_json::to_string(&opts)"),
        "serde struct param must not be JSON-serialized:\n{}",
        code.code
    );
    // (b) enum / opaque / excluded params keep the prior JSON-string representation.
    for name in ["mood", "handle", "hidden"] {
        assert!(
            code.code.contains(&format!("serde_json::to_string(&{name})")),
            "non-struct param `{name}` must keep its JSON-string representation:\n{}",
            code.code
        );
        assert!(
            !code.code.contains(&format!("::from((*{name}).clone())")),
            "non-struct param `{name}` must NOT be native-marshalled:\n{}",
            code.code
        );
    }
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

    // (c) host-implementable interface with the serde struct param typed natively and the
    //     serde struct return typed natively; non-struct params fall back to mixed.
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

#[test]
fn test_php_register_fn_typed_against_interface() {
    // (d) the PHP facade's register_* method types `backend` against the emitted interface.
    let backend = PhpBackend;
    let mut config = make_config_with_extension("greeter_ext");

    let (greeter, mut api) = make_greeter_api();
    // Give the trait a method with no params so the facade/native surface stays simple; the
    // register typing comes from the bridge config + interface name.
    api.types.insert(0, greeter);
    config.trait_bridges = vec![make_plugin_bridge_cfg_php("Greeter")];

    // The host-implementable interface file `Greeter.php` carries the typed contract.
    let iface_files = backend
        .generate_bindings(&api, &config)
        .expect("php generation must succeed");
    let iface = iface_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Greeter.php"))
        .map(|f| f.content.clone())
        .expect("Greeter.php interface file must be emitted");
    assert!(
        iface.contains("interface Greeter"),
        "interface file must declare interface:\n{iface}"
    );

    // The PHP facade types the register_* method's `backend` param against that interface.
    let facade = backend
        .generate_public_api(&api, &config)
        .expect("php public-api generation must succeed");
    let facade_typed = facade.iter().any(|f| f.content.contains("Greeter $backend) : void"));
    assert!(
        facade_typed,
        "register_* facade method must type its backend param against the `Greeter` interface"
    );
}
