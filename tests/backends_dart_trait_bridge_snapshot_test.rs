use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, TraitBridgeConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
    TypeDef, TypeRef,
};

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

fn make_method(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        rust_path: format!("demo::{}", name),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async,
        error_type: error_type.map(String::from),
        receiver: Some(ReceiverKind::Ref),
        doc: format!("Mock method: {}", name),
        cfg: None,
        is_virtual: true,
        has_default_impl: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        sanitized: false,
        return_sanitized: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn make_api_with_traits() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            // Async trait: OcrBackend
            TypeDef {
                name: "OcrBackend".to_string(),
                rust_path: "demo::OcrBackend".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![
                    make_method(
                        "process_image",
                        vec![
                            make_param("image_bytes", TypeRef::Bytes),
                            make_param("lang", TypeRef::String),
                        ],
                        TypeRef::String,
                        true,
                        Some("DemoError"),
                    ),
                    make_method(
                        "supports_language",
                        vec![make_param("lang", TypeRef::String)],
                        TypeRef::Primitive(PrimitiveType::Bool),
                        true,
                        None,
                    ),
                ],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "OCR backend trait.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec!["Plugin".to_string()],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            // Sync trait: Validator
            TypeDef {
                name: "Validator".to_string(),
                rust_path: "demo::Validator".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![make_method(
                    "validate",
                    vec![make_param("input", TypeRef::String)],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    false,
                    None,
                )],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Validation trait.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec!["Plugin".to_string()],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            // Plugin super-trait (required by both)
            TypeDef {
                name: "Plugin".to_string(),
                rust_path: "demo::Plugin".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Plugin trait.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    }
}

fn make_config_with_trait_bridges() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
version_from = "/nonexistent/Cargo.toml"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "demo::Plugin"
registry_getter = "demo::registry::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"

[[crates.trait_bridges]]
trait_name = "Validator"
super_trait = "demo::Plugin"
registry_getter = "demo::registry::get_validator_registry"
register_fn = "register_validator"
unregister_fn = "unregister_validator"
clear_fn = "clear_validators"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve()
        .expect("test config must resolve")
        .remove(0)
}

#[test]
fn snapshot_trait_bridge_registration_methods() {
    let api = make_api_with_traits();
    let config = make_config_with_trait_bridges();
    let files = DartBackend.generate_bindings(&api, &config).unwrap();

    // The main bridge class file should contain trait registration methods
    let main_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("demo.dart"))
        .expect("Main bridge file must exist");

    insta::assert_snapshot!(
        "snapshot_trait_bridge__main_bridge_class",
        &main_file.content
    );

    // The traits file should define abstract classes
    let traits_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("traits.dart"))
        .expect("Traits file must exist");

    insta::assert_snapshot!(
        "snapshot_trait_bridge__abstract_classes",
        &traits_file.content
    );
}

#[test]
fn snapshot_trait_bridge_exclusion() {
    // Test that excluded trait bridges are not generated
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
version_from = "/nonexistent/Cargo.toml"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
registry_getter = "demo::registry::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
exclude_languages = ["dart"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let api = make_api_with_traits();
    let files = DartBackend.generate_bindings(&api, &config).unwrap();

    // The main bridge file should NOT contain excluded trait bridge methods
    let main_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("demo.dart"))
        .expect("Main bridge file must exist");

    // Should not have registerOcrBackend or any OcrBackend-related methods
    assert!(
        !main_file.content.contains("registerOcrBackend"),
        "Excluded trait bridge should not be in bridge class"
    );

    // Traits file should not contain the excluded trait
    let has_traits_file = files.iter().any(|f| f.path.to_string_lossy().ends_with("traits.dart"));
    if has_traits_file {
        let traits_file = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("traits.dart"))
            .unwrap();
        assert!(
            !traits_file.content.contains("class OcrBackend"),
            "Excluded trait should not be in traits file"
        );
    }
}
