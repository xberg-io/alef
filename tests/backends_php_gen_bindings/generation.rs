use super::*;

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
            version: Default::default(),
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
            version: Default::default(),
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
    let mut policy = make_field("policy", TypeRef::Named("Policy".to_string()), false);
    policy.default = Some("#[serde(default = \"Policy::from_env\")]".to_string());
    policy.type_rust_path = Some("test_lib::Policy".to_string());

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Limits".to_string(),
            rust_path: "test_lib::Limits".to_string(),
            original_rust_path: String::new(),
            fields: vec![max_items, enabled, policy],
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
            version: Default::default(),
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
        lib.content
            .contains("pub fn limits_policy() -> test_lib::Policy { test_lib::Policy::from_env() }"),
        "function-path serde default must emit a matching helper:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("serde(default = \"crate::serde_defaults::limits_policy\")"),
        "function-path serde default must attach a field serde attribute:\n{}",
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
            version: Default::default(),
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
            version: Default::default(),
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
                    cfg: None,
                    version: Default::default(),
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
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
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
            version: Default::default(),
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
            version: Default::default(),
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
            version: Default::default(),
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
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
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
            version: Default::default(),
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
            version: Default::default(),
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
                    cfg: None,
                    version: Default::default(),
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
                    cfg: None,
                    version: Default::default(),
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
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
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
            version: Default::default(),
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
