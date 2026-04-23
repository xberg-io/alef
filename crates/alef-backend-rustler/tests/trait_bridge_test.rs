use alef_backend_rustler::trait_bridge::gen_trait_bridge;
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::*;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

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

fn make_async_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: true,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
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

fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
    }
}

fn make_visitor_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some(format!("{trait_name}Handle")),
        param_name: None,
        register_extra_args: None,
    }
}

// ---------------------------------------------------------------------------
// Plugin bridge: wrapper struct
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_generates_wrapper_struct() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let cfg = make_plugin_bridge_cfg("OcrBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("pub struct RustlerOcrBackendBridge"),
        "plugin bridge must generate RustlerOcrBackendBridge wrapper struct"
    );
    assert!(
        code.code.contains("inner: rustler::env::SavedTerm"),
        "wrapper struct must hold a rustler::env::SavedTerm"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "wrapper struct must cache the plugin name"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: trait impl
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_generates_trait_impl() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let cfg = make_plugin_bridge_cfg("OcrBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code
            .contains("impl my_lib::OcrBackend for RustlerOcrBackendBridge"),
        "plugin bridge must implement the trait for the wrapper"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must include all trait methods"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: sync method body uses OwnedEnv + map_get
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_sync_method_uses_owned_env_and_map_get() {
    let trait_def = make_trait_def("Analyzer", vec![make_method("analyze", TypeRef::String, true, false)]);
    let cfg = make_plugin_bridge_cfg("Analyzer");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("\"analyze\""),
        "sync method body must reference the method name 'analyze'"
    );
    assert!(
        code.code.contains("map_get") || code.code.contains("self.env.run"),
        "sync method body must dispatch via OwnedEnv or map_get"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: async method uses spawn_blocking
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_async_method_uses_spawn_blocking() {
    let trait_def = make_trait_def("Processor", vec![make_async_method("run")]);
    let cfg = make_plugin_bridge_cfg("Processor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("spawn_blocking"),
        "async method body must use tokio::task::spawn_blocking"
    );
    assert!(
        code.code.contains("async fn run("),
        "async method must be declared async"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: registration function
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_generates_registration_fn() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let cfg = make_plugin_bridge_cfg("OcrBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("pub fn register_ocrbackend("),
        "registration fn must be generated with the configured name"
    );
    assert!(
        code.code.contains("#[rustler::nif]"),
        "registration fn must carry #[rustler::nif] attribute"
    );
    assert!(
        code.code.contains("my_lib::get_registry"),
        "registration fn must call the configured registry getter"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: registration validates required methods
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_registration_validates_required_methods() {
    let trait_def = make_trait_def(
        "Transform",
        vec![
            make_method("transform", TypeRef::String, true, false),
            make_method("describe", TypeRef::String, false, true),
        ],
    );
    let cfg = make_plugin_bridge_cfg("Transform");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("\"transform\""),
        "registration fn must check for required method 'transform'"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: constructor caches name
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_constructor_caches_name() {
    let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
    let cfg = make_plugin_bridge_cfg("Worker");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("cached_name"),
        "constructor must populate cached_name"
    );
    assert!(
        code.code.contains("OwnedEnv") || code.code.contains("owned"),
        "constructor must create an OwnedEnv to extend term lifetime"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: super_trait generates Plugin impl
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_with_super_trait_generates_plugin_impl() {
    let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
    let cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
    };
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("impl my_lib::Plugin for RustlerOcrBackendBridge"),
        "must generate Plugin impl for bridge struct"
    );
    assert!(code.code.contains("fn name(&self)"), "Plugin impl must include name()");
    assert!(
        code.code.contains("fn initialize(&self)"),
        "Plugin impl must include initialize()"
    );
    assert!(
        code.code.contains("fn shutdown(&self)"),
        "Plugin impl must include shutdown()"
    );
}

// ---------------------------------------------------------------------------
// Visitor bridge
// ---------------------------------------------------------------------------

#[test]
fn test_visitor_bridge_generates_elixir_bridge_struct() {
    let trait_def = make_trait_def(
        "HtmlVisitor",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("HtmlVisitor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("pub struct ElixirHtmlVisitorBridge"),
        "visitor bridge must produce ElixirHtmlVisitorBridge struct"
    );
}

#[test]
fn test_visitor_bridge_does_not_generate_registration_fn() {
    let trait_def = make_trait_def(
        "HtmlVisitor",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("HtmlVisitor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    // Visitor bridges do not generate a register_{trait} function, but may
    // emit helper NIFs (e.g. visitor_reply). Verify no registration fn exists.
    assert!(
        !code.code.contains("pub fn register_"),
        "visitor bridge must not generate a register_ registration function"
    );
}

#[test]
fn test_visitor_bridge_generates_trait_impl() {
    let trait_def = make_trait_def(
        "HtmlVisitor",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("HtmlVisitor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code
            .contains("impl my_lib::HtmlVisitor for ElixirHtmlVisitorBridge"),
        "visitor bridge must implement the trait"
    );
}

#[test]
fn test_visitor_bridge_holds_owned_env_and_saved_term() {
    let trait_def = make_trait_def(
        "HtmlVisitor",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("HtmlVisitor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

    assert!(
        code.code.contains("rustler::OwnedEnv"),
        "visitor bridge struct must hold a rustler::OwnedEnv"
    );
    assert!(
        code.code.contains("rustler::env::SavedTerm"),
        "visitor bridge struct must hold a rustler::env::SavedTerm"
    );
}
