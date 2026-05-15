use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, TraitBridgeConfig, new_config::NewAlefConfig};
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
        serde_rename: None,
        serde_flatten: false,
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

fn make_opaque_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo_crate::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
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

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_from_toml(extra: &str) -> ResolvedCrateConfig {
    let toml = format!(
        r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
{extra}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

/// Helper: generate all files and find the one at the given suffix.
fn find_file<'a>(files: &'a [alef_core::backend::GeneratedFile], suffix: &str) -> Option<&'a str> {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(suffix))
        .map(|f| f.content.as_str())
}

#[test]
fn cargo_toml_uses_dart_specific_extra_dependencies() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_from_toml(
        r#"
[crates.extra_dependencies]
shared-crate = { path = "../shared-crate" }

[crates.dart.extra_dependencies]
shared-crate = { path = "../../../crates/shared-crate" }
"#,
    );

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        cargo.contains(r#"shared-crate = { path = "../../../crates/shared-crate" }"#),
        "Cargo.toml must use Dart-specific dependency override: {cargo}"
    );
    assert!(
        !cargo.contains(r#"shared-crate = { path = "../shared-crate" }"#),
        "got: {cargo}"
    );
}

#[test]
fn lib_rs_converts_named_map_values_from_core_to_mirror() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type("SecuritySchemeInfo", vec![]),
            make_type(
                "OpenApiConfig",
                vec![make_field(
                    "security_schemes",
                    TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Named("SecuritySchemeInfo".to_string())),
                    ),
                    false,
                )],
            ),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains(".map(|(k, v)| (k.into(), SecuritySchemeInfo::from(v)))"),
        "named map values must convert to mirror structs, not JSON strings: {lib}"
    );
    assert!(
        !lib.contains(".map(|(k, v)| (k.into(), serde_json::to_string(&v).unwrap_or_default()))"),
        "named map values must not serialize to String: {lib}"
    );
}

#[test]
fn opaque_methods_convert_optional_ref_string_json_params_and_returns() {
    let mut description = make_method(
        "get_description",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
    );
    description.returns_ref = true;

    let mut operation_name = make_param("operation_name", TypeRef::String);
    operation_name.optional = true;
    operation_name.is_ref = true;
    let mut variables = make_param("variables", TypeRef::Json);
    variables.optional = true;
    let mut graphql = make_method(
        "graphql",
        vec![make_param("query", TypeRef::String), variables, operation_name],
        TypeRef::Named("ResponseSnapshot".to_string()),
        true,
    );
    graphql.error_type = Some("SnapshotError".to_string());

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![make_opaque_type("TestClient", vec![description, graphql])],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("(|v: Option<&str>| v.map(|s| s.to_string()))(self.inner.get_description())"),
        "optional borrowed string returns must be owned for FRB: {lib}"
    );
    assert!(
        lib.contains("variables.as_deref().and_then(|s| serde_json::from_str(s).ok())"),
        "optional JSON string params must deserialize before core calls: {lib}"
    );
    assert!(
        lib.contains("operation_name.as_deref()"),
        "optional borrowed string params must use as_deref: {lib}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
    assert!(
        cargo.contains("flutter_rust_bridge"),
        "missing flutter_rust_bridge dep: {cargo}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // FRB v2: ordinary public functions need no annotation; bare `#[frb]` is rejected.
    assert!(
        !lib.contains("#[frb]\npub fn"),
        "bare #[frb] on fn is invalid in v2: {lib}"
    );
    assert!(lib.contains("pub fn greet_user"), "missing greet_user fn: {lib}");
    assert!(lib.contains("user_name: String"), "missing user_name param: {lib}");
    // rust_path resolution: call site uses the full module path, not the bare fn name.
    assert!(
        lib.contains("demo::greet_user("),
        "should call demo::greet_user via rust_path: {lib}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("pub async fn fetch_data"),
        "missing async fn keyword: {lib}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("#[frb(mirror(Status))]"),
        "missing mirror for Status: {lib}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let yaml =
        find_file(&files, "packages/dart/rust/flutter_rust_bridge.yaml").expect("flutter_rust_bridge.yaml not found");

    // FRB v2 schema: `rust_root` (crate dir) + `rust_input` (module path) + `dart_output`
    // (output dir). The CLI requires `rust_input` — it points at the crate root because
    // the alef-generated dart Rust crate places its entire API surface at `lib.rs`.
    // The v1 `rust_output` key was removed and must not be emitted.
    assert!(yaml.contains("rust_root: ."), "missing rust_root: {yaml}");
    assert!(yaml.contains("rust_input: crate"), "missing rust_input: crate: {yaml}");
    assert!(
        yaml.contains("demo_crate_bridge_generated"),
        "missing dart output path with module name: {yaml}"
    );
    assert!(
        !yaml.contains("rust_output:"),
        "v1 rust_output key should not be emitted: {yaml}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();

    // Should have: 1 .dart + Cargo.toml + lib.rs + build.rs + flutter_rust_bridge.yaml + frb_generated.rs = 6
    assert_eq!(files.len(), 6, "expected 6 generated files, got {}", files.len());

    let has_dart = files
        .iter()
        .any(|f| f.path.to_string_lossy().ends_with(".dart") && !f.path.to_string_lossy().contains("rust/"));
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

fn make_config_with_bridge(bridge_trait_name: &str) -> ResolvedCrateConfig {
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: bridge_trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_with_bridge("Validator");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Opaque struct with DartFnFuture callback field
    assert!(lib.contains("#[frb(opaque)]"), "missing #[frb(opaque)]: {lib}");
    assert!(
        lib.contains("pub struct ValidatorDartImpl"),
        "missing opaque struct: {lib}"
    );
    assert!(
        lib.contains("DartFnFuture"),
        "missing DartFnFuture callback type: {lib}"
    );
    assert!(lib.contains("validate:"), "missing validate field: {lib}");

    // Trait impl block
    assert!(
        lib.contains("impl demo_crate::Validator for ValidatorDartImpl"),
        "missing trait impl: {lib}"
    );
    assert!(lib.contains("fn validate("), "missing validate method: {lib}");
    assert!(lib.contains("block_on"), "missing block_on for async bridging: {lib}");

    // Factory function
    assert!(
        lib.contains("pub fn create_validator_dart_impl("),
        "missing factory fn: {lib}"
    );

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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_with_bridge("OcrBackend");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Opaque struct
    assert!(lib.contains("#[frb(opaque)]"), "missing #[frb(opaque)]: {lib}");
    assert!(
        lib.contains("pub struct OcrBackendDartImpl"),
        "missing opaque struct: {lib}"
    );
    assert!(
        lib.contains("flutter_rust_bridge::DartFnFuture<String>"),
        "missing DartFnFuture<String>: {lib}"
    );
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
    assert!(
        lib.contains("#[async_trait::async_trait]"),
        "async trait must use async_trait macro: {lib}"
    );
    assert!(
        lib.contains(".await"),
        "async method must await the DartFnFuture: {lib}"
    );
    assert!(lib.contains("fn extract_text("), "missing extract_text impl: {lib}");
}

/// When the bridge config sets `register_fn` + `registry_getter`, the codegen
/// must emit a `pub fn register_<trait>(...) -> Result<(), String>` forwarder
/// that wraps the user's `{Trait}DartImpl` in `Arc::new(...)` and inserts it
/// into the configured registry. FRB auto-bridges this `pub fn` for Dart.
#[test]
fn lib_rs_emits_register_forwarder_when_register_fn_configured() {
    let trait_def = make_trait(
        "OcrBackend",
        "demo_crate::OcrBackend",
        vec![make_method(
            "supports_language",
            vec![make_param("lang", TypeRef::String)],
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("demo_crate::plugins::registry::get_ocr_backend_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Register forwarder takes the FRB-bridged opaque struct and returns Result<(), String>.
    assert!(
        lib.contains("pub fn register_ocr_backend(impl_: OcrBackendDartImpl) -> Result<(), String>"),
        "missing register forwarder signature: {lib}"
    );
    assert!(
        lib.contains("std::sync::Arc<dyn demo_crate::plugins::OcrBackend>"),
        "register forwarder must wrap impl_ as Arc<dyn Trait>: {lib}"
    );
    assert!(
        lib.contains("demo_crate::plugins::registry::get_ocr_backend_registry()"),
        "register forwarder must call the configured registry getter: {lib}"
    );
    assert!(
        lib.contains("registry.register(arc).map_err(|e| e.to_string())"),
        "register forwarder must register the Arc and stringify errors: {lib}"
    );

    // Unregister forwarder takes the plugin name and returns Result<(), String>.
    assert!(
        lib.contains("pub fn unregister_ocr_backend(name: String) -> Result<(), String>"),
        "missing unregister forwarder signature: {lib}"
    );
    assert!(
        lib.contains("registry.remove(&name).map_err(|e| e.to_string())"),
        "unregister forwarder must call registry.remove(&name) and stringify errors: {lib}"
    );
}

/// When `register_fn` is unset, no forwarder is emitted — the bridge keeps
/// only the wrapper struct, trait impl, and factory.
#[test]
fn lib_rs_does_not_emit_register_forwarder_without_register_fn() {
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    // make_config_with_bridge() leaves register_fn = None.
    let config = make_config_with_bridge("Validator");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        !lib.contains("pub fn register_"),
        "no register forwarder should be emitted when register_fn is unset: {lib}"
    );
    assert!(
        !lib.contains("pub fn unregister_"),
        "no unregister forwarder should be emitted when unregister_fn is unset: {lib}"
    );
}

/// `register_extra_args` must be appended to the `registry.register(arc)` call.
#[test]
fn lib_rs_register_forwarder_appends_register_extra_args() {
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Validator".to_string(),
        super_trait: None,
        registry_getter: Some("demo_crate::plugins::registry::get_validator_registry".to_string()),
        register_fn: Some("register_validator".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: Some("0".to_string()),
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("registry.register(arc, 0)"),
        "register forwarder must append register_extra_args: {lib}"
    );
}

/// When `clear_fn` and `registry_getter` are both set, the codegen must emit a
/// `pub fn clear_*() -> Result<(), String>` Rust-side forwarder.  FRB auto-bridges
/// it so Dart sees it as `Future<void> clearXxxs()`.
#[test]
fn lib_rs_emits_clear_forwarder_when_clear_fn_configured() {
    let trait_def = make_trait(
        "OcrBackend",
        "demo_crate::OcrBackend",
        vec![make_method(
            "supports_language",
            vec![make_param("lang", TypeRef::String)],
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("demo_crate::plugins::registry::get_ocr_backend_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Clear forwarder takes no args and returns Result<(), String>.
    assert!(
        lib.contains("pub fn clear_ocr_backends() -> Result<(), String>"),
        "missing clear forwarder signature: {lib}"
    );
    assert!(
        lib.contains("registry.clear().map_err(|e| e.to_string())"),
        "clear forwarder must call registry.clear() and stringify errors: {lib}"
    );
    assert!(
        lib.contains("demo_crate::plugins::registry::get_ocr_backend_registry()"),
        "clear forwarder must call the configured registry getter: {lib}"
    );
}

/// When `clear_fn` is unset, no clear forwarder is emitted.
#[test]
fn lib_rs_does_not_emit_clear_forwarder_without_clear_fn() {
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_with_bridge("Validator");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        !lib.contains("pub fn clear_"),
        "no clear forwarder should be emitted when clear_fn is unset: {lib}"
    );
}

#[test]
fn cargo_toml_has_license_field() {
    use alef_core::config::ScaffoldConfig;

    let mut config = make_config();
    config.scaffold = Some(ScaffoldConfig {
        description: Some("Demo library".to_string()),
        license: Some("Apache-2.0".to_string()),
        repository: None,
        homepage: None,
        authors: vec![],
        keywords: vec![],
        generated_header: None,
        precommit: None,
        cargo: None,
    });

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        cargo.contains("license = \"Apache-2.0\""),
        "Cargo.toml must include license field; got:\n{cargo}"
    );
}

#[test]
fn cargo_toml_license_defaults_to_mit_when_scaffold_absent() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        cargo.contains("license = \"MIT\""),
        "Cargo.toml must default license to MIT when scaffold config is absent; got:\n{cargo}"
    );
}

#[test]
fn cargo_toml_does_not_include_anyhow_without_trait_bridges() {
    // Regression: anyhow was hardcoded in extra_deps even when no trait bridges are
    // configured, causing cargo-machete to fail on the generated Dart binding crate.
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        !cargo.contains("anyhow"),
        "Cargo.toml must not list anyhow when no trait bridges are configured (unused dep); got:\n{cargo}"
    );
}

#[test]
fn cargo_toml_does_not_include_anyhow_with_trait_bridges() {
    // Regression: anyhow was included in extra_deps alongside tokio and async-trait when
    // trait bridges are configured, but lib.rs never imports or uses anyhow — the bridge
    // impl uses source_crate::Result directly. cargo-machete fails on this unused dep.
    let trait_def = make_trait(
        "OcrBackend",
        "demo_crate::OcrBackend",
        vec![make_method(
            "extract_text",
            vec![make_param("data", TypeRef::Bytes)],
            TypeRef::String,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_with_bridge("OcrBackend");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        !cargo.contains("anyhow"),
        "Cargo.toml must not list anyhow even with trait bridges (lib.rs never uses it); got:\n{cargo}"
    );
    // tokio and async-trait ARE legitimately used by trait bridges
    assert!(
        cargo.contains("tokio"),
        "Cargo.toml must list tokio for trait bridges: {cargo}"
    );
    assert!(
        cargo.contains("async-trait"),
        "Cargo.toml must list async-trait for trait bridges: {cargo}"
    );
}

#[test]
fn cargo_toml_does_not_include_serde_json() {
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let cargo = find_file(&files, "packages/dart/rust/Cargo.toml").expect("Cargo.toml not found");

    assert!(
        !cargo.contains("serde_json"),
        "Cargo.toml must not list serde_json (unused dep); got:\n{cargo}"
    );
}

/// When a function name appears in `[crates.dart].stub_methods`, the generated
/// bridge fn body must be replaced with `unimplemented!()` rather than attempting
/// argument conversion. The function signature (params + return type) must still
/// be emitted so the FRB codegen can see the function.
#[test]
fn lib_rs_stub_methods_emits_unimplemented_body() {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.dart]
stub_methods = ["process_bytes_batch"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "process_bytes_batch".into(),
                rust_path: "demo::process_bytes_batch".into(),
                original_rust_path: String::new(),
                params: vec![make_param("items", TypeRef::Vec(Box::new(TypeRef::Bytes)))],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
            FunctionDef {
                name: "greet".into(),
                rust_path: "demo::greet".into(),
                original_rust_path: String::new(),
                params: vec![make_param("name", TypeRef::String)],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // The stub function must still be present with its signature.
    assert!(
        lib.contains("pub fn process_bytes_batch"),
        "stub fn must still be emitted: {lib}"
    );
    // The body must be unimplemented!(), not a real call.
    assert!(
        lib.contains("unimplemented!"),
        "stub fn body must contain unimplemented!(): {lib}"
    );
    assert!(
        !lib.contains("demo::process_bytes_batch("),
        "stub fn must NOT call the core fn: {lib}"
    );

    // Non-stub functions must not be affected.
    assert!(lib.contains("pub fn greet"), "non-stub fn must still be emitted: {lib}");
    assert!(lib.contains("demo::greet("), "non-stub fn must call core fn: {lib}");
}

/// Opaque method param: `Named` with `is_ref = true` must borrow the converted value
/// so the core method signature `fn process(&self, config: &ProcessConfig)` is satisfied.
#[test]
fn opaque_method_named_param_with_is_ref_passes_by_reference() {
    let mut config_param = make_param("config", TypeRef::Named("ProcessConfig".to_string()));
    config_param.is_ref = true;

    let mut process = make_method(
        "process",
        vec![config_param],
        TypeRef::Named("ProcessResult".to_string()),
        false,
    );
    process.error_type = Some("Error".to_string());

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type("ProcessConfig", vec![make_field("language", TypeRef::String, false)]),
            make_type("ProcessResult", vec![make_field("output", TypeRef::String, false)]),
            make_opaque_type("LanguageRegistry", vec![process]),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // The generated call must borrow the converted config (transmute via &ref, not by value).
    // ProcessConfig has no sanitized fields, so the transmute path is taken.
    assert!(
        lib.contains("transmute::<&ProcessConfig, &demo_crate::ProcessConfig>(&config)"),
        "is_ref Named param must be passed by reference (transmute &ref) to the core call: {lib}"
    );
    // Must not pass the owned value directly when is_ref is set.
    assert!(
        !lib.contains("transmute::<ProcessConfig, demo_crate::ProcessConfig>(config)"),
        "is_ref Named param must NOT be passed by owned transmute: {lib}"
    );
}

/// A struct field that is `sanitized: true` with `ty: TypeRef::String` and
/// `core_wrapper: CoreWrapper::Cow` (i.e. a `Cow<'static, str>` field that was
/// sanitized because the type resolver resolved `str` → `Named("str")` before
/// sanitize_unknown_types replaced it with `String`) must emit `v.<field>.into()`
/// in the `From<Mirror> for Core` impl — NOT `Default::default()`.
///
/// Regression test for: `ProcessConfig::language` being silently dropped when
/// converting from the dart mirror struct to the core struct.
#[test]
fn sanitized_string_cow_field_roundtrips_in_from_mirror_to_core_impl() {
    // Build a struct that mimics ProcessConfig: `language: Cow<'static, str>` was
    // extracted as TypeRef::String with sanitized=true and core_wrapper=Cow.
    let mut language_field = make_field("language", TypeRef::String, false);
    language_field.sanitized = true;
    language_field.core_wrapper = CoreWrapper::Cow;

    let config_type = TypeDef {
        name: "ProcessConfig".to_string(),
        rust_path: "demo_crate::ProcessConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![language_field],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
    };

    // A free function that takes ProcessConfig as input forces a From<Mirror> for Core impl.
    let process_fn = FunctionDef {
        name: "process".to_string(),
        params: vec![make_param("config", TypeRef::Named("ProcessConfig".to_string()))],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        return_sanitized: false,
        rust_path: "demo_crate::process".to_string(),
        original_rust_path: String::new(),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![config_type],
        functions: vec![process_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Must emit v.language.into() — not Default::default() — in the From impl body.
    assert!(
        lib.contains("language: v.language.into()"),
        "sanitized String+Cow field must emit v.language.into() in From<Mirror> for Core, got:\n{lib}"
    );
    assert!(
        !lib.contains("language: Default::default()"),
        "sanitized String+Cow field must NOT emit Default::default() for language:\n{lib}"
    );
}

/// Opaque method param: `Vec<String>` with `is_ref = true` must be bridged to `&[&str]`
/// (collect to `Vec<&str>` then auto-coerce at the call site).
#[test]
fn opaque_method_vec_string_param_with_is_ref_bridges_to_str_slice() {
    let mut names_param = make_param("names", TypeRef::Vec(Box::new(TypeRef::String)));
    names_param.is_ref = true;

    let ensure = make_method("ensure_languages", vec![names_param], TypeRef::Unit, false);
    let mut ensure_with_error = ensure;
    ensure_with_error.error_type = Some("Error".to_string());

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![make_opaque_type("DownloadManager", vec![ensure_with_error])],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    // Vec<String> must be converted to an iterator of &str slices for the core call.
    assert!(
        lib.contains("names.iter().map(|s| s.as_str()).collect::<Vec<_>>()"),
        "is_ref Vec<String> param must be bridged to &[&str] via iter().map(as_str).collect: {lib}"
    );
    // Must not pass the raw Vec<String> directly.
    assert!(
        !lib.contains("ensure_languages(names)"),
        "is_ref Vec<String> param must not be passed as raw Vec<String>: {lib}"
    );
}

/// A trait surfaced via `trait_bridges` whose methods return another trait by name
/// must NOT produce a `From<Trait> for SourceTrait` mirror-to-core impl. Trait types
/// cannot be constructed with `{}`, so the emitted block would fail to compile
/// (E0574 "expected struct, variant or union type, found trait"). The dart backend
/// iterates `types_needing_from_impl` to emit those impls and must filter out
/// `is_trait`/`is_opaque` entries — the seed set includes trait-bridge return-type
/// names so a bare membership check is insufficient.
#[test]
fn trait_bridge_return_type_does_not_emit_from_impl_for_trait() {
    let factory = make_method(
        "make_visitor",
        vec![],
        TypeRef::Named("MyVisitor".to_string()),
        false,
    );
    let trait_def = make_trait("MyFactory", "demo_crate::MyFactory", vec![factory]);
    let visitor_trait = make_trait("MyVisitor", "demo_crate::MyVisitor", vec![]);

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def, visitor_trait],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };
    let config = make_config_with_bridge("MyFactory");
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        !lib.contains("impl From<MyVisitor>"),
        "must not emit `impl From<MyVisitor>` for a trait type: {lib}"
    );
    assert!(
        !lib.contains("demo_crate::MyVisitor {}"),
        "must not construct a trait with `{{}}` literal: {lib}"
    );
}

/// A field that is sanitized down to `TypeRef::String` because its real core type is
/// not in the API surface (e.g. `Option<BoundingBox>` mirrored as `Option<String>`,
/// `core_wrapper = None`) must use `Default::default()` in the From<Mirror> for Core
/// impl. Only the `core_wrapper == Cow` case is safely round-trippable via `.into()`,
/// because that's the genuine `Cow<'static, str>`-extracted-as-String case.
#[test]
fn sanitized_string_non_cow_field_falls_back_to_default_in_from_mirror_to_core_impl() {
    let mut bounding_box_field = make_field("bounding_box", TypeRef::String, true);
    bounding_box_field.sanitized = true;
    bounding_box_field.core_wrapper = CoreWrapper::None;

    let annotation_type = TypeDef {
        name: "PdfAnnotation".to_string(),
        rust_path: "demo_crate::PdfAnnotation".to_string(),
        original_rust_path: String::new(),
        fields: vec![bounding_box_field],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
    };

    let process_fn = FunctionDef {
        name: "process".to_string(),
        params: vec![make_param("annotation", TypeRef::Named("PdfAnnotation".to_string()))],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        return_sanitized: false,
        rust_path: "demo_crate::process".to_string(),
        original_rust_path: String::new(),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![annotation_type],
        functions: vec![process_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = DartBackend.generate_bindings(&api, &make_config()).unwrap();
    let lib = find_file(&files, "packages/dart/rust/src/lib.rs").expect("lib.rs not found");

    assert!(
        lib.contains("bounding_box: Default::default()"),
        "sanitized String field with core_wrapper=None must emit Default::default(): {lib}"
    );
    assert!(
        !lib.contains("bounding_box: v.bounding_box.map(Into::into)"),
        "sanitized String field with core_wrapper=None must NOT emit .map(Into::into): {lib}"
    );
    assert!(
        !lib.contains("bounding_box: v.bounding_box.into()"),
        "sanitized String field with core_wrapper=None must NOT emit .into(): {lib}"
    );
}
