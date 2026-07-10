use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> alef::core::config::ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_field(name: &str, ty: TypeRef, optional: bool, default: Option<String>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default,
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
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_enum(name: &str, variants: Vec<EnumVariant>, serde_tag: Option<String>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        methods: vec![],
        doc: String::new(),
        variants,
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

#[test]
fn test_java_serde_default_sealed_interface_with_fields_uses_null() {
    // When a non-optional sealed interface field has #[serde(default)], and the default variant

    let backend = JavaBackend;

    let model_enum = make_enum(
        "EmbeddingModelType",
        vec![
            EnumVariant {
                name: "Preset".to_string(),
                fields: vec![FieldDef {
                    name: "name".to_string(),
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
                doc: "Use a preset model configuration (recommended)".to_string(),
                is_default: true,
                serde_rename: None,
                is_tuple: true,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![
                    FieldDef {
                        name: "model_id".to_string(),
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
                    },
                    FieldDef {
                        name: "dimensions".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U64),
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
                    },
                ],
                doc: "Use a custom ONNX model from HuggingFace".to_string(),
                is_default: false,
                serde_rename: None,
                is_tuple: true,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        Some("type".to_string()),
    );

    // Define EmbeddingConfig with a non-optional EmbeddingModelType field that has #[serde(default)]
    let model_field = make_field(
        "model",
        TypeRef::Named("EmbeddingModelType".to_string()),
        false,
        Some("/* serde(default) */".to_string()),
    );

    let config_type = make_type("EmbeddingConfig", vec![model_field]);

    let api = ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![config_type],
        functions: vec![],
        enums: vec![model_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.demo"
"#,
    );

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "generation failed: {:?}", result.err());
    let files = result.unwrap();

    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("EmbeddingConfig.java"))
        .expect("EmbeddingConfig.java not generated");

    let content = &config_file.content;

    assert!(
        content.contains("@Nullable private EmbeddingModelType model;"),
        "Builder field should be nullable without an invalid default. Got:\n{content}"
    );

    assert!(
        !content.contains("new EmbeddingModelType.Preset()"),
        "Builder field should not try to instantiate Preset() without arguments, but got:\n{content}"
    );

    assert!(
        content.contains("withModel"),
        "Builder should have withModel setter, but got:\n{content}"
    );
}

#[test]
fn test_java_serde_default_sealed_interface_zero_field_variant_uses_new() {
    let backend = JavaBackend;

    let status_enum = make_enum(
        "Status",
        vec![
            EnumVariant {
                name: "Pending".to_string(),
                fields: vec![],
                doc: "Pending status".to_string(),
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
                name: "Complete".to_string(),
                fields: vec![],
                doc: "Complete status".to_string(),
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
        Some("type".to_string()),
    );

    let status_field = make_field(
        "status",
        TypeRef::Named("Status".to_string()),
        false,
        Some("/* serde(default) */".to_string()),
    );

    let config_type = make_type("StatusConfig", vec![status_field]);

    let api = ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![config_type],
        functions: vec![],
        enums: vec![status_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.demo"
"#,
    );

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "generation failed: {:?}", result.err());
    let files = result.unwrap();

    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("StatusConfig.java"))
        .expect("StatusConfig.java not generated");

    let content = &config_file.content;

    assert!(
        content.contains("new Status.Pending()"),
        "Builder field should instantiate zero-field variant with new Status.Pending(). Got:\n{content}"
    );
}
