use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, TraitBridgeConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType,
    ReceiverKind, TypeDef, TypeRef,
};
use alef_core::template_versions::cargo as tv;

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
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
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
        has_serde: false,
        super_traits: vec![],
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        version: Some("0.1.0".to_string()),
        crate_config: CrateConfig {
            name: "demo-crate".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
    format: ::alef_core::config::FormatConfig::default(),
    format_overrides: ::std::collections::HashMap::new(),
    }
}

/// Helper: generate all files and find the one at the given suffix.
fn find_file<'a>(files: &'a [alef_core::backend::GeneratedFile], suffix: &str) -> Option<&'a str> {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(suffix))
        .map(|f| f.content.as_str())
}

#[test]
fn cargo_toml_contains_frb_version() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    let expected_frb = tv::FLUTTER_RUST_BRIDGE;
    assert!(
        cargo.contains(expected_frb),
        "Cargo.toml missing FRB version {expected_frb}: {cargo}"
    );
    assert!(cargo.contains("[package]"), "missing [package] section: {cargo}");
    assert!(cargo.contains("demo-crate-dart"), "missing crate name: {cargo}");
    assert!(cargo.contains("flutter_rust_bridge"), "missing flutter_rust_bridge dep: {cargo}");
    assert!(
        cargo.contains(r#"path = "../../..""#),
        "missing relative path dep: {cargo}"
    );
}

#[test]
fn lib_rs_emits_mirror_struct_per_ir_type() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type(
                "Point",
                vec![
                    make_field("x_coord", TypeRef::Primitive(PrimitiveType::I32), false),
                    make_field("y_coord", TypeRef::Primitive(PrimitiveType::I32), false),
                ],
            ),
            make_type("Empty", vec![]),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(lib.contains("#[frb(mirror(Point))]"), "missing mirror for Point: {lib}");
    assert!(lib.contains("pub struct Point {"), "missing Point mirror struct: {lib}");
    assert!(lib.contains("pub x_coord: i64"), "x_coord should be i64: {lib}");
    assert!(lib.contains("pub y_coord: i64"), "y_coord should be i64: {lib}");
    assert!(lib.contains("#[frb(mirror(Empty))]"), "missing mirror for Empty: {lib}");
    assert!(lib.contains("pub struct Empty {"), "missing Empty mirror struct: {lib}");
}

#[test]
fn lib_rs_emits_bridge_fn_per_ir_function() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet_user".into(),
            rust_path: "demo::greet_user".into(),
            original_rust_path: String::new(),
            params: vec![make_param("user_name", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // FRB v2: ordinary public functions need no annotation; bare `#[frb]` is rejected.
    assert!(!lib.contains("#[frb]\npub fn"), "bare #[frb] on fn is invalid in v2: {lib}");
    assert!(lib.contains("pub fn greet_user"), "missing greet_user fn: {lib}");
    assert!(lib.contains("user_name: String"), "missing user_name param: {lib}");
    // rust_path resolution: call site uses the full module path, not the bare fn name.
    assert!(lib.contains("demo::greet_user("), "should call demo::greet_user via rust_path: {lib}");
}

#[test]
fn lib_rs_async_fn_uses_async_fn_keyword() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_data".into(),
            rust_path: "demo::fetch_data".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(lib.contains("pub async fn fetch_data"), "missing async fn keyword: {lib}");
}

#[test]
fn lib_rs_result_fn_uses_map_err_to_string() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "parse_input".into(),
            rust_path: "demo::parse_input".into(),
            original_rust_path: String::new(),
            params: vec![make_param("raw", TypeRef::String)],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("ParseError".into()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("Result<String, String>"),
        "missing Result<String, String> return type: {lib}"
    );
    assert!(
        lib.contains(".map_err(|e| e.to_string())"),
        "missing .map_err(|e| e.to_string()): {lib}"
    );
}

#[test]
fn lib_rs_emits_mirror_enum_per_ir_enum() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".into(),
            rust_path: "demo::Status".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
                EnumVariant {
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(lib.contains("#[frb(mirror(Status))]"), "missing mirror for Status: {lib}");
    assert!(lib.contains("pub enum Status {"), "missing Status mirror enum: {lib}");
    assert!(lib.contains("Active,"), "missing Active variant: {lib}");
    assert!(lib.contains("Inactive,"), "missing Inactive variant: {lib}");
}

#[test]
fn build_rs_is_emitted() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let build = find_file(&files, "packages/dart/rust/build.rs").expect("build.rs not found");

    assert!(build.contains("fn main()"), "missing fn main(): {build}");
}

#[test]
fn frb_yaml_is_emitted_with_module_name() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let yaml =
        find_file(&files, "packages/dart/rust/flutter_rust_bridge.yaml").expect("flutter_rust_bridge.yaml not found");

    // FRB v2 schema: `rust_root` (crate dir) + `dart_output` (output dir).
    // The v1 keys `rust_input` / `rust_output` were removed.
    assert!(yaml.contains("rust_root: ."), "missing rust_root: {yaml}");
    assert!(
        yaml.contains("demo_crate_bridge_generated"),
        "missing dart output path with module name: {yaml}"
    );
    assert!(!yaml.contains("rust_input:"), "v1 rust_input key should not be emitted: {yaml}");
    assert!(!yaml.contains("rust_output:"), "v1 rust_output key should not be emitted: {yaml}");
}

#[test]
fn generate_bindings_returns_dart_file_plus_rust_crate_files() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();

    // Should have: 1 .dart + Cargo.toml + lib.rs + build.rs + flutter_rust_bridge.yaml = 5
    assert_eq!(files.len(), 5, "expected 5 generated files, got {}", files.len());

    let has_dart = files.iter().any(|f| f.path.to_string_lossy().ends_with(".dart") && !f.path.to_string_lossy().contains("rust/"));
    assert!(has_dart, "missing Dart wrapper file");
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
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
    }
}

fn make_trait(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,

        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
    }
}

fn make_config_with_bridge(bridge_trait_name: &str) -> AlefConfig {
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: bridge_trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
    }];
    config
}

/// A trait with a single synchronous unit-returning method should produce an opaque struct,
/// a trait impl that block_on's the DartFnFuture callback, and a factory function.
#[test]
fn lib_rs_emits_frb_trait_bridge_for_sync_method_trait() {
    let trait_def = make_trait(
        "Validator",
        "demo_crate::Validator",
        vec![make_method(
            "validate",
            vec![make_param("input", TypeRef::String)],
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
        )],
    );
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_config_with_bridge("Validator");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Opaque struct with DartFnFuture callback field
    assert!(lib.contains("#[frb(opaque)]"), "missing #[frb(opaque)]: {lib}");
    assert!(lib.contains("pub struct ValidatorDartImpl"), "missing opaque struct: {lib}");
    assert!(lib.contains("DartFnFuture"), "missing DartFnFuture callback type: {lib}");
    assert!(lib.contains("validate:"), "missing validate field: {lib}");

    // Trait impl block
    assert!(
        lib.contains("impl demo_crate::Validator for ValidatorDartImpl"),
        "missing trait impl: {lib}"
    );
    assert!(lib.contains("fn validate("), "missing validate method: {lib}");
    assert!(lib.contains("block_on"), "missing block_on for async bridging: {lib}");

    // Factory function
    assert!(lib.contains("pub fn create_validator_dart_impl("), "missing factory fn: {lib}");

    // Trait defs should NOT be emitted as mirror structs
    assert!(
        !lib.contains("#[frb(mirror(Validator))]"),
        "trait should not be emitted as mirror struct: {lib}"
    );
}

/// A trait with an async method should still produce the same structure (async-to-sync via block_on).
#[test]
fn lib_rs_emits_frb_trait_bridge_for_async_method_trait() {
    let trait_def = make_trait(
        "OcrBackend",
        "demo_crate::OcrBackend",
        vec![make_method(
            "extract_text",
            vec![make_param("data", TypeRef::Bytes)],
            TypeRef::String,
            true, // async method
        )],
    );
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_config_with_bridge("OcrBackend");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Opaque struct
    assert!(lib.contains("#[frb(opaque)]"), "missing #[frb(opaque)]: {lib}");
    assert!(lib.contains("pub struct OcrBackendDartImpl"), "missing opaque struct: {lib}");
    assert!(lib.contains("flutter_rust_bridge::DartFnFuture<String>"), "missing DartFnFuture<String>: {lib}");
    assert!(lib.contains("extract_text:"), "missing extract_text field: {lib}");

    // Factory function exists
    assert!(
        lib.contains("pub fn create_ocr_backend_dart_impl("),
        "missing factory fn: {lib}"
    );

    // Trait impl uses async_trait and awaits the DartFnFuture directly in async fn
    assert!(
        lib.contains("impl demo_crate::OcrBackend for OcrBackendDartImpl"),
        "missing trait impl: {lib}"
    );
    assert!(lib.contains("#[async_trait::async_trait]"), "async trait must use async_trait macro: {lib}");
    assert!(lib.contains(".await"), "async method must await the DartFnFuture: {lib}");
    assert!(lib.contains("fn extract_text("), "missing extract_text impl: {lib}");
}
