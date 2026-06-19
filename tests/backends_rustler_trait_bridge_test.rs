use alef::backends::rustler::trait_bridge::gen_trait_bridge;
use alef::core::config::TraitBridgeConfig;
use alef::core::ir::*;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn make_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_node_context()],
        functions: vec![],
        enums: vec![make_visit_result()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_node_context() -> TypeDef {
    TypeDef {
        name: "SyntaxContext".to_string(),
        rust_path: "my_lib::SyntaxContext".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "node_id".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
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
    }
}

fn make_visit_result() -> EnumDef {
    EnumDef {
        name: "WalkDecision".to_string(),
        rust_path: "my_lib::WalkDecision".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Stop".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
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

fn make_visitor_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: Some(format!("{trait_name}Handle")),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: Some("SyntaxContext".to_string()),
        result_type: Some("WalkDecision".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Plugin bridge: wrapper struct
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_generates_wrapper_struct() {
    let trait_def = make_trait_def(
        "TextBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );
    let cfg = make_plugin_bridge_cfg("TextBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("pub struct RustlerTextBackendBridge"),
        "plugin bridge must generate RustlerTextBackendBridge wrapper struct"
    );
    assert!(
        code.code.contains("inner: rustler::LocalPid") || code.code.contains("pid: rustler::LocalPid"),
        "wrapper struct must hold a rustler::LocalPid for message passing"
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
    let trait_def = make_trait_def(
        "TextBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );
    let cfg = make_plugin_bridge_cfg("TextBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code
            .contains("impl my_lib::TextBackend for RustlerTextBackendBridge"),
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
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("\"analyze\""),
        "sync method body must reference the method name 'analyze'"
    );
    assert!(
        code.code.contains("OwnedEnv::new()") || code.code.contains("send_and_clear"),
        "sync method body must dispatch via OwnedEnv and send_and_clear to the GenServer"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: async method uses spawn_blocking
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_async_method_uses_spawn_blocking() {
    let trait_def = make_trait_def("Processor", vec![make_async_method("run")]);
    let cfg = make_plugin_bridge_cfg("Processor");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

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
    let trait_def = make_trait_def(
        "TextBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );
    let cfg = make_plugin_bridge_cfg("TextBackend");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("pub fn register_textbackend("),
        "registration fn must be generated with the configured name"
    );
    assert!(
        code.code.contains(r#"#[rustler::nif(schedule = "DirtyCpu")]"#),
        "registration fn must carry #[rustler::nif(schedule = \"DirtyCpu\")] to prevent BEAM scheduler deadlock"
    );
    assert!(
        code.code.contains("my_lib::get_registry"),
        "registration fn must call the configured registry getter"
    );
    assert!(
        code.code
            .contains("genserver_pid: rustler::LocalPid, plugin_name: String")
            && code
                .code
                .contains("RustlerTextBackendBridge::new(genserver_pid, plugin_name)"),
        "registration fn must store the provided GenServer pid and require a plugin name, got:\n{}",
        code.code
    );
    assert!(
        !code.code.contains("RustlerTextBackendBridge::new(env.pid()")
            && !code.code.contains("RustlerTextBackendBridge::new(pid)"),
        "registration fn must not fall back to the NIF caller pid, got:\n{}",
        code.code
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
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

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
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("cached_name"),
        "constructor must populate cached_name"
    );
    assert!(
        code.code.contains("LocalPid") || code.code.contains("plugin_name"),
        "constructor must accept a LocalPid and plugin_name for message passing"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: super_trait generates Plugin impl
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_with_super_trait_generates_plugin_impl() {
    let trait_def = make_trait_def(
        "TextBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );
    let cfg = TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),

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
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("impl my_lib::Plugin for RustlerTextBackendBridge"),
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

    // Regression: initialize()/shutdown() take no args, so the args map is
    // never mutated. Emitting `let mut args` triggers unused_mut warnings
    // (clippy/rustc) in the generated NIF code. See
    // packages/elixir/native/sample_crate_nif/src/lib.rs:5159/:5196 reproduction.
    assert!(
        !code.code.contains("let mut args = serde_json::Map::new();"),
        "no-arg trait methods must emit `let args`, not `let mut args` (unused_mut)"
    );
    assert!(
        code.code.contains("let args = serde_json::Map::new();"),
        "no-arg trait methods must emit `let args = serde_json::Map::new();`"
    );
}

// ---------------------------------------------------------------------------
// Visitor bridge
// ---------------------------------------------------------------------------

#[test]
fn test_visitor_bridge_generates_elixir_bridge_struct() {
    let trait_def = make_trait_def(
        "SyntaxWalker",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("SyntaxWalker");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("pub struct ElixirSyntaxWalkerBridge"),
        "visitor bridge must produce ElixirSyntaxWalkerBridge struct"
    );
}

#[test]
fn test_visitor_bridge_does_not_generate_registration_fn() {
    let trait_def = make_trait_def(
        "SyntaxWalker",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("SyntaxWalker");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

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
        "SyntaxWalker",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("SyntaxWalker");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code
            .contains("impl my_lib::SyntaxWalker for ElixirSyntaxWalkerBridge"),
        "visitor bridge must implement the trait"
    );
}

#[test]
fn test_visitor_bridge_holds_owned_env_and_saved_term() {
    let trait_def = make_trait_def(
        "SyntaxWalker",
        vec![make_method("visit_node", TypeRef::Unit, false, true)],
    );
    let cfg = make_visitor_bridge_cfg("SyntaxWalker");
    let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("rustler::OwnedEnv"),
        "visitor bridge struct must hold a rustler::OwnedEnv"
    );
    assert!(
        code.code.contains("rustler::env::SavedTerm"),
        "visitor bridge struct must hold a rustler::env::SavedTerm"
    );
}

// ---------------------------------------------------------------------------
// Plugin bridge: Send + Sync compliance
// ---------------------------------------------------------------------------

#[test]
fn test_plugin_bridge_struct_does_not_hold_owned_env() {
    let trait_def = make_trait_def(
        "TextBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );
    let cfg = make_plugin_bridge_cfg("TextBackend");
    let output = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    // The struct definition should NOT contain an 'env:' field that holds OwnedEnv.
    // Only 'inner: SavedTerm' and 'cached_name: String', both of which are Send + Sync.
    let struct_section = output
        .code
        .split("pub struct RustlerTextBackendBridge")
        .nth(1)
        .and_then(|s| s.split("}").next())
        .unwrap_or("");

    // Check that there's no field named "env:" (but "rustler::env::" in the type is OK)
    let has_env_field = struct_section.lines().any(|line| line.trim().starts_with("env:"));

    assert!(
        !has_env_field,
        "plugin bridge struct must not hold an OwnedEnv field to ensure Send + Sync"
    );
}

#[test]
fn test_plugin_bridge_sync_method_creates_owned_env_locally() {
    let trait_def = make_trait_def("Analyzer", vec![make_method("analyze", TypeRef::String, true, false)]);
    let cfg = make_plugin_bridge_cfg("Analyzer");
    let output = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api())
        .expect("trait bridge generation should succeed");

    // Sync method body should create a fresh OwnedEnv locally, not access self.env.
    let method_impl = output.code.split("fn analyze(").nth(1).unwrap_or("");
    assert!(
        method_impl.contains("let mut env = rustler::OwnedEnv::new()"),
        "sync method must create OwnedEnv locally for thread-safe dispatch"
    );
    assert!(
        !method_impl.contains("self.env.run"),
        "sync method must not use self.env (which doesn't exist)"
    );
}
