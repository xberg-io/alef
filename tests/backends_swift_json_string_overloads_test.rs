use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, CoreWrapper, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

// ── helpers ─────────────────────────────────────────────────────────────────

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
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
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
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

fn make_type(name: &str, fields: Vec<FieldDef>, has_serde: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_function(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("mylib::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: Some("MyError".to_string()),
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
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

// ── tests ───────────────────────────────────────────────────────────────────

/// Verify that JSON-string overloads are emitted for functions with serde config params.
#[test]
fn json_string_overloads_emitted_for_serde_config() {
    // Create a serde-enabled config struct.
    let config_type = make_type(
        "ProcessConfig",
        vec![make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U64), false)],
        true, // has_serde
    );

    // Create a function that takes this config.
    let func = make_function(
        "process_data",
        vec![
            make_param("input", TypeRef::String),
            make_param("config", TypeRef::Named("ProcessConfig".to_string())),
        ],
        TypeRef::Named("ProcessResult".to_string()),
    );

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_config();
    let swift = SwiftBackend;

    // Generate bindings.
    let files = swift
        .generate_bindings(&api, &config)
        .expect("binding generation must succeed");

    // Find the main Mylib.swift file.
    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Mylib.swift"))
        .expect("Mylib.swift must be generated")
        .content
        .clone();

    // Verify JSON-string overload is emitted.
    assert!(
        content.contains("public func processData(_ input: String, _ configJson: String)"),
        "must emit positional-arg JSON-string overload with configJson parameter. Content:\n{content}"
    );

    // Verify config decoding call is present.
    assert!(
        content.contains("processConfigFromJson(configJson)"),
        "must decode JSON config via fromJson helper using configJson parameter. Content:\n{content}"
    );

    // Verify the helper delegation is present.
    assert!(
        content.contains("return try processData(input: input, config: config)"),
        "must delegate to typed base function. Content:\n{content}"
    );
}

/// Verify that _loadBytesFromPathOrUtf8 helper is emitted.
#[test]
fn load_bytes_from_path_or_utf8_helper_emitted() {
    let config_type = make_type(
        "ProcessConfig",
        vec![make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U64), false)],
        true,
    );

    let func = make_function(
        "process_data",
        vec![
            make_param("input", TypeRef::String),
            make_param("config", TypeRef::Named("ProcessConfig".to_string())),
        ],
        TypeRef::Named("ProcessResult".to_string()),
    );

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_config();
    let swift = SwiftBackend;

    let files = swift
        .generate_bindings(&api, &config)
        .expect("binding generation must succeed");

    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Mylib.swift"))
        .expect("Mylib.swift must be generated")
        .content
        .clone();

    // Verify the helper is emitted.
    assert!(
        content.contains("private func _loadBytesFromPathOrUtf8"),
        "must emit _loadBytesFromPathOrUtf8 helper. Content:\n{content}"
    );

    // Verify key logic: environment variable check.
    assert!(
        content.contains("ALEF_TEST_DOCUMENTS_DIR"),
        "must check ALEF_TEST_DOCUMENTS_DIR env var. Content:\n{content}"
    );

    // Verify fallback to UTF-8.
    assert!(
        content.contains("pathOrContent.utf8"),
        "must fallback to UTF-8 if file not found. Content:\n{content}"
    );
}

/// Verify that both async and sync functions emit JSON-string overloads.
#[test]
fn json_string_overloads_emitted_for_async_and_sync_functions() {
    let config_type = make_type(
        "ProcessConfig",
        vec![make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U64), false)],
        true,
    );

    // Create a sync function.
    let sync_func = make_function(
        "process_data",
        vec![make_param("config", TypeRef::Named("ProcessConfig".to_string()))],
        TypeRef::Named("ProcessResult".to_string()),
    );

    // Create an async function with same name.
    let mut async_func = make_function(
        "process_data",
        vec![make_param("config", TypeRef::Named("ProcessConfig".to_string()))],
        TypeRef::Named("ProcessResult".to_string()),
    );
    async_func.is_async = true;

    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![sync_func, async_func],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_config();
    let swift = SwiftBackend;

    let files = swift
        .generate_bindings(&api, &config)
        .expect("binding generation must succeed");

    let content = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Mylib.swift"))
        .expect("Mylib.swift must be generated")
        .content
        .clone();

    // Should emit JSON-string overload for both async and sync variants.
    assert!(
        content.contains("public func processData(_ configJson: String)"),
        "must emit JSON-string overload for both async and sync functions with configJson parameter. Content:\n{content}"
    );
}
