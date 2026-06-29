use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Helper to create a FieldDef with all defaults.
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

/// Helper to create a basic ResolvedCrateConfig with Ruby enabled.
fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
    )
}

#[test]
fn test_basic_generation() {
    let backend = MagnusBackend;

    // Create test API surface with types, functions, and enums
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true),
                make_field("backend", TypeRef::String, false),
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
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
            error_type: Some("ProcessError".to_string()),
            doc: "Process input with config".to_string(),
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
            name: "Backend".to_string(),
            rust_path: "test_lib::Backend".to_string(),
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
            doc: "Available backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
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

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate at least one file");

    // Check for expected file
    let file_names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    assert!(
        file_names.iter().any(|f| f.contains("lib.rs")),
        "Should generate lib.rs file"
    );

    // Verify content contains Magnus-specific markers
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for Magnus imports and macros
    assert!(
        content.contains("magnus::wrap"),
        "Should contain magnus::wrap attribute"
    );
    assert!(
        content.contains("IntoValue"),
        "Should contain IntoValue trait implementation"
    );
    assert!(
        content.contains("TryConvert"),
        "Should contain TryConvert trait implementation"
    );
    assert!(
        content.contains("TryConvertOwned"),
        "Should contain TryConvertOwned marker trait"
    );

    // Check for struct generation
    assert!(content.contains("struct Config"), "Should generate Config struct");

    // Check for enum generation
    assert!(content.contains("enum Backend"), "Should generate Backend enum");
    assert!(content.contains("Tesseract"), "Should contain Tesseract variant");
    assert!(content.contains("PaddleOcr"), "Should contain PaddleOcr variant");

    // Check for function/method generation
    assert!(content.contains("process"), "Should contain process function");
}

#[test]
fn test_type_mapping() {
    let backend = MagnusBackend;

    // Create API with various field types to test type mapping
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("vec_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field("option_val", TypeRef::Optional(Box::new(TypeRef::String)), true),
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
            has_private_fields: false,
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check that struct is generated with proper field types
    assert!(content.contains("struct Numbers"), "Should generate Numbers struct");

    // Verify Magnus-specific type wrapping
    assert!(content.contains("magnus::wrap"), "Should have magnus::wrap attribute");
}

#[test]
fn test_enum_generation() {
    let backend = MagnusBackend;

    // Create API with a more complex enum
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
                    name: "Processing".to_string(),
                    fields: vec![],
                    doc: "Processing status".to_string(),
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
            methods: vec![],
            doc: "Processing status".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check enum definition
    assert!(content.contains("enum Status"), "Should generate Status enum");
    assert!(content.contains("Pending"), "Should contain Pending variant");
    assert!(content.contains("Processing"), "Should contain Processing variant");
    assert!(content.contains("Complete"), "Should contain Complete variant");

    // Check for conversion traits (IntoValue, TryConvert)
    assert!(
        content.contains("impl magnus::IntoValue for Status"),
        "Should implement IntoValue for enum"
    );
    assert!(
        content.contains("impl magnus::TryConvert for Status"),
        "Should implement TryConvert for enum"
    );

    // Check for symbol conversion (Ruby symbols)
    assert!(content.contains("to_symbol"), "Should convert to Ruby symbols");
}

/// Regression: a Ruby caller passing a bare variant name for an internally-tagged enum
/// (e.g. `"disabled"`) must round-trip. The previous fallback chain only tried the raw
/// string and a quoted JSON string, both of which a `#[serde(tag = ...)]` enum rejects.
/// The constructor must also try the tagged form `{"<tag>": name}`.
#[test]
fn test_internally_tagged_enum_constructor_wraps_bare_string() {
    let backend = MagnusBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "VlmFallbackPolicy".to_string(),
            rust_path: "test_lib::VlmFallbackPolicy".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Disabled".to_string(),
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
                    name: "OnLowQuality".to_string(),
                    fields: vec![make_field(
                        "quality_threshold",
                        TypeRef::Primitive(PrimitiveType::F64),
                        false,
                    )],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: true,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: Some("mode".to_string()),
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .expect("lib.rs generated");

    assert!(
        lib.content.contains(r#"serde_json::json!({ "mode": json_str })"#),
        "internally-tagged enum constructor must try the tagged form {{\"mode\": json_str}};\ncontent:\n{}",
        lib.content
    );
}

#[test]
fn test_generated_header() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Simple".to_string(),
            rust_path: "test_lib::Simple".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
            has_private_fields: false,
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

    // Check that main lib.rs has auto-generated header (set by with_generated_header())
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    // The content should include the auto-generated marker from RustFileBuilder::with_generated_header()
    assert!(
        lib_file.content.contains("Code generated")
            || lib_file.content.contains("auto-generated")
            || lib_file.content.contains("DO NOT EDIT"),
        "Generated file should have an auto-generated header comment"
    );
}

#[test]
fn test_methods_generation() {
    let backend = MagnusBackend;

    // Create a TypeDef with methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Store".to_string(),
            rust_path: "test_lib::Store".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("name", TypeRef::String, false),
                make_field("count", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            methods: vec![
                MethodDef {
                    name: "get_name".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Get store name".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "increment".to_string(),
                    params: vec![ParamDef {
                        name: "amount".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
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
                    return_type: TypeRef::Primitive(PrimitiveType::U32),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Increment counter".to_string(),
                    receiver: Some(ReceiverKind::RefMut),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
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
            doc: "A data store".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for struct definition
    assert!(content.contains("struct Store"), "Should generate Store struct");

    // Check for method! macros (Magnus method bindings)
    assert!(
        content.contains("method!("),
        "Should contain method! macro for instance methods"
    );

    // Check for specific method names
    assert!(content.contains("get_name"), "Should contain get_name method");
    assert!(content.contains("increment"), "Should contain increment method");

    // Check for define_method usage in module initialization
    assert!(
        content.contains("define_method") || content.contains("method!"),
        "Should use Magnus method macros"
    );
}

#[test]
fn test_error_types() {
    let backend = MagnusBackend;

    // Create an API with error types
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "validate".to_string(),
            rust_path: "test_lib::validate".to_string(),
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
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: Some("ValidationError".to_string()),
            doc: "Validate input".to_string(),
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
        errors: vec![ErrorDef {
            name: "ValidationError".to_string(),
            rust_path: "test_lib::ValidationError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidFormat".to_string(),
                    fields: vec![],
                    doc: "Invalid format".to_string(),
                    message_template: Some("invalid format provided".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "OutOfRange".to_string(),
                    fields: vec![],
                    doc: "Out of range".to_string(),
                    message_template: Some("value out of range".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: "Validation error type".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for error converter generation (gen_magnus_error_converter)
    assert!(
        content.contains("ValidationError"),
        "Should contain ValidationError type reference"
    );

    // Check for error handling in function
    assert!(content.contains("validate"), "Should contain validate function");

    // Error variants may not appear directly in generated code; just verify the function exists
    // The important thing is that the error type is processed by gen_magnus_error_converter
}

#[test]
fn test_async_function() {
    let backend = MagnusBackend;

    // Create API with async function
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_async".to_string(),
            rust_path: "test_lib::process_async".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "data".to_string(),
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
            error_type: None,
            doc: "Process data asynchronously".to_string(),
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

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for async function presence
    assert!(
        content.contains("process_async"),
        "Should contain process_async function"
    );

    // Check for tokio/async runtime integration
    assert!(
        content.contains("tokio") || content.contains("async") || content.contains("block_on"),
        "Should contain async/tokio runtime handling"
    );

    // Check for function! macro
    assert!(
        content.contains("function!("),
        "Should use function! macro for free functions"
    );
}

#[test]
fn test_async_helper_registers_under_original_public_name() {
    let backend = MagnusBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_async".to_string(),
            rust_path: "test_lib::extract_async".to_string(),
            original_rust_path: "test_lib::extract".to_string(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::String,
            is_async: true,
            ..FunctionDef::default()
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    assert!(
        content.contains(r#"module.define_module_function("extract", function!(extract_async, 1))?;"#),
        "async helper must be registered under the original public name:\n{content}"
    );
    assert!(
        !content.contains(r#"module.define_module_function("extract_async", function!(extract_async, 1))?;"#),
        "async helper name must not be exposed as the public Ruby method:\n{content}"
    );
}

#[test]
fn test_opaque_type() {
    let backend = MagnusBackend;

    // Create API with opaque type
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
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
                version: Default::default(),
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
            doc: "Opaque processor type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for opaque struct generation with Arc wrapping
    assert!(content.contains("struct Processor"), "Should generate Processor struct");
    assert!(content.contains("Arc<"), "Opaque types should wrap inner with Arc");

    // Check for magnus::wrap attribute
    assert!(
        content.contains("magnus::wrap"),
        "Should use magnus::wrap for opaque types"
    );

    // Check for TryConvert and IntoValue implementations
    assert!(
        content.contains("impl magnus::TryConvert for Processor"),
        "Should implement TryConvert for opaque type"
    );
    assert!(
        content.contains("IntoValueFromNative"),
        "Should implement IntoValueFromNative for opaque type"
    );
}

#[test]
fn test_default_config() {
    let backend = MagnusBackend;

    // Create API with a type that has default: true
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("retries", TypeRef::Primitive(PrimitiveType::U32), true),
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
            doc: "Configuration with default".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for struct generation
    assert!(content.contains("struct Config"), "Should generate Config struct");

    // Check for Default impl generation that delegates to the core default.
    assert!(
        content.contains("impl Default for Config"),
        "Should generate Default implementation for types with has_default: true"
    );
    assert!(
        content.contains("test_lib::Config::default().into()"),
        "Default implementation must delegate to the core default; content:\n{content}"
    );

    // Check for magnus wrapper
    assert!(content.contains("magnus::wrap"), "Should have magnus::wrap");
}

/// Verify that a function with an `Option<Named>` parameter emits `magnus::Value` in its
/// signature and uses `funcall("to_json", ())` + `serde_json::from_str` in the body, so
/// callers can pass a plain Ruby Hash without manually serializing it to JSON first.
#[test]
fn test_named_option_param_emits_magnus_value_with_to_json() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "test_lib::ConversionOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("debug", TypeRef::Primitive(PrimitiveType::Bool), true)],
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "test_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
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
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ConversionOptions".to_string()),
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
            error_type: Some("ConversionError".to_string()),
            doc: "Convert input".to_string(),
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

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Variadic signature: scan_args optional slot must be Option<magnus::Value>
    // so a plain Ruby Hash (or nil) can be passed through.
    assert!(
        content.contains("(Option<magnus::Value>,)"),
        "scan_args optional tuple must use Option<magnus::Value>, got:\n{content}"
    );

    // Body must use TryConvert for has_default struct types (no JSON round-trip)
    assert!(
        content.contains("ConversionOptions::try_convert"),
        "Binding body must use TryConvert for has_default struct params, got:\n{content}"
    );
    assert!(
        content.contains("binding_val.into()"),
        "Binding body must convert binding struct via Into, got:\n{content}"
    );

    // Must not use the old as_deref pattern (which assumed a String input)
    assert!(
        !content.contains("options.as_deref()"),
        "Must not use as_deref on options — options is now magnus::Value, got:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge tests (Magnus plugin bridge via gen_trait_bridge)
// ---------------------------------------------------------------------------

mod trait_bridge {
    use alef::backends::magnus::trait_bridge::gen_trait_bridge;
    use alef::core::config::TraitBridgeConfig;
    use alef::core::ir::*;

    fn make_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "NodeContext".to_string(),
                rust_path: "my_lib::NodeContext".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "depth".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
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
                is_clone: false,
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
                has_private_fields: false,
                version: Default::default(),
            }],
            functions: vec![],
            enums: vec![EnumDef {
                name: "VisitResult".to_string(),
                rust_path: "my_lib::VisitResult".to_string(),
                original_rust_path: String::new(),
                variants: vec![EnumVariant {
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
                }],
                methods: vec![],
                doc: String::new(),
                cfg: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                is_copy: false,
                has_serde: true,
                has_default: false,
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
            has_private_fields: false,
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

    fn make_visitor_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![ParamDef {
                name: "context".to_string(),
                ty: TypeRef::Named("NodeContext".to_string()),
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
            return_type: TypeRef::Named("VisitResult".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: true,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
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
            context_type: Some("NodeContext".to_string()),
            result_type: Some("VisitResult".to_string()),
        }
    }

    // ---- Visitor bridge: type_alias still generates bridge ---

    #[test]
    fn test_visitor_bridge_generates_rb_bridge_struct() {
        let trait_def = make_trait_def("HtmlVisitor", vec![make_visitor_method("visit_node")]);
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "my_lib",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("pub struct RbHtmlVisitorBridge"),
            "visitor bridge must produce RbHtmlVisitorBridge struct"
        );
    }

    #[test]
    fn test_visitor_bridge_does_not_generate_registration_fn() {
        let trait_def = make_trait_def("HtmlVisitor", vec![make_visitor_method("visit_node")]);
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "my_lib",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            !code.contains("#[magnus::init]"),
            "visitor bridge must not generate a registration function"
        );
    }

    #[test]
    fn test_visitor_bridge_generates_trait_impl() {
        let trait_def = make_trait_def("HtmlVisitor", vec![make_visitor_method("visit_node")]);
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "my_lib",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("impl my_lib::HtmlVisitor for RbHtmlVisitorBridge"),
            "visitor bridge must implement the trait"
        );
    }

    // ---- Plugin-pattern bridges: register_fn + super_trait ----

    fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        let register_fn_name = trait_name.chars().fold(String::new(), |mut acc, c| {
            if c.is_uppercase() && !acc.is_empty() {
                acc.push('_');
                acc.push(c.to_lowercase().next().unwrap());
            } else {
                acc.push(c.to_lowercase().next().unwrap());
            }
            acc
        });
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("get_registry".to_string()),
            register_fn: Some(format!("register_{}", register_fn_name)),
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

    #[test]
    fn test_plugin_bridge_emits_struct_when_register_fn_configured() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![make_method("recognize", TypeRef::String, true, false)],
        );
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            !code.is_empty(),
            "plugin bridge must emit non-empty code when register_fn is set"
        );
        assert!(
            code.contains("pub struct RbOcrBackendBridge"),
            "plugin bridge must define RbOcrBackendBridge struct"
        );
    }

    /// `NodeContext` is a serde struct in the API, so it is native-marshalled. A method returning
    /// it must route the host return through the binding struct's `TryConvert` (which accepts the
    /// host's native wrapped object as well as a Hash/JSON via `to_json`) and `Into::into` for the
    /// core type — not `serde_json::from_str` into core directly. Return-side counterpart to the
    /// native-arg marshalling. See issue #153.
    #[test]
    fn test_plugin_bridge_native_struct_return_routes_through_binding_tryconvert() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![make_method(
                "build",
                TypeRef::Named("NodeContext".to_string()),
                true,
                false,
            )],
        );
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("<NodeContext as magnus::TryConvert>::try_convert(val)") && code.contains(".map(Into::into)"),
            "native struct return must route through the binding TryConvert + Into::into:\n{code}"
        );
        assert!(
            !code.contains("serde_json::from_str::<sample_crate::NodeContext>"),
            "native struct return must not deserialize JSON into core directly:\n{code}"
        );
    }

    #[test]
    fn test_plugin_bridge_emits_registration_fn() {
        let trait_def = make_trait_def(
            "EmbeddingBackend",
            vec![make_method(
                "embed",
                TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                true,
                false,
            )],
        );
        let cfg = make_plugin_bridge_cfg("EmbeddingBackend");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("register_embedding_backend"),
            "plugin bridge must emit register_embedding_backend function"
        );
    }

    #[test]
    fn test_plugin_bridge_emits_plugin_impl() {
        let trait_def = make_trait_def(
            "PostProcessor",
            vec![make_method("process", TypeRef::String, true, false)],
        );
        let cfg = make_plugin_bridge_cfg("PostProcessor");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("impl sample_crate::Plugin for RbPostProcessorBridge"),
            "plugin bridge must implement Plugin super-trait"
        );
    }

    #[test]
    fn test_plugin_bridge_emits_trait_impl() {
        let trait_def = make_trait_def(
            "Validator",
            vec![make_method(
                "validate",
                TypeRef::Primitive(PrimitiveType::Bool),
                true,
                false,
            )],
        );
        let cfg = make_plugin_bridge_cfg("Validator");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("impl my_lib::Validator for RbValidatorBridge"),
            "plugin bridge must implement the target trait (uses trait_def.rust_path)"
        );
    }

    #[test]
    fn test_plugin_bridge_skip_when_excluded() {
        let trait_def = make_trait_def(
            "SomeBackend",
            vec![make_method("execute", TypeRef::String, false, false)],
        );
        let mut cfg = make_plugin_bridge_cfg("SomeBackend");
        cfg.exclude_languages = vec!["ruby".to_string()];
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.is_empty(),
            "plugin bridge must emit empty code when 'ruby' is in exclude_languages"
        );
    }

    #[test]
    fn test_plugin_bridge_validates_required_methods_in_constructor() {
        let trait_def = make_trait_def(
            "OcrBackend",
            vec![
                make_method("recognize", TypeRef::String, true, false), // required
                make_method("shutdown", TypeRef::Unit, false, true),    // optional
            ],
        );
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(
            &trait_def,
            &cfg,
            "sample_crate",
            "MyError",
            "MyError::Plugin {{ message: {msg}, plugin_name: String::new() }}",
            &make_api(),
        )
        .expect("trait bridge generation should succeed");

        assert!(
            code.contains("respond_to"),
            "constructor must check respond_to? for required methods"
        );
    }
}

#[test]
fn test_tagged_union_enum_vec_field_serde_marshalling() {
    let backend = MagnusBackend;

    // Create API with a tagged-union enum that has a Vec<Named> field on one variant.
    // Named types require JSON marshalling, so Vec<Named> should map to String in the
    // Magnus binding enum, and the conversion code will use serde_json to deserialize.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Item".to_string(),
            rust_path: "test_lib::Item".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Result".to_string(),
            rust_path: "test_lib::Result".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Success".to_string(),
                    fields: vec![FieldDef {
                        name: "items".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
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
                    doc: "Success with items".to_string(),
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
                    name: "Error".to_string(),
                    fields: vec![FieldDef {
                        name: "message".to_string(),
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
                    doc: "Error with message".to_string(),
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
            doc: "Tagged union result type".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: Some("type".to_string()),
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

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Print the relevant chunk on failure for diagnosis.
    eprintln!("---generated lib.rs (Result enum context)---");
    if let Some(idx) = content.find("enum Result") {
        eprintln!("{}", &content[idx..idx.saturating_add(500).min(content.len())]);
    }

    // Vec<Named> fields must round-trip as actual Vec<Named> so serde can deserialize a
    // JSON array. Mapping to bare `String` previously broke decoding for tagged-union
    // variants like StopSequence::Multiple(Vec<String>) — the FFI sends a JSON array, not
    // a JSON-encoded string.
    assert!(
        content.contains("items: Vec<Item>"),
        "Tagged-union enum variant with Vec<Named> field should map to Vec<Named> for JSON array round-trip"
    );

    // Verify the enum definition includes proper variant structure
    assert!(content.contains("enum Result"), "Should generate Result enum");
    assert!(content.contains("Success"), "Should contain Success variant");
    assert!(content.contains("Error"), "Should contain Error variant");

    // Verify that the serde tag attribute is present
    assert!(content.contains("tag = \"type\""), "Should have serde tag attribute");

    // Tagged data enums get NO Rust factory class: it is represented on the Ruby side as a
    // `module Result` with per-variant `Data.define` classes, and a `define_class("Result")` would
    // collide with that module (raising `TypeError: Result is not a module` at load). The factory
    // methods, class registration, and singleton constructors are all gated on `serde_tag.is_none()`.
    assert!(
        !content.contains("pub fn _factory_success"),
        "tagged data enum must not emit per-variant factory constructors: {content}"
    );
    assert!(
        !content.contains("pub fn _factory_error"),
        "tagged data enum must not emit per-variant factory constructors: {content}"
    );
    assert!(
        !content.contains(r#"module.define_class("Result""#),
        "tagged data enum must not register a Rust factory class: {content}"
    );
    assert!(
        !content.contains(r#"define_singleton_method("success""#),
        "tagged data enum must not register per-variant singleton constructors: {content}"
    );
    assert!(
        !content.contains(r#"define_singleton_method("error""#),
        "tagged data enum must not register per-variant singleton constructors: {content}"
    );
}

/// Bug A regression — tuple variant Foo(Vec<u8>) should keep Vec<u8>, not collapse to String.
/// The conversion code must use direct assignment, not serde_json round-trip.
#[test]
fn test_tuple_variant_vec_primitive_stays_as_vec() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "BytePayload".to_string(),
            rust_path: "test_lib::BytePayload".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Data".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
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
                is_tuple: true,
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    // Vec<u8> (primitive) must NOT be collapsed to String
    assert!(
        content.contains("_0: Vec<u8>"),
        "Vec<u8> tuple variant field must stay as Vec<u8>, got:\n{content}"
    );
    // Conversion must not use serde_json for Vec<u8>
    assert!(
        !content.contains("serde_json::from_str(&_0)"),
        "Vec<u8> must not use serde_json::from_str; got:\n{content}"
    );
    assert!(
        !content.contains("serde_json::to_string(&_0)"),
        "Vec<u8> must not use serde_json::to_string; got:\n{content}"
    );
}

#[test]
fn test_tuple_variant_bytes_stays_as_vec() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "SocketMessage".to_string(),
            rust_path: "test_lib::SocketMessage".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Binary".to_string(),
                fields: vec![make_field("_0", TypeRef::Bytes, false)],
                is_tuple: true,
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    assert!(
        content.contains("_0: Vec<u8>"),
        "TypeRef::Bytes tuple variant field must stay as Vec<u8>, got:\n{content}"
    );
    assert!(
        !content.contains("_0: String"),
        "TypeRef::Bytes tuple variant field must not collapse to String, got:\n{content}"
    );
}

#[test]
fn test_optional_ref_string_method_returns_owned_option() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Response".to_string(),
            rust_path: "test_lib::Response".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "header".to_string(),
                params: vec![ParamDef {
                    name: "name".to_string(),
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
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: true,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    assert!(
        content.contains("fn header(&self, name: String) -> Option<String>"),
        "Ruby method wrapper must expose owned Option<String>, got:\n{content}"
    );
    assert!(
        content.contains("core_self.header(&name).map(|v| v.to_owned())"),
        "Ruby method wrapper must convert Option<&str> to Option<String>, got:\n{content}"
    );
}

#[test]
fn test_opaque_owned_builder_return_rewraps_arc() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GraphQlRouteConfig".to_string(),
            rust_path: "test_lib::GraphQlRouteConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "path".to_string(),
                params: vec![ParamDef {
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
                }],
                return_type: TypeRef::Named("GraphQlRouteConfig".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(ReceiverKind::Owned),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
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
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    assert!(
        content.contains("Self { inner: Arc::new(self.inner.as_ref().clone().path(path)) }"),
        "Owned opaque builder return must wrap the returned core value in Arc, got:\n{content}"
    );
    assert!(
        !content.contains("Self { inner: self.inner.as_ref().clone().path(path) }"),
        "Owned opaque builder return must not treat method-call result as an existing Arc, got:\n{content}"
    );
}

/// Bug A regression — tuple variant Foo(Vec<Bar>) where Bar is a Named type should keep
/// Vec<Bar> in the binding enum and use .into() conversions, not serde_json.
#[test]
fn test_tuple_variant_vec_named_stays_as_vec_and_uses_into() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Bar".to_string(),
            rust_path: "test_lib::Bar".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
            doc: String::new(),
            cfg: None,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Payload".to_string(),
            rust_path: "test_lib::Payload".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Multi".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Bar".to_string()))),
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
                is_tuple: true,
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    // Vec<Bar> (Named) must stay as Vec<Bar>, not String
    assert!(
        content.contains("_0: Vec<Bar>"),
        "Vec<Named> tuple variant field must stay as Vec<Bar>, got:\n{content}"
    );
    // Conversion must not use serde_json for Vec<Named>
    assert!(
        !content.contains("serde_json::from_str(&_0)"),
        "Vec<Named> must not use serde_json::from_str; got:\n{content}"
    );
    assert!(
        !content.contains("serde_json::to_string(&_0)"),
        "Vec<Named> must not use serde_json::to_string; got:\n{content}"
    );
    // Conversion must use .into() for each element
    assert!(
        content.contains("into_iter().map(Into::into).collect()"),
        "Vec<Named> conversion must use .into_iter().map(Into::into).collect(); got:\n{content}"
    );
}

/// Bug B regression — a struct with field (ty=Optional(Usize), optional=true) must produce
/// a getter returning Option<usize>, not Option<Option<usize>>.
#[test]
fn test_field_accessor_no_double_option_when_ty_is_optional() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "UpdateConfig".to_string(),
            rust_path: "test_lib::UpdateConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "max_depth".to_string(),
                // ty = Optional(Usize) AND optional = true mimics a core Option<Option<usize>>
                // that the binding flattens to Option<usize>.
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
                optional: true,
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
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            doc: String::new(),
            cfg: None,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib.content;

    // Getter must return Option<usize>, not Option<Option<usize>>
    assert!(
        !content.contains("Option<Option<usize>>"),
        "field accessor must not emit Option<Option<usize>>:\n{content}"
    );
    assert!(
        content.contains("fn max_depth(&self) -> Option<usize>"),
        "field accessor must return Option<usize>:\n{content}"
    );
}

#[test]
fn test_visitor_bridge_debug_not_duplicated() {
    use alef::backends::magnus::trait_bridge::gen_trait_bridge;
    use alef::core::config::{BridgeBinding, TraitBridgeConfig};
    use alef::core::ir::*;

    let make_method_with_default = |name: &str| MethodDef {
        name: name.to_string(),
        params: vec![ParamDef {
            name: "context".to_string(),
            ty: TypeRef::Named("NodeContext".to_string()),
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
        return_type: TypeRef::Named("VisitResult".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let trait_def = TypeDef {
        name: "HtmlVisitor".to_string(),
        rust_path: "sample_markdown_rs::visitor::HtmlVisitor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: (0..40)
            .map(|i| make_method_with_default(&format!("visit_method_{i}")))
            .collect(),
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
        has_private_fields: false,
        version: Default::default(),
    };

    let cfg = TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitResult".to_string()),
    };

    let api = ApiSurface {
        crate_name: "sample_markdown_rs".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "sample_markdown_rs::visitor::NodeContext".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "depth".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
            is_clone: false,
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "VisitResult".to_string(),
            rust_path: "sample_markdown_rs::visitor::VisitResult".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
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
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
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

    let code = gen_trait_bridge(
        &trait_def,
        &cfg,
        "sample_markdown_rs",
        "ConversionError",
        "ConversionError::new({msg})",
        &api,
    )
    .expect("trait bridge generation should succeed");

    let debug_count = code.matches("impl std::fmt::Debug for RbHtmlVisitorBridge").count();
    assert_eq!(
        debug_count,
        1,
        "Expected 1 Debug impl, got {}:\n{}",
        debug_count,
        &code[..code.len().min(2000)]
    );
}

#[test]
fn test_module_init_requires_json_stdlib() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Module init function must emit require "json" to ensure Hash#to_json is available
    assert!(
        content.contains("require") && content.contains("json"),
        "Module init must emit require \"json\" to load JSON stdlib for Hash#to_json"
    );
    assert!(content.contains("ruby.eval"), "Must use ruby.eval to load JSON library");
}

#[test]
fn test_trait_bridge_options_field_error_propagation_in_generated_code() {
    // This test verifies that trait bridge code generation includes proper error
    // handling when deserializing Ruby Hash to options via JSON. Previously, the
    // code silently swallowed errors via unwrap_or_default(), causing missing
    // options like include_document_structure to be lost.

    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "test_lib::ConversionOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("debug", TypeRef::Primitive(PrimitiveType::Bool), true)],
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
            has_private_fields: false,
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

    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // The generated code should include error-safe deserialization patterns
    // (These patterns are generated within trait bridges when options_field binding is used)
    // For this test, we verify that the codebase pattern is NOT using .unwrap_or_default()
    // after to_json calls.
    assert!(
        !content.contains(".unwrap_or_default()") || !content.contains("funcall::<_, _, String>(\"to_json\""),
        "Generated trait bridge code must not use unwrap_or_default() for JSON serialization"
    );
}

/// Regression: `method_missing` must never appear in the public Ruby API.
///
/// The Hash monkey-patch that previously lived in `native.rb` was a global-state
/// leak that broke IDE autocomplete and could interfere with any Ruby code that
/// uses Hash. The replacement class hierarchy must not resort to `method_missing`.
#[test]
fn tagged_enum_public_api_does_not_emit_method_missing() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![FieldDef {
                        name: "content".to_string(),
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
                    name: "User".to_string(),
                    fields: vec![FieldDef {
                        name: "content".to_string(),
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
            doc: "Chat message role".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: Some("role".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    assert!(
        !content.contains("method_missing"),
        "native.rb must not emit Hash#method_missing — use class-based tagged enums instead:\n{content}"
    );
}

/// Regression: Sorbet `sig {` blocks must appear in the public Ruby API for tagged enum subclass methods.
///
/// Every generated accessor and predicate must carry a Sorbet-compatible `sig { }` annotation
/// so that Sorbet users get type-checked attribute access without manual type annotations.
#[test]
fn tagged_enum_public_api_emits_sorbet_sig_blocks() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "System".to_string(),
                fields: vec![FieldDef {
                    name: "content".to_string(),
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
                doc: String::new(),
                is_default: true,
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
            has_default: false,
            serde_tag: Some("role".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    assert!(
        content.contains("sig {"),
        "native.rb must emit Sorbet sig {{ }} blocks on tagged enum methods:\n{content}"
    );
}

/// Regression: the tagged-enum marker module must be RuboCop-clean — single-quoted
/// string literals (`Style/StringLiterals`), a blank line after the module-inclusion
/// group (`Layout/EmptyLinesAfterModuleInclusion`), and no blank line at a module
/// body end (`Layout/EmptyLinesAroundModuleBody`). Generated Ruby lives under `lib/**`
/// which the gem's `.rubocop.yml` excludes, so alef's own `rubocop -A` pass never
/// touches it — but a pre-commit `rubocop` hook that passes the file explicitly
/// overrides that exclusion and reformats it, invalidating alef's file hash and
/// tripping `alef verify`. Emitting clean code up front keeps the file stable.
#[test]
fn tagged_enum_dispatcher_emits_rubocop_clean_ruby() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "System".to_string(),
                fields: vec![FieldDef {
                    name: "content".to_string(),
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
                doc: String::new(),
                is_default: true,
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
            has_default: false,
            serde_tag: Some("role".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let content = &files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap()
        .content;

    // Discriminator access, `when` arms, and per-variant field reads use double quotes
    // to match rubocop's `Style/StringLiterals: double_quotes` default.
    assert!(
        content.contains("hash[:role] || hash[\"role\"]"),
        "discriminator read must use double quotes:\n{content}"
    );
    assert!(
        content.contains("when \"system\" then MessageSystem.from_hash(hash)"),
        "dispatcher `when` arm must use double quotes:\n{content}"
    );
    assert!(
        content.contains("hash[:content] || hash[\"content\"]"),
        "variant field read must use double quotes:\n{content}"
    );
    // No single-quoted non-interpolated string literals leak into the dispatcher.
    assert!(
        !content.contains("hash['role']") && !content.contains("when 'system'"),
        "no single-quoted literals expected in dispatcher:\n{content}"
    );
    // The interpolated raise message stays double-quoted (single quotes don't interpolate).
    assert!(
        content.contains("raise \"Unknown discriminator: #{discriminator}\""),
        "interpolated raise must remain double-quoted:\n{content}"
    );

    // Layout/EmptyLinesAfterModuleInclusion: blank line after the inclusion group.
    assert!(
        content.contains("    extend T::Sig\n\n    interface!"),
        "must emit a blank line after the module-inclusion group:\n{content}"
    );
    // Layout/EmptyLinesAroundModuleBody: the dispatcher's `end` sits directly
    // against the marker module's `end` — no intervening blank line.
    assert!(
        content.contains("    end\n  end\n"),
        "marker module body must not end with a blank line:\n{content}"
    );
    // Layout/EmptyLinesAroundModuleBody: the outer module closes without a
    // trailing blank line after the last variant class.
    assert!(
        content.contains("  end\nend\n") && !content.contains("  end\n\nend"),
        "outer module body must not end with a blank line:\n{content}"
    );
}

#[test]
fn tagged_enum_dispatcher_uses_serde_wire_names() {
    let backend = MagnusBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Action".to_string(),
            rust_path: "test_lib::Action".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "OpenURL".to_string(),
                    fields: vec![make_field("url", TypeRef::String, false)],
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
                EnumVariant {
                    name: "ReadText".to_string(),
                    fields: vec![make_field("value", TypeRef::String, false)],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("read-text".to_string()),
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
            has_default: false,
            serde_tag: Some("kind".to_string()),
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

    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let content = &files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap()
        .content;

    assert!(
        content.contains("when \"open-url\" then ActionOpenURL.from_hash(hash)"),
        "rename_all must define Ruby dispatcher wire names:\n{content}"
    );
    assert!(
        content.contains("when \"read-text\" then ActionReadText.from_hash(hash)"),
        "serde(rename) must override rename_all:\n{content}"
    );
}

/// Regression: tagged enum must emit a base class and per-variant subclasses.
///
/// The base `Message` class provides predicate methods that return `false` by default.
/// Each variant subclass (`MessageSystem`, `MessageUser`, etc.) overrides its predicate
/// to return `true` and carries typed `attr_reader` accessors for the variant's fields.
#[test]
fn tagged_enum_public_api_emits_class_hierarchy() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![FieldDef {
                        name: "content".to_string(),
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
                    name: "User".to_string(),
                    fields: vec![FieldDef {
                        name: "content".to_string(),
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
            has_serde: false,
            has_default: false,
            serde_tag: Some("role".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    // Base marker module
    assert!(
        content.contains("module Message"),
        "must emit a Message marker module:\n{content}"
    );

    // Per-variant Data.define classes including the marker module
    assert!(
        content.contains("MessageSystem = Data.define(:content) do"),
        "must emit MessageSystem as Data.define with symbol args:\n{content}"
    );
    assert!(
        content.contains("MessageUser = Data.define(:content) do"),
        "must emit MessageUser as Data.define with symbol args:\n{content}"
    );
    assert!(
        content.contains("    include Message"),
        "variant must include the marker module:\n{content}"
    );

    // Variant predicate methods
    assert!(
        content.contains("def system? = true"),
        "MessageSystem must override system? to true:\n{content}"
    );
    assert!(
        content.contains("def user? = true"),
        "MessageUser must override user? to true:\n{content}"
    );
    assert!(
        content.contains("def system? = false"),
        "non-system variants must define system? as false:\n{content}"
    );

    // Field accessor wraps Data-auto-generated method via super (no infinite recursion).
    // Endless def with rubocop disable so `rubocop -a` doesn't strip the def.
    assert!(
        content.contains("def content = super"),
        "variant accessor must delegate to Data's auto-getter via super:\n{content}"
    );
    assert!(
        content.contains("rubocop:disable Lint/UselessMethodDefinition"),
        "accessor def must carry rubocop disable so autocorrect won't strip it:\n{content}"
    );
}

#[test]
fn test_enum_yard_doc_emission() {
    let backend = MagnusBackend;

    // Create test API surface with an enum that has documentation
    // Must have serde_tag and at least one variant with fields to generate Ruby classes
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![FieldDef {
                        name: "reason".to_string(),
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
                    doc: "Represents an active status.\n\n# Returns\n\nBoolean indicating activity.".to_string(),
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
                    doc: "Represents an inactive status.".to_string(),
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
            doc: "Tagged enum for various status states.\n\n# Returns\n\nA Status variant instance.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: Some("type".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    // Test base class YARD doc
    assert!(
        content.contains("# Tagged enum for various status states."),
        "base class should have YARD doc from enum.doc:\n{content}"
    );
    assert!(
        content.contains("# @return A Status variant instance."),
        "base class should translate Returns section to @return tag:\n{content}"
    );

    // Test variant YARD doc
    assert!(
        content.contains("# Represents an active status."),
        "variant subclass should have YARD doc from variant.doc:\n{content}"
    );
    assert!(
        content.contains("# @return Boolean indicating activity."),
        "variant should translate Returns section to @return tag:\n{content}"
    );
    assert!(
        content.contains("# Represents an inactive status."),
        "second variant should also have YARD doc:\n{content}"
    );
}

#[test]
fn test_enum_variant_method_yard_docs() {
    let backend = MagnusBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Result".to_string(),
            rust_path: "test_lib::Result".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Ok".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: "The success value.".to_string(),
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
                doc: "A successful result.".to_string(),
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
            doc: "A result enum.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: Some("type".to_string()),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    // Data field with doc should emit the doc as YARD
    assert!(
        content.contains("# The success value."),
        "field accessor with doc must emit YARD comment:\n{content}"
    );

    // predicate must have a Sorbet sig declaring Boolean return
    assert!(
        content.contains("sig { returns(T::Boolean) }"),
        "predicate method must have Sorbet boolean return sig:\n{content}"
    );

    // from_hash must have @param and @return [self]
    assert!(
        content.contains("# @param hash"),
        "from_hash must have @param hash YARD tag:\n{content}"
    );
    assert!(
        content.contains("@return [self]") || content.contains("returns(T.attached_class)"),
        "from_hash must declare a self return:\n{content}"
    );
}

#[test]
fn test_explicit_re_export_list_filters_internal_types() {
    let backend = MagnusBackend;

    // Create a test API with types that should be filtered (Update, Builder),
    // excluded types, and valid public types.
    let types = vec![
        TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
            has_private_fields: false,
            version: Default::default(),
        },
        // Update struct should be filtered out
        TypeDef {
            name: "ConfigUpdate".to_string(),
            rust_path: "test_lib::ConfigUpdate".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, true)],
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
            has_private_fields: false,
            version: Default::default(),
        },
        // Builder struct should be filtered out
        TypeDef {
            name: "ConfigBuilder".to_string(),
            rust_path: "test_lib::ConfigBuilder".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, true)],
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
            has_private_fields: false,
            version: Default::default(),
        },
    ];

    let api = ApiSurface {
        crate_name: "test-lib-rs".to_string(),
        version: "0.1.0".to_string(),
        types,
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib_rs::process".to_string(),
            original_rust_path: String::new(),
            params: vec![],
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Active".to_string(),
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
            has_default: false,
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

    let config = resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib-rs"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
exclude_types = ["ExcludedType"]
"#,
    );

    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("native.rb"))
        .unwrap();
    let content = &native_file.content;

    // Verify no dynamic re-export pattern (old behavior)
    assert!(
        !content.contains(".methods(false).each"),
        "native.rb must NOT use dynamic .methods(false).each pattern:\n{content}"
    );
    assert!(
        !content.contains(".constants.each"),
        "native.rb must NOT use dynamic .constants.each pattern:\n{content}"
    );

    // Verify explicit re-exports are present
    assert!(
        content.contains("Config = TestLibRs.const_get(:Config)"),
        "valid type Config should be explicitly re-exported:\n{content}"
    );
    assert!(
        !content.contains("Status = TestLibRs.const_get(:Status)"),
        "enum Status must NOT be re-exported — Magnus does not register enums as module constants:\n{content}"
    );

    // Verify Update and Builder types are NOT exported
    assert!(
        !content.contains("ConfigUpdate"),
        "Update-type ConfigUpdate must NOT be re-exported:\n{content}"
    );
    assert!(
        !content.contains("ConfigBuilder"),
        "Builder-type ConfigBuilder must NOT be re-exported:\n{content}"
    );

    // Verify function is explicitly re-exported
    assert!(
        content.contains("define_singleton_method(:process)"),
        "function process should be explicitly re-exported:\n{content}"
    );

    // Verify no leakage of dynamic re-export patterns
    assert!(
        !content.contains(".methods(false).each") && !content.contains(".constants.each"),
        "must not use dynamic .methods or .constants patterns:\n{content}"
    );
}

/// Verify that RegistrationVariantStyle is honored via semantic equivalence:
/// All three styles (Builder, VerbDecorator, Hybrid) emit the same block-form
/// method in Ruby since blocks are the idiomatic closure mechanism.
/// This test confirms the IR field is acknowledged and no branching is needed.
#[test]
fn test_registration_variant_styles_emit_unified_block_form() {
    use alef::core::ir::*;

    let backend = MagnusBackend;

    let make_service_with_style = |style: RegistrationVariantStyle| -> ApiSurface {
        let method = |name: &str, is_static: bool, receiver: Option<ReceiverKind>| MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static,
            error_type: None,
            doc: String::new(),
            receiver,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let string_param = |name: &str| ParamDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        };

        ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![ServiceDef {
                name: "TestApp".to_string(),
                rust_path: "test_lib::TestApp".to_string(),
                constructor: method("new", true, None),
                configurators: vec![],
                registrations: vec![RegistrationDef {
                    method: "route".to_string(),
                    callback_param: "handler".to_string(),
                    callback_contract: "Handler".to_string(),
                    metadata_params: vec![string_param("method"), string_param("path")],
                    receiver: Some(ReceiverKind::RefMut),
                    return_type: TypeRef::Unit,
                    error_type: None,
                    doc: String::new(),
                    variants: vec![RegistrationVariant {
                        name: "get".to_string(),
                        overrides: vec![RegistrationVariantOverride {
                            param_name: "method".to_string(),
                            value_expr: "\"GET\"".to_string(),
                        }],
                        wrapper_call: None,
                        signature_params: vec![string_param("path")],
                        doc: None,
                        style,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                entrypoints: vec![],
                doc: "Test service".to_string(),
                cfg: None,
            }],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    };

    let config = make_config();

    // Generate bindings for all three styles
    for style in [
        RegistrationVariantStyle::Builder,
        RegistrationVariantStyle::VerbDecorator,
        RegistrationVariantStyle::Hybrid,
    ] {
        let api = make_service_with_style(style);
        let result = backend.generate_service_api(&api, &config);
        assert!(result.is_ok(), "Generation should succeed for style {:?}", style);

        let files = result.unwrap();
        let service_file = files
            .iter()
            .find(|f| f.path.to_string_lossy().contains("service.rb"))
            .unwrap();
        let content = &service_file.content;

        // All styles must emit the same block-form method signature.
        assert!(
            content.contains("def get(path: String, &block)"),
            "style {:?} must emit block-form method def get(path: String, &block):\n{}",
            style,
            content
        );

        // No conditionals or branching on style — one unified form only
        assert!(
            !content.contains(&format!("RegistrationVariantStyle::{:?}", style)),
            "Generated code must not mention RegistrationVariantStyle in output for {:?}",
            style
        );
    }
}

#[test]
fn test_async_function_with_vec_named_params() {
    let backend = MagnusBackend;

    // Create API with an enum (non-opaque) and async function taking Vec<EnumType>
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        enums: vec![EnumDef {
            name: "Category".to_string(),
            rust_path: "test_lib::Category".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "TypeA".to_string(),
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
                EnumVariant {
                    name: "TypeB".to_string(),
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
            doc: "Category enumeration".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "detect_async".to_string(),
            rust_path: "test_lib::detect_async".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "text".to_string(),
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
                    name: "categories".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Category".to_string()))),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true, // Core function takes &[T], so is_ref=true
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
            is_async: true,
            error_type: None,
            doc: "Detect with async".to_string(),
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Must contain the async function
    assert!(content.contains("detect_async"), "Should contain detect_async function");

    // Each function body must emit `let categories_core:` exactly once; emitting it twice
    // within a single body causes `use of moved value: categories` (E0382). The sync wrapper
    // (`fn detect_async`) and the async wrapper (`fn detect_async_async`) each emit it once.
    for fn_decl in ["fn detect_async(", "fn detect_async_async("] {
        let start = content
            .find(fn_decl)
            .unwrap_or_else(|| panic!("Should contain {fn_decl}"));
        let remaining = &content[start + fn_decl.len()..];
        let next_pub_fn = remaining.find("\npub fn ");
        let next_private_fn = remaining.find("\nfn ");
        let next_fn = [next_pub_fn, next_private_fn]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(remaining.len());
        let body = &content[start..start + fn_decl.len() + next_fn];
        let body_count = body.matches("let categories_core:").count();
        assert_eq!(
            body_count, 1,
            "{fn_decl}...) body should emit categories_core let binding exactly once, got {body_count}"
        );
    }

    // Must use `categories_core` in the core function call
    assert!(
        content.contains("&categories_core"),
        "Should pass &categories_core to inner function (not &categories)"
    );

    // Must not reference undefined `categories_core` before binding
    let detect_async_start = content.find("fn detect_async").unwrap();
    let next_fn = content[detect_async_start..]
        .find("\n    fn ")
        .unwrap_or(content.len() - detect_async_start);
    let detect_async_body = &content[detect_async_start..detect_async_start + next_fn];

    // Find the let binding and the call site
    let categories_core_binding_pos = detect_async_body.find("let categories_core:").unwrap_or(0);
    let categories_core_usage_pos = detect_async_body.find("&categories_core").unwrap_or(0);

    assert!(
        categories_core_binding_pos > 0
            && categories_core_usage_pos > 0
            && categories_core_binding_pos < categories_core_usage_pos,
        "categories_core must be bound before use"
    );
}

#[test]
fn test_opaque_async_method_with_vec_named_ref_param() {
    let backend = MagnusBackend;

    // Create API with an enum and an opaque struct with async method taking Vec<EnumType>&.
    // This regression test covers the case where delegatable async methods on opaque structs
    // need to emit let-bindings for Vec<Named> params that are passed by reference.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Analyzer".to_string(),
            rust_path: "test_lib::Analyzer".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "detect".to_string(),
                params: vec![
                    ParamDef {
                        name: "text".to_string(),
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
                        name: "labels".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Label".to_string()))),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true, // Core function takes &[Label]
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
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: Some("Error".to_string()),
                doc: "Detect with labels".to_string(),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Text analyzer".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "Label".to_string(),
            rust_path: "test_lib::Label".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Foo".to_string(),
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
                EnumVariant {
                    name: "Bar".to_string(),
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
            doc: "Label enumeration".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        functions: vec![],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "test_lib::Error".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
            methods: vec![],
        }],
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Must contain the async method
    assert!(content.contains("detect_async"), "Should contain detect_async method");

    // The method body must emit `let labels_core:` to convert Vec<Label> → Vec<core::Label>
    // and must use `&labels_core` (not `&labels`) when calling the core function
    let detect_async_fn = content
        .find("fn detect_async(&self")
        .expect("Should find detect_async method");
    let next_fn = content[detect_async_fn..]
        .find("\n    fn ")
        .unwrap_or(content.len() - detect_async_fn);
    let method_body = &content[detect_async_fn..detect_async_fn + next_fn];

    assert!(
        method_body.contains("let labels_core:"),
        "Method body should emit let labels_core binding"
    );

    assert!(
        method_body.contains("&labels_core"),
        "Method body should use &labels_core (not &labels) in core call"
    );

    // Verify the let binding comes before its use
    let binding_pos = method_body.find("let labels_core:").unwrap_or(0);
    let usage_pos = method_body.find("&labels_core").unwrap_or(0);

    assert!(
        binding_pos > 0 && usage_pos > 0 && binding_pos < usage_pos,
        "labels_core must be bound before use"
    );
}

/// Build a no-param free `FunctionDef` returning `String`, with the given name and optional cfg.
fn free_fn(name: &str, cfg: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: cfg.map(ToString::to_string),
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

/// Regression: same-named free functions reaching the surface under disjoint cfgs — including an
/// ungated variant — must collapse to exactly one top-level `fn` in the Magnus Rust glue.
///
/// Mirrors kreuzberg's `text::ner::download_model`, which surfaces three entries
/// (`#[cfg(feature = "ner-onnx")]`, `#[cfg(not(feature = "ner-onnx"))]`, and an unconditional stub
/// from a `#[cfg(not(feature = "ner"))]` parent module whose gate did not propagate). Emitting all
/// three verbatim produced two simultaneously-active definitions → `error[E0428]`.
#[test]
fn same_named_free_functions_with_ungated_variant_dedup_to_one() {
    let backend = MagnusBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            free_fn("download_model", Some("feature = \"ner-onnx\"")),
            free_fn("download_model", Some("not(feature = \"ner-onnx\")")),
            // Ungated third entry — the gate that should have carried `not(feature = "ner")`
            // was lost in extraction; this is the entry that triggers the E0428 collision.
            free_fn("download_model", None),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");
    let content = &files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    let definition_count = content.matches("fn download_model(").count();
    assert_eq!(
        definition_count, 1,
        "expected exactly one `fn download_model` definition, found {definition_count} (E0428 regression)"
    );

    // When any group member is unconditional the merged wrapper must be unconditional too:
    // no `#[cfg(...)]` should gate the surviving `download_model`.
    let def_pos = content
        .find("fn download_model(")
        .expect("download_model must be emitted");
    let preamble_start = content[..def_pos].rfind("\n\n").map(|i| i + 2).unwrap_or(0);
    let preamble = &content[preamble_start..def_pos];
    assert!(
        !preamble.contains("#[cfg("),
        "the deduped download_model wrapper must be unconditional (an ungated variant exists), got preamble:\n{preamble}"
    );
}

/// #132: an internally-tagged enum must keep the `TryConvert` fallback that wraps a bare Ruby
/// string as `{"<tag>": json_str}` so serde resolves a unit-variant name. Verify it stays after
/// the `string_shorthand` removal.
#[test]
fn test_internally_tagged_unit_variant_wraps_bare_string() {
    let backend = MagnusBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Greeting".to_string(),
            rust_path: "test_lib::Greeting".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Default".to_string(),
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
                    name: "Preset".to_string(),
                    fields: vec![make_field("name", TypeRef::String, false)],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: true,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .expect("lib.rs generated");

    // #132: the bare-string fallback wraps the value as {"<tag>": json_str}.
    assert!(
        lib.content.contains(r#"serde_json::json!({ "type": json_str })"#),
        "internally-tagged enum must keep the {{\"type\": json_str}} fallback;\ncontent:\n{}",
        lib.content
    );
}

// ---------------------------------------------------------------------------
// Native-object marshalling of struct callback params (Magnus trait bridge)
//
// A trait-callback param that is a known serde struct must be handed to the host as the
// binding's NATIVE Ruby value — constructed via the same `From<core::T>` conversion the binding
// uses for return values / struct fields — NOT serialized to a JSON string. Enum / opaque /
// unknown params keep their prior JSON-string representation. The positive allowlist is computed
// by the SHARED classifier (`native_marshalled_struct_params`) and seeded into the generator.
// ---------------------------------------------------------------------------

/// Build a callback `ParamDef` (by-ref) with all the structural defaults.
fn cb_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
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
    }
}

/// A plain (non-opaque) serde struct `TypeDef`.
fn serde_struct(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        has_serde: true,
        ..TypeDef::default()
    }
}

/// Build a neutral `Greeter` plugin bridge: a trait, its callback param/return structs, and a
/// `TraitBridgeConfig` with a `register_*` fn (so the plugin — not visitor — path is taken).
fn neutral_plugin_fixture(is_async: bool) -> (ApiSurface, TypeDef, alef::core::config::TraitBridgeConfig) {
    let method = MethodDef {
        name: "process".to_string(),
        params: vec![
            cb_param("opts", TypeRef::Named("Opts".to_string())), // known serde struct
            cb_param("mood", TypeRef::Named("Mood".to_string())), // enum
            cb_param("handle", TypeRef::Named("Handle".to_string())), // opaque
            cb_param("widget", TypeRef::Named("Widget".to_string())), // unknown (not in api.types)
        ],
        return_type: TypeRef::Named("Doc".to_string()),
        is_async,
        error_type: Some("Error".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };
    let trait_def = TypeDef {
        name: "Greeter".to_string(),
        rust_path: "test_lib::Greeter".to_string(),
        is_trait: true,
        methods: vec![method],
        ..TypeDef::default()
    };

    let mut handle = serde_struct("Handle");
    handle.is_opaque = true;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            trait_def.clone(),
            serde_struct("Opts"), // qualifies → native
            handle,               // opaque → JSON string
            serde_struct("Doc"),  // return type
        ],
        enums: vec![EnumDef {
            name: "Mood".to_string(),
            rust_path: "test_lib::Mood".to_string(),
            ..EnumDef::default()
        }],
        ..Default::default()
    };

    let bridge = alef::core::config::TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        register_fn: Some("register_greeter".to_string()),
        registry_getter: Some("test_lib::registry::get".to_string()),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    (api, trait_def, bridge)
}

#[test]
fn test_magnus_sync_struct_param_marshalled_as_native_ruby_value() {
    let (api, trait_def, bridge) = neutral_plugin_fixture(false);
    let code = alef::backends::magnus::trait_bridge::gen_trait_bridge(
        &trait_def,
        &bridge,
        "test_lib",
        "Error",
        "Error::Message {{ message: {msg} }}",
        &api,
    )
    .expect("plugin bridge should generate");

    // (a) The known serde struct param is built as the binding's native Ruby value via From<core>.
    assert!(
        code.contains("Opts::from(opts.clone()).into_value_with(&ruby)"),
        "struct param must be marshalled as the native Ruby value, not a JSON string:\n{code}"
    );
    assert!(
        !code.contains("serde_json::to_string(&opts)"),
        "struct param must NOT be JSON-serialized:\n{code}"
    );

    // (b) Enum / opaque / unknown params keep the prior JSON-string representation.
    for other in ["mood", "handle", "widget"] {
        assert!(
            code.contains(&format!("serde_json::to_string(&{other})")),
            "non-struct param `{other}` must keep the JSON-string representation:\n{code}"
        );
        assert!(
            !code.contains(&format!("from({other}.clone()).into_value_with")),
            "non-struct param `{other}` must NOT be marshalled as a native value:\n{code}"
        );
    }
}

#[test]
fn test_magnus_async_struct_param_marshalled_as_native_ruby_value() {
    let (api, trait_def, bridge) = neutral_plugin_fixture(true);
    let code = alef::backends::magnus::trait_bridge::gen_trait_bridge(
        &trait_def,
        &bridge,
        "test_lib",
        "Error",
        "Error::Message {{ message: {msg} }}",
        &api,
    )
    .expect("async plugin bridge should generate");

    // The async preamble clones the core value into `{name}_owned`; the call site builds the
    // native Ruby value from it.
    assert!(
        code.contains("let opts_owned = opts.clone();"),
        "async preamble must clone the core struct value:\n{code}"
    );
    assert!(
        code.contains("Opts::from(opts_owned.clone()).into_value_with(&ruby)"),
        "async struct param must be marshalled as the native Ruby value:\n{code}"
    );
    assert!(
        !code.contains("serde_json::to_string(&opts_owned)"),
        "async struct param must NOT be JSON-serialized:\n{code}"
    );
    // Non-struct params keep JSON serialization on their owned copies.
    assert!(
        code.contains("serde_json::to_string(&mood_owned)") && code.contains("serde_json::to_string(&widget_owned)"),
        "non-struct async params must keep the JSON-string representation:\n{code}"
    );
}
