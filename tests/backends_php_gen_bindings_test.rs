use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::*;

/// Helper to create a config with a specific extension name for namespace testing.
#[allow(dead_code)]
fn make_config_with_extension(extension_name: &str) -> ResolvedCrateConfig {
    let toml = format!(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "{extension_name}"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

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

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_with_php_output(output_path: &std::path::Path) -> ResolvedCrateConfig {
    let output = output_path.to_string_lossy();
    let toml = format!(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.output]
php = "{output}"

[crates.php]
extension_name = "test_lib"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_with_php_excludes() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"
exclude_functions = ["hidden_function"]
exclude_types = ["HiddenConfig"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn php_native_and_facade_allow_null_default_config_param() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        }],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "mime_type".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");
    assert!(
        lib.content.contains("config: Option<&ExtractionConfig>"),
        "native PHP method must accept omitted/null default config:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("config_core.unwrap_or_default()"),
        "native PHP method must materialize Default for null config:\n{}",
        lib.content
    );

    let stubs = backend.generate_type_stubs(&api, &make_config()).unwrap();
    let stub = &stubs[0].content;
    assert!(
        stub.contains("?\\Test\\Lib\\ExtractionConfig $config = null"),
        "PHP facade stub must allow null default config:\n{stub}"
    );
}

#[test]
fn php_serde_defaults_are_generated_from_typed_default_metadata() {
    let backend = PhpBackend;
    let mut max_items = make_field("max_items", TypeRef::Primitive(PrimitiveType::Usize), false);
    max_items.typed_default = Some(DefaultValue::IntLiteral(500));
    let mut enabled = make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false);
    enabled.typed_default = Some(DefaultValue::BoolLiteral(true));

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Limits".to_string(),
            rust_path: "test_lib::Limits".to_string(),
            original_rust_path: String::new(),
            fields: vec![max_items, enabled],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        }],
        ..ApiSurface::default()
    };

    let root = tempfile::tempdir().expect("tempdir");
    let output_dir = root.path().join("crates/test-lib-php/src");
    std::fs::create_dir_all(&output_dir).expect("create output dir");
    std::fs::write(
        root.path().join("crates/test-lib-php/Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write Cargo.toml");
    let config = make_config_with_php_output(&output_dir);
    let files = backend
        .generate_bindings(&api, &config)
        .expect("PHP bindings must generate");
    let lib = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("lib.rs generated");

    assert!(
        lib.content
            .contains("#[serde(default = \"crate::serde_defaults::limits_max_items\")]"),
        "typed default metadata must drive serde default helpers:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("pub fn limits_max_items() -> i64 { 500 }"),
        "integer typed default must emit a matching helper:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("pub fn limits_enabled() -> bool { true }"),
        "boolean typed default must emit a matching helper:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("security_limits"),
        "serde default helpers must not branch on downstream type names:\n{}",
        lib.content
    );
}

#[test]
fn test_basic_generation() {
    let backend = PhpBackend;

    // Create test API surface
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Extraction configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "test_lib::extract_file_sync".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::String,
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
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("Config".to_string()),
                    optional: true,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text from file".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![EnumDef {
            name: "OcrBackend".to_string(),
            rust_path: "test_lib::OcrBackend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    doc: "Tesseract OCR".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR backend".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: "Available OCR backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");

    // Check for lib.rs file
    let file_names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    assert!(
        file_names.iter().any(|f| f.contains("lib.rs")),
        "Should generate lib.rs"
    );

    // Verify content contains PHP-specific markers
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    // Should contain #[php_class] for types
    assert!(
        lib_rs.content.contains("#[php_class]"),
        "Should contain #[php_class] marker for classes"
    );

    // Functions are generated as static methods in a *Api class (avoids inventory crate issue on macOS)
    assert!(
        lib_rs.content.contains("Api") && lib_rs.content.contains("#[php_impl]"),
        "Should contain Api class with #[php_impl] for functions"
    );

    // Should contain ext_php_rs imports
    assert!(lib_rs.content.contains("ext_php_rs"), "Should import ext_php_rs");
}

#[test]
fn type_stubs_honor_php_excludes_and_enum_wire_values() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "HiddenConfig".to_string(),
            rust_path: "test_lib::HiddenConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("name", TypeRef::String, false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![FunctionDef {
            name: "hidden_function".to_string(),
            rust_path: "test_lib::hidden_function".to_string(),
            original_rust_path: String::new(),
            params: vec![],
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
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "test_lib::OutputFormat".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "PlainText".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
            }],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: Some("kebab-case".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = backend
        .generate_type_stubs(&api, &make_config_with_php_excludes())
        .unwrap()[0]
        .content
        .clone();
    assert!(
        !content.contains("HiddenConfig"),
        "excluded type leaked into stubs:\n{content}"
    );
    assert!(
        !content.contains("hiddenFunction"),
        "excluded function leaked into stubs:\n{content}"
    );
    assert!(
        content.contains("case PlainText = 'plain-text';"),
        "enum stub must use serde wire value:\n{content}"
    );
}

#[test]
fn test_type_mapping() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), false),
                make_field("list_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // Should have proper field definitions with types
    assert!(content.contains("u32_val"), "Should contain u32_val field");
    assert!(content.contains("i64_val"), "Should contain i64_val field");
    assert!(content.contains("string_val"), "Should contain string_val field");
    assert!(
        content.contains("opt_string") || content.contains("Option"),
        "Should handle optional types"
    );
    assert!(
        content.contains("list_val") || content.contains("Vec"),
        "Should handle vec types"
    );
}

#[test]
fn test_enum_generation() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: "Inactive status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: "Processing status".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // Enum should generate constants for PHP
    assert!(
        content.contains("Pending") && content.contains("Active") && content.contains("Inactive"),
        "Should contain all enum variants"
    );
}

#[test]
fn test_generated_header() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // All files should have generated_header set to false (as per PHP backend code)
    for file in &files {
        assert!(
            !file.generated_header,
            "PHP backend files should have generated_header=false"
        );
    }
}

#[test]
fn test_methods_generation() {
    let backend = PhpBackend;

    // Create a type with methods
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::String, false)],
            methods: vec![
                MethodDef {
                    name: "process".to_string(),
                    params: vec![ParamDef {
                        name: "input".to_string(),
                        ty: TypeRef::String,
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
                    }],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Process input".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                MethodDef {
                    name: "from_id".to_string(),
                    params: vec![ParamDef {
                        name: "id".to_string(),
                        ty: TypeRef::String,
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
                    }],
                    return_type: TypeRef::Named("Processor".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Create from ID".to_string(),
                    receiver: None,
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
            ],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Text processor".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Method generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Check for #[php_impl] attribute for method blocks
    assert!(
        content.contains("#[php_impl]"),
        "Should contain #[php_impl] for method implementation"
    );

    // Check for method names in output
    assert!(content.contains("process"), "Should contain process method");
    assert!(content.contains("from_id"), "Should contain from_id static method");
}

#[test]
fn test_error_types() {
    let backend = PhpBackend;

    // Create error types with variants
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "risky_operation".to_string(),
            rust_path: "test_lib::risky_operation".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("ProcessError".to_string()),
            doc: "Operation that can fail".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    doc: "Resource not found".to_string(),
                    message_template: Some("resource not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input: {reason}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                },
            ],
            doc: "Errors during processing".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Error type generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Check that error converter function is generated
    assert!(
        content.contains("ProcessError") || content.contains("risky_operation"),
        "Should reference error type or function with error"
    );

    // Function with error_type should generate static method in Api class
    assert!(
        content.contains("risky_operation"),
        "Should generate method for function with error"
    );
}

#[test]
fn test_async_function() {
    let backend = PhpBackend;

    // Create an async function
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_data".to_string(),
            rust_path: "test_lib::fetch_data".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "url".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("FetchError".to_string()),
            doc: "Fetch data asynchronously".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "FetchError".to_string(),
            rust_path: "test_lib::FetchError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NetworkError".to_string(),
                fields: vec![],
                doc: "Network error".to_string(),
                message_template: Some("network failure".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            }],
            doc: "Fetch error".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Async function generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Async functions should generate a WORKER_RUNTIME for blocking
    assert!(
        content.contains("WORKER_RUNTIME") || content.contains("block_on") || content.contains("_async"),
        "Should contain async runtime support or _async function"
    );

    // Functions are generated as static methods in Api class
    assert!(
        content.contains("Api") && content.contains("#[php_impl]"),
        "Should contain Api class with #[php_impl] for async function"
    );

    // The PHP-facing method name must be camelCase so the userland facade and stubs
    // (which call `fetchData`) resolve correctly; the Rust fn ident stays snake_case.
    assert!(
        content.contains("#[php(name = \"fetchData\")]"),
        "Extension binding should expose the PHP method as camelCase `fetchData`; content:\n{content}"
    );
    assert!(
        content.contains("pub fn fetch_data("),
        "Rust fn ident should remain snake_case `fetch_data`; content:\n{content}"
    );
}

#[test]
fn test_opaque_type() {
    let backend = PhpBackend;

    // Create an opaque type
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "test_lib::Handle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "close".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Close the handle".to_string(),
                receiver: Some(ReceiverKind::Owned),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handle to resource".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Opaque type generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Opaque types should have Arc import
    assert!(content.contains("std::sync::Arc"), "Should import Arc for opaque types");

    // Should contain #[php_class] for opaque type
    assert!(
        content.contains("#[php_class]") && content.contains("Handle"),
        "Should contain #[php_class] for opaque Handle type"
    );

    // Should contain method implementation
    assert!(
        content.contains("close"),
        "Should contain close method for opaque Handle"
    );
}

#[test]
fn test_default_config() {
    let backend = PhpBackend;

    // Create a type with has_default: true
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Default config generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Type with has_default: true should derive Default or have constructor with defaults
    assert!(
        content.contains("Default") || content.contains("__construct") || content.contains("#[derive"),
        "Should handle default configuration type"
    );

    // Should contain Config type definition
    assert!(content.contains("Config"), "Should contain Config type");
}

#[test]
fn test_multiple_types_with_shared_error() {
    let backend = PhpBackend;

    // Create multiple types and functions sharing an error type
    let shared_error = ErrorDef {
        name: "SharedError".to_string(),
        rust_path: "test_lib::SharedError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "IoError".to_string(),
                fields: vec![],
                doc: "I/O error".to_string(),
                message_template: Some("I/O failed".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            },
            ErrorVariant {
                name: "ParseError".to_string(),
                fields: vec![],
                doc: "Parse error".to_string(),
                message_template: Some("Parse failed".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            },
        ],
        doc: "Shared error type".to_string(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Reader".to_string(),
                rust_path: "test_lib::Reader".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("path", TypeRef::String, false)],
                methods: vec![MethodDef {
                    name: "read".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: Some("SharedError".to_string()),
                    doc: "Read file".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "File reader".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            },
            TypeDef {
                name: "Parser".to_string(),
                rust_path: "test_lib::Parser".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("format", TypeRef::String, false)],
                methods: vec![MethodDef {
                    name: "parse".to_string(),
                    params: vec![ParamDef {
                        name: "content".to_string(),
                        ty: TypeRef::String,
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
                    }],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: Some("SharedError".to_string()),
                    doc: "Parse content".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Content parser".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![shared_error],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(
        result.is_ok(),
        "Generation with multiple types sharing error should succeed"
    );

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Should contain both types
    assert!(
        content.contains("Reader") && content.contains("Parser"),
        "Should contain both Reader and Parser types"
    );

    // Should contain #[php_class] for both
    let php_class_count = content.matches("#[php_class]").count();
    assert!(php_class_count >= 2, "Should have #[php_class] for both types");

    // Error should be referenced in both methods
    assert!(
        content.contains("SharedError") || (content.contains("read") && content.contains("parse")),
        "Should reference shared error or contain both methods"
    );
}

#[test]
fn test_generate_type_stubs_contains_exception_and_api_class() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![FunctionDef {
            name: "create_thing".to_string(),
            rust_path: "test_lib::create_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::Named("Config".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();

    assert!(!files.is_empty(), "Should generate stubs file");
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // Exception class must extend \RuntimeException to satisfy PHPStan as Throwable
    assert!(
        content.contains("class TestLibException extends \\RuntimeException"),
        "Exception should extend \\RuntimeException; content:\n{content}"
    );

    // Api class must exist as a static method holder for free functions
    assert!(
        content.contains("class TestLibApi"),
        "Should generate TestLibApi class; content:\n{content}"
    );

    // Api class methods must have fully-qualified return types
    assert!(
        content.contains("createThing") || content.contains("create_thing"),
        "Should have createThing method in TestLibApi; content:\n{content}"
    );

    // Stubs should be namespaced correctly
    assert!(
        content.contains("namespace Test\\Lib"),
        "Should use Test\\Lib namespace; content:\n{content}"
    );
}

#[test]
fn test_generate_public_api_delegates_to_api_class() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "do_work".to_string(),
            rust_path: "test_lib::do_work".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Do some work".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).unwrap();

    assert!(!files.is_empty(), "Should generate public API file");
    let facade = files.first().unwrap();
    let content = &facade.content;

    // The facade class must delegate to TestLibApi (not TestLib directly)
    assert!(
        content.contains("TestLibApi::doWork") || content.contains("TestLibApi::do_work"),
        "Facade should delegate to TestLibApi; content:\n{content}"
    );

    // @throws annotation must reference the exception class
    assert!(
        content.contains("@throws") && content.contains("TestLibException"),
        "Should have @throws annotation for TestLibException; content:\n{content}"
    );
}

#[test]
fn test_opaque_class_promotes_parameters_after_first_optional() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TestClient".to_string(),
            rust_path: "test_lib::TestClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "post".to_string(),
                params: vec![
                    ParamDef {
                        name: "path".to_string(),
                        ty: TypeRef::String,
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
                    },
                    ParamDef {
                        name: "json".to_string(),
                        ty: TypeRef::String,
                        optional: true,
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
                    },
                    ParamDef {
                        name: "multipart".to_string(),
                        ty: TypeRef::String,
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
                    },
                ],
                return_type: TypeRef::Named("ResponseSnapshot".to_string()),
                is_async: false,
                is_static: false,
                error_type: Some("Error".to_string()),
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
            }],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let client = files
        .iter()
        .find(|file| file.path.ends_with("TestClient.php"))
        .expect("public API should include TestClient.php");

    assert!(
        client
            .content
            .contains("post(string $path, ?string $json = null, ?string $multipart = null): ResponseSnapshot"),
        "opaque PHP class should keep PHP syntax valid when a required Rust param follows an optional one; content:\n{}",
        client.content
    );
}

#[test]
fn test_sanitized_function_generates_stub_not_direct_call() {
    // Regression test for functions whose return types were sanitized from unknown types
    // (e.g. tuples) to String/Vec<String>/Option<String>.  The PHP backend must NOT emit a
    // direct core call (which would be a type mismatch), but instead generate an unimplemented
    // stub body — consistent with the pyo3 and napi backends.
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            // Mimics `extension_ambiguity`: core returns Option<(&str, &[&str])>,
            // sanitized to Option<String> in the IR.
            FunctionDef {
                name: "extension_ambiguity".to_string(),
                rust_path: "test_lib::extension_ambiguity".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "ext".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            // Mimics `split_code`: core returns Vec<(usize, usize)>,
            // sanitized to Vec<String> in the IR.
            FunctionDef {
                name: "split_code".to_string(),
                rust_path: "test_lib::split_code".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Vec(Box::new(TypeRef::String)),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // The generated bodies must NOT contain a direct delegating call to the core function.
    // Sanitized functions emit unimplemented stubs instead.
    assert!(
        !content.contains("test_lib::extension_ambiguity("),
        "extension_ambiguity must not delegate to core (type mismatch); content:\n{content}"
    );
    assert!(
        !content.contains("test_lib::split_code("),
        "split_code must not delegate to core (type mismatch); content:\n{content}"
    );

    // Sanitized functions without error_type must emit type-appropriate default values,
    // NOT PhpException stubs (which would be a type mismatch for non-Result return types).
    // extension_ambiguity returns Option<String>: stub must be `None`
    assert!(
        content.contains("None"),
        "extension_ambiguity (Option<String>, no Result) should emit `None` stub; content:\n{content}"
    );
    // split_code returns Vec<String>: stub must be `Vec::new()`
    assert!(
        content.contains("Vec::new()"),
        "split_code (Vec<String>, no Result) should emit `Vec::new()` stub; content:\n{content}"
    );
    // Neither must be wrapped in a PhpException Err
    assert!(
        !content.contains("Err(ext_php_rs::exception::PhpException::default(\"Not implemented: extension_ambiguity"),
        "extension_ambiguity must not emit PhpException (no error_type); content:\n{content}"
    );
    assert!(
        !content.contains("Err(ext_php_rs::exception::PhpException::default(\"Not implemented: split_code"),
        "split_code must not emit PhpException (no error_type); content:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// PHP trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def_php(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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
    }
}

fn make_method_php(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
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
    }
}

fn make_async_method_php(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
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
    }
}

fn make_node_context_php() -> TypeDef {
    TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "my_lib::NodeContext".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("node_id", TypeRef::String, false)],
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
    }
}

fn make_visit_result_php() -> EnumDef {
    EnumDef {
        name: "VisitResult".to_string(),
        rust_path: "my_lib::VisitResult".to_string(),
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
            },
        ],
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
    }
}

fn make_api_php() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_node_context_php()],
        functions: vec![],
        enums: vec![make_visit_result_php()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_plugin_bridge_cfg_php(trait_name: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
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

fn make_visitor_bridge_cfg_php(trait_name: &str, type_alias: &str) -> alef::core::config::TraitBridgeConfig {
    alef::core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
    }
}

// ---------------------------------------------------------------------------
// PHP trait bridge tests
// ---------------------------------------------------------------------------

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

/// Regression test: tagged data enums with tuple variants holding distinct inner types
/// must produce per-variant flat field names, not a shared `_0` field that collapses all
/// variant types to the first one.  Mirrors the `Message` enum in sample-llm:
///   System(SystemMessage), User(UserMessage), Assistant(AssistantMessage)
/// The flat struct must have distinct fields `system`, `user`, `assistant` (not `_0`).
/// The From impls must reference those per-variant field names.
#[test]
fn test_tagged_data_enum_tuple_variants_get_distinct_fields() {
    let backend = PhpBackend;

    let message_enum = EnumDef {
        name: "Message".to_string(),
        rust_path: "test_lib::Message".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "System".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("system".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
            },
            EnumVariant {
                name: "User".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("UserMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("user".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
            },
            EnumVariant {
                name: "Assistant".to_string(),
                fields: vec![make_field("_0", TypeRef::Named("AssistantMessage".to_string()), false)],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("assistant".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
            },
        ],
        doc: "Chat message".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("role".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![message_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed: {:?}", result.err());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // The flat struct must NOT have a shared `_0` field.
    assert!(
        !content.contains("pub _0:"),
        "Flat struct must not have a shared `_0` field; each tuple variant needs its own field"
    );

    // The flat struct must have per-variant fields named after the variant (snake_case).
    assert!(
        content.contains("pub system:"),
        "Flat struct must have `system` field for System variant; content:\n{content}"
    );
    assert!(
        content.contains("pub user:"),
        "Flat struct must have `user` field for User variant; content:\n{content}"
    );
    assert!(
        content.contains("pub assistant:"),
        "Flat struct must have `assistant` field for Assistant variant; content:\n{content}"
    );

    // Each field must carry a distinct type.
    assert!(
        content.contains("Option<SystemMessage>"),
        "Field `system` must have type Option<SystemMessage>; content:\n{content}"
    );
    assert!(
        content.contains("Option<UserMessage>"),
        "Field `user` must have type Option<UserMessage>; content:\n{content}"
    );
    assert!(
        content.contains("Option<AssistantMessage>"),
        "Field `assistant` must have type Option<AssistantMessage>; content:\n{content}"
    );

    // The core→binding From impl must assign per-variant fields.
    assert!(
        content.contains("system: Some(_0.into())"),
        "core→binding From impl must assign to `system`; content:\n{content}"
    );
    assert!(
        content.contains("user: Some(_0.into())"),
        "core→binding From impl must assign to `user`; content:\n{content}"
    );

    // The binding→core From impl must read from per-variant flat fields.
    assert!(
        content.contains("val.system.map(Into::into)"),
        "binding→core From impl must read from `val.system`; content:\n{content}"
    );
    assert!(
        content.contains("val.user.map(Into::into)"),
        "binding→core From impl must read from `val.user`; content:\n{content}"
    );
    assert!(
        content.contains("val.assistant.map(Into::into)"),
        "binding→core From impl must read from `val.assistant`; content:\n{content}"
    );

    // From impls must be present.
    assert!(
        content.contains("impl From<test_lib::Message> for Message"),
        "Must emit core→binding From impl; content:\n{content}"
    );
    assert!(
        content.contains("impl From<Message> for test_lib::Message"),
        "Must emit binding→core From impl; content:\n{content}"
    );
}

/// Regression test: tagged data enums (struct variants) must be lowered to flat PHP classes,
/// not string constants.  A `HashMap<String, DataEnum>` field on a struct must compile:
/// there must be a `From<core::DataEnum> for DataEnum` impl (not `From<DataEnum> for String`).
#[test]
fn test_tagged_data_enum_generates_flat_class_not_string_constants() {
    let backend = PhpBackend;

    let data_enum = EnumDef {
        name: "SecuritySchemeInfo".to_string(),
        rust_path: "test_lib::SecuritySchemeInfo".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Http".to_string(),
                fields: vec![
                    make_field("scheme", TypeRef::String, false),
                    make_field("bearer_format", TypeRef::String, true),
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("http".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
            },
            EnumVariant {
                name: "ApiKey".to_string(),
                fields: vec![
                    make_field("location", TypeRef::String, false),
                    make_field("name", TypeRef::String, false),
                ],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("apiKey".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
            },
        ],
        doc: "Security scheme types".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: Some("lowercase".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
    };

    let config_type = TypeDef {
        name: "OpenApiConfig".to_string(),
        rust_path: "test_lib::OpenApiConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field(
            "security_schemes",
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("SecuritySchemeInfo".to_string())),
            ),
            false,
        )],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![],
        enums: vec![data_enum],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // Must NOT emit string constants for the data enum
    assert!(
        !content.contains("pub const SECURITYSCHEMEINFO_HTTP"),
        "Data enum must not generate string constants"
    );

    // Must emit a flat PHP class struct
    assert!(
        content.contains("pub struct SecuritySchemeInfo"),
        "Data enum must generate a flat PHP class struct"
    );

    // The struct must have a discriminator field named after the serde tag
    assert!(
        content.contains("type_tag"),
        "Flat struct must have a type_tag discriminator field"
    );

    // The struct must have variant fields
    assert!(content.contains("scheme"), "Flat struct must have scheme field");
    assert!(content.contains("location"), "Flat struct must have location field");

    // Must emit From<core::SecuritySchemeInfo> for SecuritySchemeInfo
    assert!(
        content.contains("impl From<test_lib::SecuritySchemeInfo> for SecuritySchemeInfo"),
        "Must emit core→binding From impl"
    );

    // Must emit From<SecuritySchemeInfo> for core::SecuritySchemeInfo
    assert!(
        content.contains("impl From<SecuritySchemeInfo> for test_lib::SecuritySchemeInfo"),
        "Must emit binding→core From impl"
    );

    // The HashMap field on OpenApiConfig must use the PHP class, not String
    assert!(
        content.contains("HashMap<String, SecuritySchemeInfo>"),
        "HashMap field must use the flat PHP class type, not String"
    );
}

#[test]
fn test_stubs_non_void_methods_have_return_statements() {
    // PHPStan at level 9 rejects non-void methods with empty `{ }` bodies.
    // All stub methods with non-void return types must emit a body that
    // satisfies the static analyser — `throw new \RuntimeException(...)`.
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("label", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![FunctionDef {
            name: "create_config".to_string(),
            rust_path: "test_lib::create_config".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::Named("Config".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // getErrorCode(): int must NOT be bare `{ }` — PHPStan rejects it.
    assert!(
        !content.contains("getErrorCode(): int { }"),
        "getErrorCode stub must not have an empty body `{{ }}`; content:\n{content}"
    );

    // Non-void getter stubs must contain a throw or return, not bare `{ }`.
    // The pattern `): int { }` or `): string { }` or `): ?string { }` are all wrong.
    assert!(
        !content.contains("): int { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );
    assert!(
        !content.contains("): string { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );
    assert!(
        !content.contains("): ?string { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );

    // The static method stub in the Api class must also not be empty.
    assert!(
        !content.contains("): \\Test\\Lib\\Config { }"),
        "Api class stub method must not have an empty body; content:\n{content}"
    );

    // Stubs should use throw to satisfy PHPStan.
    assert!(
        content.contains("throw new \\RuntimeException"),
        "stub bodies must throw \\RuntimeException to satisfy PHPStan level 9; content:\n{content}"
    );
}

#[test]
fn test_static_stubs_promote_parameters_after_first_optional() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "submit_form".to_string(),
            rust_path: "test_lib::submit_form".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::String,
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
                },
                ParamDef {
                    name: "json".to_string(),
                    ty: TypeRef::String,
                    optional: true,
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
                },
                ParamDef {
                    name: "multipart".to_string(),
                    ty: TypeRef::String,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let content = &files.first().unwrap().content;

    assert!(
        content.contains("submitForm(string $path, ?string $json = null, ?string $multipart = null): string"),
        "static stub should keep PHP syntax valid when a required Rust param follows an optional one; content:\n{content}"
    );
}

#[test]
fn test_vec_named_struct_parameter() {
    let backend = PhpBackend;

    // Create test API with Vec<Item> parameter
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Item".to_string(),
            rust_path: "test_lib::Item".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("id", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![FunctionDef {
            name: "batch_process".to_string(),
            rust_path: "test_lib::batch_process".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Batch process items".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);
    assert!(
        result.is_ok(),
        "Generation should succeed for Vec<NamedStruct> parameter"
    );

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    // Should contain the Item type definition
    assert!(
        lib_rs.content.contains("pub struct Item"),
        "Should contain Item struct definition"
    );

    // Should contain the batch_process function with Vec<Item> parameter handling
    assert!(
        lib_rs.content.contains("batch_process"),
        "Should contain batch_process function"
    );

    // The generated code should contain array iteration logic for Vec<Item>
    // (looking for the manual conversion pattern we implemented)
    assert!(
        lib_rs.content.contains("items_core") || lib_rs.content.contains(".iter()"),
        "Should contain array iteration logic for Vec<Item> parameter conversion"
    );

    // Should NOT contain a panic stub or empty body for the function
    assert!(
        !lib_rs
            .content
            .contains(&"fn batch_process() {\n        unimplemented!()".to_string()),
        "Should NOT generate unimplemented stub for batch_process"
    );
}

#[test]
fn test_dto_stubs_use_final_class_with_readonly_promoted_params() {
    // PHP 8.3+ idiom: DTOs must be emitted as `final class` with constructor property
    // promotion (`public readonly`) and no redundant getter methods.
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SystemMessage".to_string(),
            rust_path: "test_lib::SystemMessage".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // (a) DTOs must use `final class`, not bare `class`
    assert!(
        content.contains("final class SystemMessage"),
        "DTO stub must use `final class`; content:\n{content}"
    );

    // (b) Constructor parameters must use `public readonly` promotion
    assert!(
        content.contains("public readonly string $content"),
        "Required field must use `public readonly` promotion; content:\n{content}"
    );
    assert!(
        content.contains("public readonly ?string $name"),
        "Optional field must use `public readonly` promotion with nullable type; content:\n{content}"
    );

    // (c) No redundant getFoo() getter methods alongside public readonly properties
    assert!(
        !content.contains("getContent()"),
        "Redundant getter `getContent()` must not be emitted; content:\n{content}"
    );
    assert!(
        !content.contains("getName()"),
        "Redundant getter `getName()` must not be emitted; content:\n{content}"
    );

    // (d) No separate property declarations (they are promoted into the constructor)
    assert!(
        !content.contains("    public string $content;"),
        "Separate property declaration must not be emitted; content:\n{content}"
    );
}

#[test]
fn test_dto_properties_use_camel_case_php_names() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("device_id", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("include_headers", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field("strip_text", TypeRef::String, true),
                make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U64), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Test config with snake_case fields".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let stubs_result = backend.generate_type_stubs(&api, &config);
    assert!(stubs_result.is_ok(), "Stub generation should succeed");

    let stubs_files = stubs_result.unwrap();
    let stubs = stubs_files.first().expect("Should generate stubs file");
    let content = &stubs.content;

    // Verify camelCase conversion in PHP stubs: snake_case Rust names → camelCase PHP names
    assert!(
        content.contains("$deviceId"),
        "Property device_id should be converted to $deviceId (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$includeHeaders"),
        "Property include_headers should be converted to $includeHeaders (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$stripText"),
        "Property strip_text should be converted to $stripText (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$timeoutMs"),
        "Property timeout_ms should be converted to $timeoutMs (camelCase)\nContent:\n{content}"
    );

    // Verify snake_case names are NOT present in stubs
    assert!(
        !content.contains("$device_id"),
        "Property name should NOT be in snake_case: $device_id\nContent:\n{content}"
    );
    assert!(
        !content.contains("$include_headers"),
        "Property name should NOT be in snake_case: $include_headers\nContent:\n{content}"
    );
}

#[test]
fn test_unit_enums_emit_native_php_81_backed_enums() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "test_lib::OutputFormat".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Markdown".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Html".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                },
            ],
            doc: "Output format options".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let stubs_result = backend.generate_type_stubs(&api, &config);
    assert!(stubs_result.is_ok(), "Stub generation should succeed");

    let stubs_files = stubs_result.unwrap();
    let stubs = stubs_files.first().expect("Should generate stubs file");
    let content = &stubs.content;

    // Unit-variant enums should emit as native PHP 8.1+ backed enums
    assert!(
        content.contains("enum OutputFormat: string"),
        "Unit-variant enum should be emitted as PHP 8.1+ native enum with string backing\nContent:\n{content}"
    );
    assert!(
        content.contains("case Text = "),
        "Enum case Text should be present with value\nContent:\n{content}"
    );
    assert!(
        content.contains("case Markdown = "),
        "Enum case Markdown should be present with value\nContent:\n{content}"
    );
    assert!(
        content.contains("case Html = "),
        "Enum case Html should be present with value\nContent:\n{content}"
    );

    // Should NOT emit as a class with constants
    assert!(
        !content.contains("final class OutputFormat") && !content.contains("public const Text"),
        "Unit-variant enum should NOT be emitted as a class with constants\nContent:\n{content}"
    );
}

fn make_field_with_doc(name: &str, ty: TypeRef, optional: bool, doc: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: doc.to_string(),
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

#[test]
fn test_type_stubs_documented_field_emits_var_phpdoc_with_description() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ClientConfig".to_string(),
            rust_path: "test_lib::ClientConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field_with_doc(
                    "base_url",
                    TypeRef::Optional(Box::new(TypeRef::String)),
                    true,
                    "Base URL of the remote API endpoint. Defaults to OpenAI's.",
                ),
                make_field_with_doc(
                    "timeout_secs",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                    true,
                    "Request timeout in seconds.\nDefaults to 30.",
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // Single-line doc: compact /** @var T Description. */ form.
    assert!(
        content.contains("@var ?string Base URL of the remote API endpoint. Defaults to OpenAI's."),
        "Documented optional string field should have @var ?string with description;\ncontent:\n{content}"
    );

    // Multi-line doc: multi-line block with description + @var tag.
    assert!(
        content.contains("@var ?int"),
        "Documented optional int field should have @var ?int tag;\ncontent:\n{content}"
    );
    assert!(
        content.contains("Request timeout in seconds."),
        "Multi-line doc first line should appear in PHPDoc;\ncontent:\n{content}"
    );
    assert!(
        content.contains("Defaults to 30."),
        "Multi-line doc second line should appear in PHPDoc;\ncontent:\n{content}"
    );
}

#[test]
fn test_type_stubs_undocumented_field_emits_var_phpdoc_type_only() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field(
                    "max_retries",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                    true,
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // Type-only compact form for undocumented fields.
    assert!(
        content.contains("/** @var bool */"),
        "Undocumented bool field should have type-only /** @var bool */;\ncontent:\n{content}"
    );
    assert!(
        content.contains("/** @var ?int */"),
        "Undocumented optional int field should have type-only /** @var ?int */;\ncontent:\n{content}"
    );
}

#[test]
fn test_public_api_sanitizes_rust_syntax_from_docstrings() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "test_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "html".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::String,
            error_type: None,
            is_async: false,
            doc: "Convert markup conversion, returning a result.\n\n# Arguments\n\n* `html` - The HTML string to convert.\n\n# Example\n\n```rust\nuse test_lib::convert;\nlet result = convert(html, None).unwrap();\n```"
                .to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
};

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let facade = files.first().unwrap();
    let content = &facade.content;

    // Verify Rust syntax is NOT in the docstring
    assert!(
        !content.contains("use test_lib::convert;"),
        "Rust 'use' statement must not leak into PHPDoc"
    );
    assert!(!content.contains(".unwrap()"), ".unwrap() must not leak into PHPDoc");
    assert!(!content.contains("```rust"), "Raw Rust fence must not appear in PHPDoc");

    // Verify summary IS present
    assert!(
        content.contains("Convert markup conversion"),
        "Summary must be preserved in PHPDoc"
    );

    // Verify @param/@return tags are present (emitted separately)
    assert!(
        content.contains("@param"),
        "@param tag must be present for documented parameters"
    );
    assert!(content.contains("@return"), "@return tag must be present");
}

/// Regression test: a Duration field on a Default struct is stored as `Option<i64>` in the
/// binding (via the `option_duration_on_defaults` path in the struct emitter). The getter
/// return type must mirror the storage type and also be `Option<i64>`, not bare `i64`.
///
/// Before the fix the getter was emitted as `pub fn get_ttl(&self) -> i64 { self.ttl.clone() }`,
/// which caused E0308 because `self.ttl` is `Option<i64>`.
#[test]
fn test_duration_field_on_default_struct_getter_returns_option() {
    let backend = PhpBackend;

    // Simulate a struct like `CacheConfig { ttl: Duration }` with `has_default = true`.
    // The IR uses TypeRef::Duration for the field and has `optional = false`.
    // The struct emitter wraps it in Option<i64> when option_duration_on_defaults is enabled;
    // the getter must match.
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "CacheConfig".to_string(),
            rust_path: "test_lib::CacheConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("max_entries", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("ttl", TypeRef::Duration, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Cache configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation must succeed: {:?}", result.err());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // The struct field must be Option<i64> (Duration → i64 ms, wrapped in Option).
    assert!(
        content.contains("pub ttl: Option<i64>"),
        "Duration field on Default struct must be stored as Option<i64>; got:\n{content}"
    );

    // The getter must return Option<i64>, not bare i64, to match the storage type.
    assert!(
        content.contains("fn get_ttl") && content.contains("-> Option<i64>"),
        "getter for Duration field on Default struct must return Option<i64>; got:\n{content}"
    );

    // Must NOT emit the wrong bare return type.
    assert!(
        !content.contains("fn get_ttl(&self) -> i64"),
        "getter must not return bare i64 for a Duration-on-Default field; got:\n{content}"
    );
}

/// Regression test for the `has_default` + `#[serde(default)]` defaults bug.
///
/// When a core struct has a custom `impl Default` (e.g. `max_redirects: 10`), the binding's
/// struct-level `#[serde(default)]` previously caused missing JSON fields to fall back to
/// the derived `Default`, which uses Rust's primitive zeros. The zeros were then propagated
/// to the core type via `From<BindingType>`, silently clobbering the core's semantic defaults.
///
/// The fix suppresses the auto `#[derive(Default)]` and emits a delegating
/// `impl Default for BindingType { fn default() -> Self { <core::Type as Default>::default().into() } }`
/// so that `serde(default)` and `unwrap_or_default()` honour the core's custom values.
///
/// This test exercises the shared struct generator directly with the same configuration
/// shape PHP uses when serde is available, so it does not depend on filesystem-based
/// Cargo.toml serde detection.
#[test]
fn has_default_struct_emits_delegating_impl_not_derived_default() {
    use alef::codegen::generators::{AsyncPattern, RustBindingConfig, gen_struct_with_per_field_attrs};
    use alef::core::ir::FieldDef;

    let typ = TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "test_lib::CrawlConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            // Mirrors the real-world bug: core sets `max_redirects: 10` in its custom
            // Default, but the binding's derived Default uses `0` for i64, which then
            // overrides the core's value on round-trip through `From<BindingType>`.
            make_field("max_redirects", TypeRef::Primitive(PrimitiveType::I64), false),
            make_field("respect_robots_txt", TypeRef::Primitive(PrimitiveType::Bool), false),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
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
    };

    struct StubMapper;
    impl alef::codegen::type_mapper::TypeMapper for StubMapper {
        fn error_wrapper(&self) -> &str {
            "Result"
        }
    }
    let mapper = StubMapper;

    // PHP-shaped config with `emit_delegating_default_impl: true` and serde derives.
    // Mirrors `gen_php_struct`'s `cfg.has_serde == true` branch but without depending on
    // crate-private symbols.
    let struct_attrs: &[&str] = &["php_class", "serde(default, rename_all = \"camelCase\")"];
    let struct_derives: &[&str] = &["Clone", "serde::Serialize", "serde::Deserialize"];
    let cfg = RustBindingConfig {
        struct_attrs,
        field_attrs: &[],
        struct_derives,
        method_block_attr: Some("php_impl"),
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[php_function]",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "test_lib",
        async_pattern: AsyncPattern::TokioBlockOn,
        has_serde: true,
        type_name_prefix: "",
        option_duration_on_defaults: true,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: true,
        skip_methods_when_not_delegatable: false,
    };

    let content = gen_struct_with_per_field_attrs(&typ, &mapper, &cfg, |_: &FieldDef| vec![]);

    // The struct must NOT derive Default — that would override the core's custom defaults.
    let struct_start = content
        .find("pub struct CrawlConfig")
        .expect("CrawlConfig struct must be emitted");
    let derive_window = &content[..struct_start];
    assert!(
        !derive_window.contains("Default"),
        "CrawlConfig must NOT derive Default — that would emit zeros instead of \
         delegating to the core's custom Default. Derive block:\n{derive_window}"
    );

    // The delegating `impl Default` must be emitted and delegate to the core type's Default.
    assert!(
        content.contains("impl Default for CrawlConfig"),
        "delegating impl Default must be emitted for has_default types; got:\n{content}"
    );
    assert!(
        content.contains("<test_lib::CrawlConfig as Default>::default().into()"),
        "impl Default must delegate to the core type's Default via `.into()`; got:\n{content}"
    );

    // The struct should still carry struct-level `#[serde(default)]` for from_json to accept
    // partial JSON — this is the path that previously surfaced the bug.
    assert!(
        content.contains("serde(default"),
        "struct must still carry struct-level `#[serde(default)]`; got:\n{content}"
    );

    // Serde Serialize/Deserialize derives must remain — only Default is suppressed.
    assert!(
        content.contains("serde::Serialize"),
        "struct must still derive serde::Serialize; got:\n{content}"
    );
    assert!(
        content.contains("serde::Deserialize"),
        "struct must still derive serde::Deserialize; got:\n{content}"
    );
}

/// Companion test: when `emit_delegating_default_impl` is false (default for non-PHP backends),
/// the auto `Default` derive is preserved and no delegating impl is emitted.
#[test]
fn has_default_struct_keeps_derived_default_when_delegation_disabled() {
    use alef::codegen::generators::{AsyncPattern, RustBindingConfig, gen_struct_with_per_field_attrs};
    use alef::core::ir::FieldDef;

    let typ = TypeDef {
        name: "PlainConfig".to_string(),
        rust_path: "test_lib::PlainConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("count", TypeRef::Primitive(PrimitiveType::I64), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
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
    };

    struct StubMapper;
    impl alef::codegen::type_mapper::TypeMapper for StubMapper {
        fn error_wrapper(&self) -> &str {
            "Result"
        }
    }
    let mapper = StubMapper;

    let cfg = RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &["Clone"],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "test_lib",
        async_pattern: AsyncPattern::None,
        has_serde: false,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
    };

    let content = gen_struct_with_per_field_attrs(&typ, &mapper, &cfg, |_: &FieldDef| vec![]);

    assert!(
        content.contains("Default"),
        "Default must still be derived when emit_delegating_default_impl is false; got:\n{content}"
    );
    assert!(
        !content.contains("impl Default for PlainConfig"),
        "no delegating impl Default should be emitted when the flag is disabled; got:\n{content}"
    );
}

/// Regression test: when a Rust function has `Option<T>` parameters (e.g., `mime_type: Option<&str>`),
/// the PHP wrapper must emit nullable type hints (`?string`) with defaults (`= null`), not non-nullable.
/// Previously, when `TypeRef::Optional` was already prepended by `php_type()`, the code would incorrectly
/// add another `?` prefix, creating invalid double-nullable types or failing to detect existing nullability.
#[test]
fn test_php_option_param_emits_nullable_with_default() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "do_thing".to_string(),
            rust_path: "test_lib::do_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "required_str".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "optional_str".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Do a thing with strings".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate ok");

    let facade_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".php"))
        .expect("facade file exists");

    let content = &facade_file.content;

    // Required parameter should be `string $required_str` (not nullable, no default).
    assert!(
        content.contains("string $required_str"),
        "required parameter must be non-nullable; got:\n{content}"
    );

    // Optional parameter should be `?string $optional_str = null` (nullable with default).
    // Must NOT be `??string` (double-nullable) or `string $optional_str` (missing null default).
    assert!(
        content.contains("?string $optional_str = null"),
        "optional parameter must be ?string with = null default; got:\n{content}"
    );

    // Verify no double-nullable nonsense.
    assert!(
        !content.contains("??string"),
        "must not have double-nullable ??string; got:\n{content}"
    );
}

/// Regression test for Block B7: required &str parameters must not be marked nullable.
/// When a function has both required and optional string parameters, the required ones
/// should remain non-nullable (string $param) even if they're followed by optional ones.
/// This test ensures that nullable inference doesn't propagate from optional params
/// to required ones, which would cause null to pass through the PHP wrapper and panic
/// in the Rust core where the parameter is actually required.
#[test]
fn test_php_required_str_param_not_nullable_with_optional_tail() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_document".to_string(),
            rust_path: "test_lib::process_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                // Required &str parameter (maps to TypeRef::String with optional=false)
                ParamDef {
                    name: "content_type".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    is_ref: true, // Rust signature: &str
                    ..ParamDef::default()
                },
                // Optional &str parameter (maps to TypeRef::Optional(String) with is_ref=true)
                ParamDef {
                    name: "hint".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Process a document with optional hint".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate ok");

    let facade_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".php"))
        .expect("facade file exists");

    let content = &facade_file.content;

    // Required parameter MUST be non-nullable, not "?string $content_type".
    // The Rust core function signature is processDocument(content_type: &str, ...)
    // so null is never valid for this parameter.
    assert!(
        content.contains("string $content_type") && !content.contains("?string $content_type"),
        "required &str parameter must be non-nullable string; got:\n{content}"
    );

    // Optional parameter MUST be nullable with default.
    // The Rust core function accepts Option<&str>, so PHP can pass null.
    assert!(
        content.contains("?string $hint = null"),
        "optional parameter must be ?string with = null default; got:\n{content}"
    );

    // Sanity: no double-nullable.
    assert!(
        !content.contains("??string"),
        "must not have double-nullable ??string; got:\n{content}"
    );
}

/// Every generated PHP source file must have a blank line immediately after the
/// `<?php` opening tag. PSR-12's `blank_line_after_opening_tag` rule (enforced by
/// php-cs-fixer) inserts one post-write, which would mutate the alef-hash-tracked
/// file and break `alef verify`. Emitting it natively keeps the formatter a no-op.
#[test]
fn test_php_source_files_have_blank_line_after_opening_tag() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Config".to_string(),
                rust_path: "test_lib::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Config".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            },
            TypeDef {
                name: "Handle".to_string(),
                rust_path: "test_lib::Handle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "close".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Close the handle".to_string(),
                    receiver: Some(ReceiverKind::Owned),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Opaque handle".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    // Collect all generated PHP source files (facade + opaque class files + type stubs).
    let mut php_files: Vec<alef::core::backend::GeneratedFile> = Vec::new();
    php_files.extend(backend.generate_public_api(&api, &config).expect("public api ok"));
    php_files.extend(backend.generate_type_stubs(&api, &config).expect("type stubs ok"));
    php_files.retain(|f| f.path.extension().and_then(|e| e.to_str()) == Some("php"));
    assert!(!php_files.is_empty(), "expected at least one generated .php file");

    for file in &php_files {
        let name = file.path.to_string_lossy().to_string();
        assert!(
            file.content.starts_with("<?php\n\n"),
            "{name} must have a blank line after `<?php` (PSR-12 blank_line_after_opening_tag). got:\n{}",
            &file.content[..file.content.len().min(120)],
        );
    }

    // Strongest check: run php-cs-fixer with the scaffold's @PSR12 ruleset and assert it
    // produces zero changes. Skips when php or php-cs-fixer are unavailable.
    use std::process::Command;
    let tools_available = Command::new("php").arg("--version").output().is_ok()
        && Command::new("php-cs-fixer").arg("--version").output().is_ok();
    if !tools_available {
        eprintln!("skipping php-cs-fixer no-op check: php or php-cs-fixer not installed");
        return;
    }

    let dir = tempfile::tempdir().unwrap();

    // The scaffold's php-cs-fixer config formats `src/` but explicitly excludes `stubs/`
    // (`->notPath('stubs')`) because the stub files carry ext-php-rs scaffolding the formatter
    // would otherwise rewrite. So the formatter no-op contract applies to the userland `src/`
    // files (facade + opaque DTO classes) — those are what `alef verify` and the formatter must
    // agree on. Stub files only need the blank-line-after-`<?php` guarantee asserted above.
    for file in php_files.iter().filter(|f| !f.path.to_string_lossy().contains("stubs")) {
        let php_path = dir.path().join("subject.php");
        std::fs::write(&php_path, &file.content).unwrap();
        let output = Command::new("php-cs-fixer")
            .arg("fix")
            .arg("--using-cache=no")
            .arg("--rules=@PSR12")
            .arg(&php_path)
            .output()
            .expect("run php-cs-fixer");
        let after = std::fs::read_to_string(&php_path).unwrap();
        assert_eq!(
            after,
            file.content,
            "php-cs-fixer rewrote {}; stderr:\n{}",
            file.path.display(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn facade_emits_nullable_marker_for_non_tail_optional_param() {
    // Regression: when an `Option<T>` param is followed by a non-nullable required
    // param, PHP 8.1 ordering forces the optional param into a non-tail position.
    // The facade must still emit `?T $name` (nullable, no default) so callers can
    // pass `null`. Before the fix, the emitter dropped the `?` entirely, producing
    // `string $mime_type` for the canonical `extract_file(path, mime_type, config)`
    // signature, which made every test passing `null` for `mime_type` fail with a
    // PHP TypeError.
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        }],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "mime_type".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let facade = files.first().expect("facade file generated");
    assert!(
        facade.content.contains("?string $mime_type"),
        "facade must keep the nullable marker on non-tail Option<T> params; got:\n{}",
        facade.content
    );
    // Reject a non-nullable `string $mime_type` (must be `?string`). Use leading-space
    // anchors so the `?` form isn't a substring match for the non-`?` form.
    assert!(
        !facade.content.contains(" string $mime_type"),
        "facade must not emit a non-nullable `string $mime_type`; got:\n{}",
        facade.content
    );
}

#[test]
fn module_entry_uses_explicit_extension_name_not_cargo_pkg_name() {
    // Regression test for PHP module registration bug where the module name
    // did not match the extension name, causing `php -m` to fail and PIE
    // install to error with "already loaded". The root cause was #[php_module]
    // macro expansion using env!("CARGO_PKG_NAME") which could differ from
    // the publishable extension_name (e.g., crate "ts-pack-core-php" vs.
    // extension "tree_sitter_language_pack").
    // Solution: generate ModuleBuilder::new(extension_name, version) explicitly.
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.2.3".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config_with_extension("tree_sitter_language_pack");
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files.iter().find(|f| f.path.ends_with("lib.rs")).expect("lib.rs generated");

    // Verify the module entry function explicitly passes extension_name to ModuleBuilder::new()
    assert!(
        lib_rs.content.contains("ModuleBuilder::new(") && lib_rs.content.contains("tree_sitter_language_pack"),
        "module entry must use explicit extension name in ModuleBuilder::new(); got:\n{}",
        lib_rs.content
    );

    // Verify it does NOT use env!("CARGO_PKG_NAME") fallback
    assert!(
        !lib_rs.content.contains("CARGO_PKG_NAME"),
        "module entry must not rely on CARGO_PKG_NAME macro; got:\n{}",
        lib_rs.content
    );

    // Verify the get_module function is properly formed with manual ModuleBuilder
    assert!(
        lib_rs.content.contains("extern \"C\" fn get_module()"),
        "module entry must export get_module extern function; got:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs.content.contains("StaticModuleEntry"),
        "module entry must use StaticModuleEntry for thread-safe singleton; got:\n{}",
        lib_rs.content
    );
}
