use alef::backends::wasm::WasmBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

/// Helper to create a field definition with all defaults
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

/// Helper to create minimal ResolvedCrateConfig with WASM enabled
fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_basic_generation() {
    let backend = WasmBackend;

    // Create test API surface with 1 TypeDef (2 fields), 1 FunctionDef, 1 EnumDef (2 variants)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
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
            doc: "Test configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
            error_type: None,
            doc: "Process input".to_string(),
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
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
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
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
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
            doc: "Processing mode".to_string(),
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

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();

    // Should generate lib.rs + Cargo.toml
    assert_eq!(files.len(), 2, "Should generate lib.rs and Cargo.toml");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().ends_with("lib.rs"),
        "First file should be lib.rs"
    );

    let cargo_file = &files[1];
    assert!(
        cargo_file.path.to_string_lossy().ends_with("Cargo.toml"),
        "Second file should be Cargo.toml"
    );

    let content = &lib_file.content;

    // Assert content contains #[wasm_bindgen] markers
    assert!(
        content.contains("#[wasm_bindgen]"),
        "Content should contain #[wasm_bindgen] attribute"
    );

    // Assert struct generation with Wasm prefix
    assert!(
        content.contains("pub struct WasmConfig"),
        "Should generate Wasm-prefixed Config struct"
    );

    // Assert enum generation with Wasm prefix
    assert!(content.contains("pub enum WasmMode"), "Should generate WasmMode enum");

    // Assert function binding
    assert!(content.contains("pub fn process"), "Should generate process function");
}

#[test]
fn test_type_mapping() {
    let backend = WasmBackend;

    // Create test API with various type fields
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TypeTest".to_string(),
            rust_path: "test_lib::TypeTest".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_field", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_field", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_field", TypeRef::String, false),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("vec_string", TypeRef::Vec(Box::new(TypeRef::String)), false),
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
            doc: "Type mapping test".to_string(),
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

    let content = &files[0].content;

    // Should contain WasmTypeTest struct
    assert!(content.contains("pub struct WasmTypeTest"));

    // Should have #[wasm_bindgen] on struct
    assert!(content.contains("#[wasm_bindgen]"));

    // Should have fields for all types
    assert!(content.contains("u32_field"));
    assert!(content.contains("i64_field"));
    assert!(content.contains("string_field"));
    assert!(content.contains("opt_string"));
    assert!(content.contains("vec_string"));
}

#[test]
fn test_enum_generation() {
    let backend = WasmBackend;

    // Create test API with enum containing 3 variants
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Level".to_string(),
            rust_path: "test_lib::Level".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Low".to_string(),
                    fields: vec![],
                    doc: "Low level".to_string(),
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
                    name: "Medium".to_string(),
                    fields: vec![],
                    doc: "Medium level".to_string(),
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
                    name: "High".to_string(),
                    fields: vec![],
                    doc: "High level".to_string(),
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
            doc: "Severity levels".to_string(),
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

    let content = &files[0].content;

    // Should contain WasmLevel enum exported with its prefixed Rust name (no
    // js_name override) so the JS API matches alef-e2e imports.
    assert!(content.contains("#[wasm_bindgen]"));
    assert!(content.contains("pub enum WasmLevel"));
    assert!(!content.contains("js_name = \"Level\""));

    // Should have all variants
    assert!(content.contains("Low"));
    assert!(content.contains("Medium"));
    assert!(content.contains("High"));

    // Should have #[derive] for Copy
    assert!(content.contains("Copy"));
}

#[test]
fn test_generated_header() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Data".to_string(),
            rust_path: "test_lib::Data".to_string(),
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

    // lib.rs has generated_header: false because the builder embeds the header into the
    // content string itself (via RustFileBuilder::with_generated_header).
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("must have lib.rs");
    assert!(
        !lib_file.generated_header,
        "lib.rs should have generated_header: false (header is embedded in content)"
    );

    // Cargo.toml uses generated_header: true so that write_files always regenerates it.
    let cargo_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("must have Cargo.toml");
    assert!(
        cargo_file.generated_header,
        "Cargo.toml should have generated_header: true so it is always regenerated"
    );

    // lib.rs content should contain a generated header comment
    assert!(
        lib_file.content.contains("generated by alef") || lib_file.content.contains("DO NOT EDIT"),
        "lib.rs content should have a generated code marker"
    );
}

#[test]
fn test_async_function() {
    let backend = WasmBackend;

    // Create test API with an async function
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
            error_type: None,
            doc: "Fetch data from URL".to_string(),
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
    let content = &files[0].content;

    // Should contain async keyword in function
    assert!(content.contains("pub async fn fetch_data"));
    // Should contain .await call
    assert!(content.contains(".await"));
}

#[test]
fn test_async_function_with_error() {
    let backend = WasmBackend;

    // Create test API with an async function that returns Result
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "parse_json".to_string(),
            rust_path: "test_lib::parse_json".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "json".to_string(),
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
            error_type: Some("ParseError".to_string()),
            doc: "Parse JSON".to_string(),
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
    let content = &files[0].content;

    // Should contain async function
    assert!(content.contains("pub async fn parse_json"));
    // Should handle error with map_err
    assert!(content.contains("map_err"));
}

#[test]
fn test_methods_generation() {
    let backend = WasmBackend;

    // Create test API with a TypeDef that has methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Counter".to_string(),
            rust_path: "test_lib::Counter".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![
                MethodDef {
                    name: "increment".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
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
                MethodDef {
                    name: "get_value".to_string(),
                    params: vec![],
                    return_type: TypeRef::Primitive(PrimitiveType::U32),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Get current value".to_string(),
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
            doc: "A simple counter".to_string(),
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
    let content = &files[0].content;

    // Should generate Counter struct with Wasm prefix
    assert!(content.contains("pub struct WasmCounter"));

    // Should have methods
    assert!(content.contains("fn increment"));
    assert!(content.contains("fn get_value"));

    // Should have #[wasm_bindgen] on impl
    assert!(content.contains("#[wasm_bindgen]"));
}

#[test]
fn test_async_methods() {
    let backend = WasmBackend;

    // Create test API with an async method
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "AsyncWorker".to_string(),
            rust_path: "test_lib::AsyncWorker".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("name", TypeRef::String, false)],
            methods: vec![MethodDef {
                name: "process".to_string(),
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
                is_static: false,
                error_type: None,
                doc: "Process data asynchronously".to_string(),
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
            doc: "Async worker".to_string(),
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
    let content = &files[0].content;

    // Should contain async method
    assert!(content.contains("pub async fn process"));
    // Should have .await
    assert!(content.contains(".await"));
}

#[test]
fn test_error_types() {
    let backend = WasmBackend;

    // Create test API with error definitions
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ValidationError".to_string(),
            rust_path: "test_lib::ValidationError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "OutOfRange".to_string(),
                    fields: vec![],
                    doc: "Value out of range".to_string(),
                    message_template: Some("value out of range".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: "Validation errors".to_string(),
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

    assert!(result.is_ok());
    let files = result.unwrap();
    let content = &files[0].content;

    // Should have error converter function
    assert!(
        content.contains("ValidationError") || content.contains("fn"),
        "Should contain error handling"
    );
}

#[test]
fn test_opaque_type() {
    let backend = WasmBackend;

    // Create test API with an opaque type
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueHandle".to_string(),
            rust_path: "test_lib::OpaqueHandle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
            doc: "An opaque handle".to_string(),
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
    let content = &files[0].content;

    // Should generate opaque struct with Arc
    assert!(content.contains("Arc"));
    // Should have WasmOpaqueHandle struct
    assert!(content.contains("WasmOpaqueHandle"));
}

#[test]
fn test_opaque_type_configured_in_config() {
    let backend = WasmBackend;

    // Create test API with an opaque type that's also declared in config.opaque_types
    // (like `Language` in tree-sitter-language-pack)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "test_lib::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
            doc: "Opaque language type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![
            // Function that returns the opaque type
            FunctionDef {
                name: "get_language".to_string(),
                rust_path: "test_lib::get_language".to_string(),
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
                return_type: TypeRef::Named("Language".to_string()),
                is_async: false,
                error_type: None,
                doc: "Get language by name".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
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

    let mut config = make_config();
    // Declare Language as an opaque type (external to the binding layer)
    config
        .opaque_types
        .insert("Language".to_string(), "test_lib::Language".to_string());

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();
    let content = &files[0].content;

    // Regression check: Opaque wrapper struct MUST be emitted even though Language
    // is in config.opaque_types. This is needed so that get_language() can return WasmLanguage.
    assert!(
        content.contains("pub struct WasmLanguage"),
        "WasmLanguage struct must be emitted for opaque return type"
    );
    assert!(
        content.contains("pub(crate) inner: Arc"),
        "WasmLanguage must wrap Arc<core::Language>"
    );
    assert!(
        content.contains("pub fn get_language"),
        "get_language function must be present"
    );
    // The function should return WasmLanguage (or Result<WasmLanguage, ...>)
    assert!(
        content.contains("WasmLanguage"),
        "get_language should return or work with WasmLanguage"
    );
}

#[test]
fn test_opaque_type_filter_simple_newtype_not_excluded() {
    let backend = WasmBackend;

    // Regression test: Simple newtype opaque (no generic params) should NOT be excluded
    // from WASM binding generation, so wasm-bindgen can emit a wrapper struct.
    // This is the tree-sitter-language-pack::Language case.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "test_lib::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
            doc: "Simple opaque language type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "get_language".to_string(),
            rust_path: "test_lib::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: "Get default language".to_string(),
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

    let mut config = make_config();
    // Declare Language as an opaque type with NO generic params in the path
    config
        .opaque_types
        .insert("Language".to_string(), "test_lib::Language".to_string());

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed for simple newtype opaque");
    let files = result.unwrap();
    let content = &files[0].content;

    // Simple newtype opaques should have wrapper struct emitted
    assert!(
        content.contains("pub struct WasmLanguage"),
        "WasmLanguage wrapper struct must be emitted for simple newtype opaque (no generics in path)"
    );
}

#[test]
fn test_opaque_type_filter_generic_path_excluded() {
    let backend = WasmBackend;

    // Regression test: Opaque with generic params in path should be EXCLUDED
    // because wasm-bindgen cannot wrap generic types. This is the sample_crate
    // Arc<Mutex<dyn Trait>> case.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handler".to_string(),
            rust_path: "test_lib::Handler".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
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
            doc: "Generic opaque handler".to_string(),
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

    let mut config = make_config();
    // Declare Handler with generic params in the path (like Arc<Mutex<dyn Trait>>)
    config.opaque_types.insert(
        "Handler".to_string(),
        "std::sync::Arc<std::sync::Mutex<dyn test_lib::Handler>>".to_string(),
    );

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();
    let content = &files[0].content;

    // Generic-path opaques should NOT have wrapper struct (excluded)
    assert!(
        !content.contains("pub struct WasmHandler"),
        "WasmHandler wrapper struct must NOT be emitted for generic-path opaque (contains '<')"
    );
}

#[test]
fn test_exclude_functions() {
    let backend = WasmBackend;

    // Create test API with multiple functions
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "public_func".to_string(),
                rust_path: "test_lib::public_func".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Public function".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "hidden_func".to_string(),
                rust_path: "test_lib::hidden_func".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Hidden function".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
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

    // Create config with exclude_functions set
    let mut config = make_config();
    if let Some(wasm_cfg) = &mut config.wasm {
        wasm_cfg.exclude_functions = vec!["hidden_func".to_string()];
    }

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();
    let content = &files[0].content;

    // Should contain public_func
    assert!(content.contains("pub fn public_func"));

    // Should NOT contain hidden_func
    assert!(
        !content.contains("pub fn hidden_func"),
        "excluded function should not be in output"
    );
}

#[test]
fn test_exclude_types() {
    let backend = WasmBackend;

    // Create test API with multiple types
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "PublicType".to_string(),
                rust_path: "test_lib::PublicType".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("field", TypeRef::String, false)],
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
                doc: "Public type".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "HiddenType".to_string(),
                rust_path: "test_lib::HiddenType".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("secret", TypeRef::String, false)],
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
                doc: "Hidden type".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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

    // Create config with exclude_types set
    let mut config = make_config();
    if let Some(wasm_cfg) = &mut config.wasm {
        wasm_cfg.exclude_types = vec!["HiddenType".to_string()];
    }

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();
    let content = &files[0].content;

    // Should contain PublicType
    assert!(content.contains("pub struct WasmPublicType"));

    // Should NOT contain HiddenType
    assert!(
        !content.contains("WasmHiddenType"),
        "excluded type should not be in output"
    );
}

#[test]
fn test_exclude_fields_removes_wasm_struct_field() {
    let backend = WasmBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ServerConfig".to_string(),
            rust_path: "test_lib::ServerConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("host", TypeRef::String, false),
                make_field("enable_http_trace", TypeRef::Primitive(PrimitiveType::Bool), false),
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

    let mut config = make_config();
    if let Some(wasm_cfg) = &mut config.wasm {
        wasm_cfg
            .exclude_fields
            .insert("ServerConfig".to_string(), vec!["enable_http_trace".to_string()]);
    }

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(content.contains("pub struct WasmServerConfig"), "content: {content}");
    assert!(content.contains("host: String"), "content: {content}");
    assert!(
        !content.contains("enable_http_trace"),
        "excluded wasm field must be absent from struct and conversions: {content}"
    );
}

// ---------------------------------------------------------------------------
// WASM trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_def_wasm(name: &str, methods: Vec<MethodDef>) -> TypeDef {
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

fn make_method_wasm(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
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

fn make_async_method_wasm(name: &str, return_type: TypeRef) -> MethodDef {
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
        version: Default::default(),
    }
}

fn make_node_context_wasm() -> TypeDef {
    TypeDef {
        name: "SyntaxContext".to_string(),
        rust_path: "my_lib::SyntaxContext".to_string(),
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
        version: Default::default(),
    }
}

fn make_visit_result_wasm() -> EnumDef {
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

fn make_api_wasm() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![make_node_context_wasm()],
        functions: vec![],
        enums: vec![make_visit_result_wasm()],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_plugin_bridge_cfg_wasm(trait_name: &str) -> alef::core::config::TraitBridgeConfig {
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

fn make_visitor_bridge_cfg_wasm(trait_name: &str, type_alias: &str) -> alef::core::config::TraitBridgeConfig {
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
        context_type: Some("SyntaxContext".to_string()),
        result_type: Some("WalkDecision".to_string()),
    }
}

// ---------------------------------------------------------------------------
// WASM trait bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_wasm_visitor_bridge_produces_visitor_struct() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "SyntaxWalker",
        vec![make_method_wasm("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_wasm("SyntaxWalker", "SyntaxWalker");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("WasmSyntaxWalkerBridge"),
        "WASM visitor bridge struct must be named Wasm{{TraitName}}Bridge"
    );
    assert!(
        code.code
            .contains("impl my_lib::SyntaxWalker for WasmSyntaxWalkerBridge"),
        "WASM visitor bridge must implement the trait"
    );
}

#[test]
fn test_wasm_visitor_bridge_has_js_obj_field() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "SyntaxWalker",
        vec![make_method_wasm("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_wasm("SyntaxWalker", "SyntaxWalker");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("js_obj: wasm_bindgen::JsValue"),
        "WASM visitor bridge must store JsValue in a 'js_obj' field"
    );
}

#[test]
fn test_wasm_plugin_bridge_produces_wrapper_struct_with_inner_and_cached_name() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "TextBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("TextBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("pub struct WasmTextBackendBridge"),
        "WASM plugin bridge wrapper struct must be WasmTextBackendBridge"
    );
    assert!(
        code.code.contains("inner:"),
        "WASM plugin bridge wrapper must have an 'inner' field"
    );
    assert!(
        code.code.contains("cached_name: String"),
        "WASM plugin bridge wrapper must have a 'cached_name: String' field"
    );
}

#[test]
fn test_wasm_plugin_bridge_generates_super_trait_impl() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "TextBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("TextBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("impl my_lib::Plugin for WasmTextBackendBridge"),
        "WASM plugin bridge must implement Plugin super-trait"
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
fn test_wasm_plugin_bridge_generates_trait_impl_with_forwarded_methods() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "TextBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("TextBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("impl my_lib::TextBackend for WasmTextBackendBridge"),
        "WASM plugin bridge must implement the trait itself"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must forward the 'process' method"
    );
}

#[test]
fn test_wasm_plugin_bridge_generates_registration_fn_with_wasm_bindgen_attribute() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "TextBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("TextBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("#[wasm_bindgen"),
        "WASM registration function must carry the #[wasm_bindgen] attribute"
    );
    assert!(
        code.code.contains("pub fn register_textbackend("),
        "WASM registration function must use the configured name"
    );
}

#[test]
fn test_wasm_plugin_bridge_validates_required_methods() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "Analyzer",
        vec![
            make_method_wasm("analyze", TypeRef::String, true, false), // required
            make_method_wasm("describe", TypeRef::String, false, true), // optional
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
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // Registration fn must check for the required camelCase method "analyze"
    assert!(
        code.code.contains("\"analyze\""),
        "WASM registration fn must validate required method 'analyze'"
    );
}

#[test]
fn test_wasm_sync_method_body_uses_js_sys_reflect() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm("Scanner", vec![make_method_wasm("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Scanner");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    assert!(
        code.code.contains("js_sys::Reflect"),
        "WASM sync method body must use js_sys::Reflect to look up JS methods"
    );
}

#[test]
fn test_wasm_async_method_body_uses_box_pin() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm("Processor", vec![make_async_method_wasm("run", TypeRef::Unit)]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Processor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // Plugin bridges use #[async_trait] which wraps in Box::pin automatically, so the method body
    // should NOT contain Box::pin(async move { ... }) — that would cause double-boxing.
    // Instead, the body should directly contain the method implementation.
    assert!(
        code.code.contains("let key = wasm_bindgen::JsValue::from_str"),
        "WASM async method body must contain JS reflection code"
    );
    assert!(
        !code.code.contains("Box::pin(async move"),
        "WASM async method body with #[async_trait] must NOT use Box::pin — #[async_trait] already wraps it"
    );
}

/// When `generate_bindings` runs, it must always include a Cargo.toml with `js-sys` as a
/// dependency. The trait-bridge and visitor-bridge generated code references `js_sys::Object`,
/// `js_sys::Reflect`, and `js_sys::Function` via full paths, so the crate won't compile without
/// that dependency. Including the Cargo.toml in the `generate_bindings` output (rather than only
/// in the scaffold) ensures the dep is always present even for projects whose Cargo.toml
/// pre-dates the scaffold template change that added `js-sys`.
#[test]
fn test_generate_bindings_cargo_toml_includes_js_sys() {
    let backend = WasmBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
    let config = make_config();

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    // Should emit lib.rs + Cargo.toml
    assert_eq!(files.len(), 2, "generate_bindings must emit lib.rs and Cargo.toml");

    let cargo_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("generate_bindings must include a Cargo.toml");

    assert!(
        cargo_file.generated_header,
        "wasm Cargo.toml must have generated_header = true so it is always regenerated"
    );
    assert!(
        cargo_file.content.contains("js-sys"),
        "wasm Cargo.toml must include the js-sys dependency"
    );
    assert!(
        cargo_file.content.contains("wasm-bindgen"),
        "wasm Cargo.toml must include the wasm-bindgen dependency"
    );
    assert!(
        cargo_file.content.contains("serde-wasm-bindgen"),
        "wasm Cargo.toml must include the serde-wasm-bindgen dependency"
    );
    assert!(
        cargo_file
            .content
            .contains(r#"getrandom = { version = "0.4", features = ["wasm_js"] }"#),
        "wasm Cargo.toml must use the current getrandom wasm_js dependency"
    );
    assert!(
        cargo_file.content.contains("[lib]"),
        "wasm Cargo.toml must have a [lib] section with crate-type = [\"cdylib\"]"
    );
    assert!(
        cargo_file.content.contains("cdylib"),
        "wasm Cargo.toml must declare crate-type = [\"cdylib\"]"
    );
}

/// The js-sys dep must be present even when trait bridges ARE configured, since that is
/// precisely the code path that emits `js_sys::Reflect`, `js_sys::Object`, etc.
#[test]
fn test_generate_bindings_cargo_toml_js_sys_with_trait_bridge() {
    use alef::core::config::TraitBridgeConfig;
    use alef::core::ir::{MethodDef, ReceiverKind};

    let backend = WasmBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.5.0".to_string(),
        types: vec![TypeDef {
            name: "Visitor".to_string(),
            rust_path: "test_lib::Visitor".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "visit".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                is_static: false,
                receiver: Some(ReceiverKind::Ref),
                error_type: None,
                doc: String::new(),
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

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: Some("register_visitor".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with trait bridge should succeed");

    let cargo_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("generate_bindings must include a Cargo.toml");

    assert!(
        cargo_file.content.contains("js-sys"),
        "Cargo.toml must include js-sys when trait bridge code is generated that uses js_sys::Reflect"
    );
}

#[test]
fn test_vec_string_is_ref_serde_path_emits_refs_binding() {
    // Regression test: functions with Vec<String> is_ref=true params that also have an
    // error_type (putting them into the serde path) must emit a `{name}_refs` let binding.
    // Without the fix, gen_call_args_with_let_bindings emits `&names_refs` but no binding
    // was created, causing E0425: cannot find value `names_refs` in this scope.
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "download".to_string(),
            rust_path: "test_lib::download".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "names".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
            return_type: TypeRef::Primitive(PrimitiveType::Usize),
            is_async: false,
            error_type: Some("TestError".to_string()),
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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    assert!(
        lib_file.content.contains("let names_refs: Vec<&str>"),
        "generated lib.rs must create names_refs intermediate binding for Vec<String> is_ref=true;\n\
         actual content:\n{}",
        &lib_file.content[lib_file.content.find("fn download").unwrap_or(0)
            ..(lib_file.content.find("fn download").unwrap_or(0) + 300).min(lib_file.content.len())]
    );
    assert!(
        lib_file.content.contains("&names_refs"),
        "generated lib.rs must pass &names_refs to core function"
    );
}

/// Regression test: a non-opaque struct with `has_default: true` and a static `default()`
/// method returning `TypeRef::Named` with the same struct name must emit `.into()` so the
/// binding wrapper type (e.g. `WasmParseOptions`) is returned, not the bare core type.
///
/// Before the fix, `wrap_return_with_mutex` skipped `.into()` when `n == type_name`, which
/// caused `fn default() -> WasmParseOptions { core::ParseOptions::default() }` —
/// a type mismatch compile error.
#[test]
fn test_static_default_returns_binding_wrapper_not_core_type() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::options::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
            methods: vec![MethodDef {
                name: "default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("ParseOptions".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: String::new(),
                receiver: None,
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must call the core default() and convert with .into() so the
    // Wasm-prefixed binding wrapper is returned, not the bare inner core type.
    // The wasm backend builds the core call as `{core_import}::{type_name}::method()`.
    assert!(
        content.contains("::ParseOptions::default().into()"),
        "static default() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn default:\n{}",
        extract_fn_snippet(content, "fn default")
    );
}

/// Regression test: a static `from_update()` method on a non-opaque struct that takes a
/// `Named` param and returns `TypeRef::Named` with the same struct name must produce
/// a body ending in `.into()`, converting the core result to the binding wrapper.
#[test]
fn test_static_from_update_returns_binding_wrapper_not_core_type() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::options::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
                methods: vec![MethodDef {
                    name: "from_update".to_string(),
                    params: vec![ParamDef {
                        name: "update".to_string(),
                        ty: TypeRef::Named("ParseOptionsUpdate".to_string()),
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
                    return_type: TypeRef::Named("ParseOptions".to_string()),
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: String::new(),
                    receiver: None,
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
            },
            TypeDef {
                name: "ParseOptionsUpdate".to_string(),
                rust_path: "test_lib::ParseOptionsUpdate".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field(
                    "enabled",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                    true,
                )],
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The body must convert the core result with .into() so the binding wrapper is returned.
    assert!(
        content.contains("ParseOptions::from_update(update_core).into()"),
        "static from_update() must wrap core call with .into() to return binding wrapper;\n\
         actual content around fn from_update:\n{}",
        extract_fn_snippet(content, "fn from_update")
    );
}

/// `[wasm].core_crate_override` must redirect the Rust core dep to the named crate, and
/// `[wasm].exclude_extra_dependencies` must filter the listed keys out of the merged
/// `[crate.extra_dependencies]` set so the wasm binding can target a wasm-safe sub-crate
/// while siblings (e.g. http-only crates) are kept off the link line.
#[test]
fn test_wasm_core_crate_override_and_exclude_extra_dependencies() {
    let backend = WasmBackend;
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
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
    let mut config = make_config();
    config.name = "mylib".to_string();
    let mut crate_extras = std::collections::HashMap::new();
    crate_extras.insert("mylib-http".to_string(), toml::Value::String("1".to_string()));
    crate_extras.insert("mylib-graphql".to_string(), toml::Value::String("1".to_string()));
    config.extra_dependencies = crate_extras;
    let wasm = config.wasm.as_mut().expect("wasm config seeded");
    wasm.core_crate_override = Some("mylib-core".to_string());
    wasm.exclude_extra_dependencies = vec!["mylib-http".to_string(), "mylib-graphql".to_string()];

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let cargo_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("generate_bindings must include a Cargo.toml");
    let content = &cargo_file.content;

    assert!(
        content.contains(r#"mylib-core = { path = "../mylib-core""#),
        "wasm Cargo.toml must depend on the override crate via path = \"../mylib-core\";\nactual:\n{content}"
    );
    assert!(
        !content.contains(r#"mylib = { path = "../mylib""#),
        "wasm Cargo.toml must not also depend on the umbrella crate when override is set;\nactual:\n{content}"
    );
    assert!(
        !content.contains("mylib-http"),
        "wasm Cargo.toml must filter out excluded extra dep `mylib-http`;\nactual:\n{content}"
    );
    assert!(
        !content.contains("mylib-graphql"),
        "wasm Cargo.toml must filter out excluded extra dep `mylib-graphql`;\nactual:\n{content}"
    );
    // The published package name must remain `<crate.name>-wasm` regardless of override.
    assert!(
        content.contains(r#"name = "mylib-wasm""#),
        "wasm Cargo.toml package name must remain `mylib-wasm` when override is set;\nactual:\n{content}"
    );
}

/// Lock in the contract for `Map<String, NamedStruct>` fields in the WASM backend.
///
/// The WASM backend sets `map_uses_jsvalue = true`, which causes the entire
/// `HashMap<K, V>` to be serialised as a `JsValue`.  This is intentional:
/// wasm-bindgen cannot pass a Rust `HashMap` across the JS/Wasm boundary directly.
///
/// Core→binding direction uses `js_sys::JSON::parse` (via a `serde_json::to_string`
/// round-trip) to produce a plain JS object.  `serde_wasm_bindgen::to_value` is
/// intentionally NOT used here because it produces ES6 Maps for `serialize_map`
/// calls, whereas callers expect plain JS objects.
///
/// Binding→core direction uses `serde_wasm_bindgen::from_value`.
///
/// This test locks in that emission so a future refactor cannot silently switch to
/// a `.into_iter().map(|(k, v)| (k, v.into())).collect()` pattern, which would fail
/// to compile (the binding-wrapper type does not implement `Into<CoreType>` for map
/// values when the field is typed as `JsValue`).
#[test]
fn test_map_named_value_uses_serde_wasm_bindgen_not_into() {
    let backend = WasmBackend;

    // ChildStruct: a Named type whose values appear inside the Map.
    let child = TypeDef {
        name: "ChildStruct".to_string(),
        rust_path: "test_lib::ChildStruct".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("label", TypeRef::String, false)],
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
        doc: "A named child type used as map values.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    // ParentStruct: has Map<String, ChildStruct> and Option<Map<String, ChildStruct>> fields.
    let parent = TypeDef {
        name: "ParentStruct".to_string(),
        rust_path: "test_lib::ParentStruct".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field(
                "children",
                TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("ChildStruct".to_string())),
                ),
                false,
            ),
            make_field(
                "opt_children",
                TypeRef::Optional(Box::new(TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("ChildStruct".to_string())),
                ))),
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
        has_serde: true,
        super_traits: vec![],
        doc: "A struct with Map<String, NamedStruct> fields.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    // A function that accepts ParentStruct as a parameter forces the codegen to emit a
    // binding→core From impl for ParentStruct (only input types receive From<WasmX> for X).
    let process_fn = FunctionDef {
        name: "process_parent".to_string(),
        rust_path: "test_lib::process_parent".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "parent".to_string(),
            ty: TypeRef::Named("ParentStruct".to_string()),
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![child, parent],
        functions: vec![process_fn],
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
        .expect("generate_bindings should succeed for Map<String, Named> types");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");

    let content = &lib_file.content;

    // The generated From impl for ParentStruct → WasmParentStruct (core→binding) must use
    // js_sys::JSON::parse (via a serde_json::to_string round-trip) for Map fields.
    // serde_wasm_bindgen::to_value is intentionally NOT used here because it always produces
    // ES6 Maps for serialize_map calls, whereas js_sys::JSON::parse produces plain JS objects.
    assert!(
        content.contains("js_sys::JSON::parse"),
        "Map<String, Named> core→binding conversion must use js_sys::JSON::parse;\n\
         actual content around 'children':\n{}",
        extract_field_snippet(content, "children")
    );
    assert!(
        content.contains("serde_json::to_string"),
        "Map<String, Named> core→binding conversion must serialize via serde_json::to_string;\n\
         actual content around 'children':\n{}",
        extract_field_snippet(content, "children")
    );

    // The generated From impl for WasmParentStruct → ParentStruct (binding→core) must use
    // serde_wasm_bindgen::from_value for the Map fields.
    assert!(
        content.contains("serde_wasm_bindgen::from_value"),
        "Map<String, Named> binding→core conversion must use serde_wasm_bindgen::from_value;\n\
         actual content around 'children':\n{}",
        extract_field_snippet(content, "children")
    );

    // Confirm the Map field is typed as JsValue in the binding struct, not HashMap.
    // The WasmParentStruct struct definition must not reference HashMap.
    let wasm_struct_start = content.find("pub struct WasmParentStruct").unwrap_or(0);
    let wasm_struct_end = content[wasm_struct_start..]
        .find('}')
        .map(|i| wasm_struct_start + i + 1)
        .unwrap_or(content.len());
    let struct_body = &content[wasm_struct_start..wasm_struct_end];
    assert!(
        !struct_body.contains("HashMap"),
        "WasmParentStruct must use JsValue for Map fields, not HashMap;\nstruct body:\n{struct_body}"
    );

    // Must not use an iterator-based pattern (.into_iter().collect()) for Map fields, as that
    // would require the binding-wrapper type to implement Into<CoreType> which it does not for
    // types converted via serde_wasm_bindgen.
    assert!(
        !content.contains("into_iter().collect()"),
        "Map<String, Named> conversion must not use into_iter().collect() — \
         Named values inside a Map must be converted via serde_wasm_bindgen as whole JsValue, \
         not per-element .into();\nactual content snippet:\n{}",
        extract_field_snippet(content, "children")
    );
}

// ---------------------------------------------------------------------------
// WASM bridge bug fix tests
// ---------------------------------------------------------------------------

#[test]
fn test_wasm_bridge_constructor_reads_js_name_property() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "TextBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("TextBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // The constructor must read the JS object's "name" property using Reflect::get
    assert!(
        code.code.contains("Reflect::get"),
        "constructor must use Reflect::get to read JS name property"
    );
    assert!(
        code.code.contains("\"name\""),
        "constructor must read the 'name' property from JS object"
    );

    // The constructor must use the extracted name in shorthand field init
    assert!(
        code.code.contains("cached_name,"),
        "constructor must use cached_name in shorthand field init (not hardcoded string)"
    );

    // Should NOT contain the hardcoded string "wasm_bridge"
    assert!(
        !code.code.contains("cached_name: \"wasm_bridge\".to_string(),"),
        "constructor must NOT hardcode cached_name as \"wasm_bridge\""
    );
}

#[test]
fn test_wasm_bridge_bytes_param_generates_uint8array() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    // Create a method with a Bytes parameter (is_ref=true means &[u8])
    let method = MethodDef {
        name: "process".to_string(),
        params: vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
        return_type: TypeRef::String,
        is_async: false,
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
    };

    let trait_def = make_trait_def_wasm("Processor", vec![method]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Processor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // The generated code must convert bytes to Uint8Array
    assert!(
        code.code.contains("Uint8Array::from"),
        "Bytes param must be converted to Uint8Array"
    );

    // Must NOT use the old Debug format for bytes
    assert!(
        !code.code.contains("format!(\"{{:?}}\""),
        "Bytes param must NOT use Rust Debug format ({{:?}})"
    );
}

#[test]
fn test_wasm_bridge_async_method_awaits_promise() {
    use alef::backends::wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm("Processor", vec![make_async_method_wasm("run", TypeRef::String)]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Processor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api)
        .expect("trait bridge generation should succeed");

    // The generated async method must await the JS Promise using JsFuture
    assert!(
        code.code.contains("JsFuture::from"),
        "async method must await Promise using JsFuture::from"
    );

    // Must have .await keyword
    assert!(
        code.code.contains(".await"),
        "async method must have .await keyword for the Promise"
    );

    // Must check that result is a Promise (dyn_into::<js_sys::Promise>)
    assert!(
        code.code.contains("js_sys::Promise"),
        "async method must convert result to js_sys::Promise"
    );
}

/// Extract a ~300-char snippet around the first occurrence of `marker` for assertion messages.
fn extract_field_snippet<'a>(content: &'a str, marker: &str) -> &'a str {
    let start = content.find(marker).unwrap_or(0);
    let end = (start + 300).min(content.len());
    &content[start..end]
}

/// Extract a ~200-char snippet around the first occurrence of `marker` for assertion messages.
fn extract_fn_snippet<'a>(content: &'a str, marker: &str) -> &'a str {
    let start = content.find(marker).unwrap_or(0);
    let end = (start + 200).min(content.len());
    &content[start..end]
}

/// Synthetic `default()` factory must be emitted on every wasm struct with
/// fields, regardless of whether the wasm-bindgen `(constructor)` requires
/// arguments. Without this, JS callers who want an arg-free instance can only
/// invoke `new WasmFoo()` — which throws when any field is non-Optional in the
/// IR (e.g. `WasmChatCompletionTool { tool_type, function }`).
#[test]
fn test_default_factory_emitted_for_required_args_struct() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Tool".to_string(),
            rust_path: "test_lib::Tool".to_string(),
            original_rust_path: String::new(),
            // Required (non-Optional) fields force the constructor to take
            // positional args — exactly the case where `new WasmTool()` fails.
            fields: vec![
                make_field("kind", TypeRef::Named("ToolKind".to_string()), false),
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
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "ToolKind".to_string(),
            rust_path: "test_lib::ToolKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Function".to_string(),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");
    let content = &lib_file.content;

    // The constructor still takes the required positional args.
    assert!(
        content.contains("pub fn new("),
        "Should still emit wasm_bindgen constructor;\n\
         actual:\n{}",
        extract_fn_snippet(content, "pub fn new")
    );

    // The synthetic default() factory must also be emitted, returning the
    // binding wrapper via the derived Default impl.
    assert!(
        content.contains("pub fn default() -> WasmTool"),
        "Should emit synthetic default() factory on wasm wrapper;\n\
         actual:\n{}",
        extract_fn_snippet(content, "pub fn default")
    );
    assert!(
        content.contains("<WasmTool as ::core::default::Default>::default()"),
        "default() factory must delegate to the derived Default impl;\n\
         actual:\n{}",
        extract_fn_snippet(content, "pub fn default")
    );
}

/// When a struct already exposes an explicit static `default` method in its
/// IR (e.g. one carried over from the source crate), the synthetic factory
/// must NOT be emitted — duplicate `pub fn default` would fail to compile.
#[test]
fn test_default_factory_skipped_when_explicit_default_method_present() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false)],
            methods: vec![MethodDef {
                name: "default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("Options".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: String::new(),
                receiver: None,
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");
    let content = &lib_file.content;

    // The synthetic delegating factory MUST NOT appear (would conflict with
    // the explicit `default` method emitted via the methods loop).
    assert!(
        !content.contains("<WasmOptions as ::core::default::Default>::default()"),
        "Synthetic default() factory must be skipped when an explicit default method exists;\n\
         actual content:\n{}",
        content
    );
}

fn make_enum_def(name: &str, variants: &[&str], serde_rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|v| EnumVariant {
                name: v.to_string(),
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
            })
            .collect(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: serde_rename_all.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

fn make_type_def(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
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
    }
}

/// Optional enum fields must generate `Option<String>` getters (not `Option<WasmEnum>`)
/// so JS receives the serde wire string (e.g. "stop", "tool_calls") rather than a
/// numeric discriminant.
#[test]
fn test_optional_enum_getter_returns_option_string() {
    let backend = WasmBackend;

    let finish_reason_enum = make_enum_def("FinishReason", &["Stop", "ToolCalls", "Length"], Some("snake_case"));

    let choice_type = make_type_def(
        "StreamChoice",
        vec![make_field(
            "finish_reason",
            TypeRef::Named("FinishReason".to_string()),
            true, // optional
        )],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![choice_type],
        functions: vec![],
        enums: vec![finish_reason_enum],
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
        .expect("generate_bindings should succeed");
    let content = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    // Getter must return Option<String>, not Option<WasmFinishReason>
    assert!(
        content.contains("pub fn finish_reason(&self) -> Option<String>"),
        "optional enum getter must return Option<String>;\nactual content:\n{}",
        content
    );
    // Getter must call to_api_str()
    assert!(
        content.contains("self.finish_reason.map(|v| v.to_api_str().to_owned())"),
        "optional enum getter must use to_api_str().to_owned();\nactual content:\n{}",
        content
    );
    // Setter must still accept Option<WasmFinishReason> (unchanged)
    assert!(
        content.contains("fn set_finish_reason(&mut self, value: Option<WasmFinishReason>)"),
        "setter must still accept Option<WasmFinishReason>;\nactual content:\n{}",
        content
    );
    // The enum itself must emit to_api_str()
    assert!(
        content.contains("Self::Stop => \"stop\""),
        "enum must emit to_api_str with snake_case strings;\nactual content:\n{}",
        content
    );
    assert!(
        content.contains("Self::ToolCalls => \"tool_calls\""),
        "ToolCalls variant must map to \"tool_calls\";\nactual content:\n{}",
        content
    );
}

/// Optional Vec-of-struct fields must generate `Option<js_sys::Array>` getters so JS
/// can access prototype methods on each element (e.g. `choice.toolCalls[0].function.name`).
#[test]
fn test_optional_vec_of_struct_getter_returns_js_array() {
    let backend = WasmBackend;

    // ToolCall is a plain struct (not an enum)
    let tool_call_type = make_type_def("ToolCall", vec![make_field("id", TypeRef::String, false)]);

    let delta_type = make_type_def(
        "StreamDelta",
        vec![make_field(
            "tool_calls",
            TypeRef::Vec(Box::new(TypeRef::Named("ToolCall".to_string()))),
            true, // optional
        )],
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![tool_call_type, delta_type],
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings should succeed");
    let content = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    // Getter must return Option<js_sys::Array>
    assert!(
        content.contains("pub fn tool_calls(&self) -> Option<js_sys::Array>"),
        "optional Vec-of-struct getter must return Option<js_sys::Array>;\nactual content:\n{}",
        content
    );
    // Getter body must use js_sys::Array::new() and push items
    assert!(
        content.contains("js_sys::Array::new()"),
        "getter body must create js_sys::Array;\nactual content:\n{}",
        content
    );
    assert!(
        content.contains("arr.push(&JsValue::from(item.clone()))"),
        "getter body must push items via JsValue::from;\nactual content:\n{}",
        content
    );
    // Setter must still accept Option<Vec<WasmToolCall>> (unchanged)
    assert!(
        content.contains("fn set_tool_calls(&mut self, value: Option<Vec<WasmToolCall>>)"),
        "setter must still accept Option<Vec<WasmToolCall>>;\nactual content:\n{}",
        content
    );
}

/// Regression test: a `Vec<TaggedDataEnum>` struct field must be stored as `JsValue` so that
/// plain JS object literals (e.g. `{ role: "user", content: "..." }`) can be assigned in e2e
/// tests without wasm-bindgen throwing "array contains a value of the wrong type".
///
/// Before the fix `messages: Vec<WasmMessage>` was emitted; wasm-bindgen type-checks each
/// element and rejects plain objects, causing all non-streaming chat tests to fail in CI.
#[test]
fn test_vec_of_tagged_data_enum_field_uses_js_value() {
    let backend = WasmBackend;

    // Build a tagged-data enum (Message) — serde_tag + at least one variant with fields.
    let make_data_variant = |name: &str, tag: &str| EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(format!("{name}Msg")),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: Some(tag.to_string()),
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        is_tuple: true,
        doc: String::new(),
        is_default: false,
        serde_rename: Some(tag.to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    };
    let message_enum = EnumDef {
        name: "Message".to_string(),
        rust_path: "test_lib::Message".to_string(),
        original_rust_path: String::new(),
        variants: vec![make_data_variant("User", "user"), make_data_variant("System", "system")],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("role".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Build a request struct with a required Vec<Message> field.
    let request_type = make_type_def(
        "ChatRequest",
        vec![make_field(
            "messages",
            TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string()))),
            false, // required
        )],
    );

    // Add a function that accepts ChatRequest so binding→core From impl is generated.
    let chat_fn = FunctionDef {
        name: "chat".to_string(),
        rust_path: "test_lib::chat".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "request".to_string(),
            ty: TypeRef::Named("ChatRequest".to_string()),
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![request_type],
        functions: vec![chat_fn],
        enums: vec![message_enum],
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
        .expect("generate_bindings should succeed");
    let content = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    // The struct field must be JsValue, not Vec<WasmMessage>.
    assert!(
        content.contains("messages: JsValue"),
        "Vec<TaggedDataEnum> struct field must be stored as JsValue;\nactual content:\n{content}"
    );
    assert!(
        !content.contains("messages: Vec<WasmMessage>"),
        "Vec<TaggedDataEnum> must NOT be Vec<WasmMessage>;\nactual content:\n{content}"
    );

    // The getter must return JsValue.
    assert!(
        content.contains("pub fn messages(&self) -> JsValue"),
        "Vec<TaggedDataEnum> getter must return JsValue;\nactual content:\n{content}"
    );

    // The setter must accept JsValue.
    assert!(
        content.contains("fn set_messages(&mut self, value: JsValue)"),
        "Vec<TaggedDataEnum> setter must accept JsValue;\nactual content:\n{content}"
    );

    // The From<WasmChatRequest>→core binding→core conversion must use serde_wasm_bindgen.
    assert!(
        content.contains("serde_wasm_bindgen::from_value(val.messages.clone()).unwrap_or_default()"),
        "binding→core From impl must deserialize JsValue via serde_wasm_bindgen;\nactual content:\n{content}"
    );

    // The core→binding From impl must serialize via serde_wasm_bindgen.
    assert!(
        content.contains("serde_wasm_bindgen::to_value(&val.messages).unwrap_or(JsValue::NULL)"),
        "core→binding From impl must serialize Vec<Message> via serde_wasm_bindgen;\nactual content:\n{content}"
    );
}

/// Regression: `Option<TaggedDataEnum>` and bare `TaggedDataEnum` scalar fields must use
/// `Option<JsValue>` / `JsValue` storage (not `Option<WasmFoo>` / `WasmFoo`) so that plain JS
/// object literals can be assigned without constructing explicit wasm-bindgen class instances.
///
/// Before this fix the error was: `Error: expected instance of WasmResponseFormat`.
#[test]
fn test_option_and_bare_tagged_data_enum_fields_use_js_value() {
    let backend = WasmBackend;

    // Build a tagged-data enum: ResponseFormat with at least one data variant.
    let format_enum = EnumDef {
        name: "ResponseFormat".to_string(),
        rust_path: "test_lib::ResponseFormat".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("TextFormat".to_string()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: Some("text".to_string()),
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("text".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "JsonObject".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("json_object".to_string()),
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
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Build a struct with:
    //   - an optional tagged-data enum field:  response_format: Option<ResponseFormat>
    //     (IR shape: optional=true, ty=Named — the mapper wraps it in Option<>)
    //   - a bare (required) tagged-data enum field:  format: ResponseFormat
    //     (IR shape: optional=false, ty=Named)
    let request_type = make_type_def(
        "ChatRequest",
        vec![
            make_field(
                "response_format",
                TypeRef::Named("ResponseFormat".to_string()),
                true, // optional=true: the mapper adds Option<> wrapping
            ),
            make_field(
                "format",
                TypeRef::Named("ResponseFormat".to_string()),
                false, // required
            ),
        ],
    );

    // Add a function that accepts ChatRequest so binding→core From impl is generated.
    let chat_fn = FunctionDef {
        name: "chat".to_string(),
        rust_path: "test_lib::chat".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "request".to_string(),
            ty: TypeRef::Named("ChatRequest".to_string()),
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![request_type],
        functions: vec![chat_fn],
        enums: vec![format_enum],
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
        .expect("generate_bindings should succeed");
    let content = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    // --- Option<TaggedDataEnum> checks ---

    // The struct field must be Option<JsValue>, not Option<WasmResponseFormat>.
    assert!(
        content.contains("response_format: Option<JsValue>"),
        "Option<TaggedDataEnum> struct field must be stored as Option<JsValue>;\nactual:\n{content}"
    );
    assert!(
        !content.contains("response_format: Option<WasmResponseFormat>"),
        "Option<TaggedDataEnum> must NOT be Option<WasmResponseFormat>;\nactual:\n{content}"
    );

    // The getter must return Option<JsValue>.
    assert!(
        content.contains("pub fn response_format(&self) -> Option<JsValue>"),
        "Option<TaggedDataEnum> getter must return Option<JsValue>;\nactual:\n{content}"
    );

    // The setter must accept Option<JsValue>.
    assert!(
        content.contains("fn set_response_format(&mut self, value: Option<JsValue>)"),
        "Option<TaggedDataEnum> setter must accept Option<JsValue>;\nactual:\n{content}"
    );

    // The binding→core From impl must use serde_wasm_bindgen for the optional field.
    assert!(
        content.contains(
            "response_format: val.response_format.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
        ),
        "Option<TaggedDataEnum> binding→core From impl must use serde_wasm_bindgen;\nactual:\n{content}"
    );

    // The core→binding From impl must serialize via serde_wasm_bindgen.
    assert!(
        content.contains(
            "response_format: val.response_format.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())"
        ),
        "Option<TaggedDataEnum> core→binding From impl must use serde_wasm_bindgen;\nactual:\n{content}"
    );

    // --- Bare TaggedDataEnum (required) checks ---

    // The struct field must be JsValue, not WasmResponseFormat.
    assert!(
        content.contains("format: JsValue"),
        "bare TaggedDataEnum struct field must be stored as JsValue;\nactual:\n{content}"
    );
    assert!(
        !content.contains("format: WasmResponseFormat"),
        "bare TaggedDataEnum must NOT be WasmResponseFormat;\nactual:\n{content}"
    );

    // The getter must return JsValue.
    assert!(
        content.contains("pub fn format(&self) -> JsValue"),
        "bare TaggedDataEnum getter must return JsValue;\nactual:\n{content}"
    );

    // The setter must accept JsValue.
    assert!(
        content.contains("fn set_format(&mut self, value: JsValue)"),
        "bare TaggedDataEnum setter must accept JsValue;\nactual:\n{content}"
    );

    // The binding→core From impl must use serde_wasm_bindgen for the bare field.
    assert!(
        content.contains("format: serde_wasm_bindgen::from_value(val.format.clone()).unwrap_or_default()"),
        "bare TaggedDataEnum binding→core From impl must use serde_wasm_bindgen;\nactual:\n{content}"
    );

    // The core→binding From impl must serialize via serde_wasm_bindgen.
    assert!(
        content.contains("format: serde_wasm_bindgen::to_value(&val.format).unwrap_or(JsValue::NULL)"),
        "bare TaggedDataEnum core→binding From impl must use serde_wasm_bindgen;\nactual:\n{content}"
    );

    // --- Constructor parameter type checks ---

    // The constructor parameter for an optional tagged-data enum field must be Option<JsValue>,
    // not Option<WasmResponseFormat> — JS callers pass plain object literals, not Rust wrappers.
    assert!(
        content.contains("response_format: Option<JsValue>"),
        "Option<TaggedDataEnum> constructor param must be Option<JsValue>;\nactual:\n{content}"
    );
    assert!(
        !content.contains("response_format: Option<WasmResponseFormat>"),
        "Option<TaggedDataEnum> constructor param must NOT be Option<WasmResponseFormat>;\nactual:\n{content}"
    );

    // The constructor parameter for a required (bare) tagged-data enum field must be JsValue.
    // `format` is required (optional=false), so the constructor takes it as JsValue directly.
    // `response_format` is optional (optional=true), so it comes last as Option<JsValue>.
    assert!(
        content.contains("pub fn new(format: JsValue, responseFormat: Option<JsValue>)"),
        "bare TaggedDataEnum constructor param must be JsValue, optional must be Option<JsValue>;\nactual:\n{content}"
    );
}

#[test]
fn test_constructor_params_camel_case() {
    let backend = WasmBackend;

    // Create a test struct with snake_case fields: field_one, field_two, field_three.
    // The constructor parameters should be camelCase for JS consumers, while the struct
    // initialization must use explicit field syntax with the renamed parameters.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MyConfig".to_string(),
            rust_path: "test_lib::MyConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("field_one", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field("field_two", TypeRef::String, false),
                make_field("field_three", TypeRef::Primitive(PrimitiveType::U32), true),
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

    let content = &files[0].content;

    // The constructor must have camelCase parameter names (fieldOne, fieldTwo, fieldThree)
    // for JS consumers to see consistent naming in .d.ts hints and IDE autocomplete.
    assert!(
        content.contains("pub fn new(fieldOne: bool, fieldTwo: String, fieldThree: Option<u32>)"),
        "Constructor parameters must be camelCase for JS consumers; actual content:\n{content}"
    );

    // The struct initialization must use explicit field syntax mapping the camelCase
    // parameter names to the snake_case field names, e.g. "field_one: fieldOne".
    assert!(
        content.contains("WasmMyConfig { field_one: fieldOne, field_two: fieldTwo, field_three: fieldThree }"),
        "Struct literal must use explicit field syntax with renamed params; actual content:\n{content}"
    );
}

#[test]
fn test_wasm_js_name_on_non_opaque_struct() {
    let backend = WasmBackend;

    // Create test API with a non-opaque struct
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
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
            doc: "Test configuration".to_string(),
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

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();
    let content = &files[0].content;

    // The struct should be exported via wasm-bindgen using its prefixed Rust
    // name (`WasmConfig`) so the JS API matches what alef-e2e codegen imports.
    // No `js_name` override — the JS class name == the Rust struct name.
    assert!(
        content.contains("#[wasm_bindgen]\npub struct WasmConfig"),
        "Non-opaque struct should be exported as the prefixed Rust name (no js_name override); actual content:\n{content}"
    );
    assert!(
        !content.contains("js_name = \"Config\""),
        "Non-opaque struct must NOT strip the wasm prefix via js_name; actual content:\n{content}"
    );
}

#[test]
fn test_wasm_js_name_on_opaque_struct() {
    let backend = WasmBackend;

    // Create test API with an opaque struct
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Parser".to_string(),
            rust_path: "test_lib::Parser".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "parse".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                is_static: false,
                is_async: false,
                error_type: None,
                doc: "Parse source code".to_string(),
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
            doc: "Parser handle".to_string(),
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

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();
    let content = &files[0].content;

    // The opaque struct is exported via wasm-bindgen using its prefixed Rust
    // name (`WasmParser`) so the JS API matches what alef-e2e codegen imports.
    assert!(
        content.contains("#[wasm_bindgen]\npub struct WasmParser"),
        "Opaque struct should be exported as the prefixed Rust name (no js_name override); actual content:\n{content}"
    );
    assert!(
        !content.contains("js_name = \"Parser\""),
        "Opaque struct must NOT strip the wasm prefix via js_name; actual content:\n{content}"
    );
}

#[test]
fn test_wasm_js_name_on_unit_enum() {
    let backend = WasmBackend;

    // Create test API with a unit enum
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
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
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
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
            doc: "Processing mode".to_string(),
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
    let content = &files[0].content;

    // The enum is exported via wasm-bindgen using its prefixed Rust name
    // (`WasmMode`) so the JS API matches what alef-e2e codegen imports.
    assert!(
        content.contains("#[wasm_bindgen]\n#[derive(Clone, Copy, PartialEq, Eq)]\npub enum WasmMode"),
        "Unit enum should be exported as the prefixed Rust name (no js_name override); actual content:\n{content}"
    );
    assert!(
        !content.contains("js_name = \"Mode\""),
        "Unit enum must NOT strip the wasm prefix via js_name; actual content:\n{content}"
    );
}

#[test]
fn test_has_default_struct_delegates_wasm_default_to_core_default() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "CrawlConfig".to_string(),
                rust_path: "test_lib::CrawlConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("max_depth", TypeRef::Primitive(PrimitiveType::U32), true)],
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
                doc: "Crawl config".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "UrlExtractionConfig".to_string(),
                rust_path: "test_lib::UrlExtractionConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("crawl", TypeRef::Named("CrawlConfig".to_string()), false)],
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
                doc: "URL config".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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

    let files = backend
        .generate_bindings(&api, &make_config())
        .expect("generate_bindings failed");
    let content = &files[0].content;

    let struct_start = content
        .find("pub struct WasmUrlExtractionConfig")
        .expect("WasmUrlExtractionConfig struct must be emitted");
    let derive_start = content[..struct_start]
        .rfind("#[derive")
        .expect("WasmUrlExtractionConfig derive block must be emitted");
    let derive_window = &content[derive_start..struct_start];
    assert!(
        !derive_window.contains("Default"),
        "WasmUrlExtractionConfig must not derive field-level Default; content:\n{content}"
    );
    assert!(
        content.contains("impl Default for WasmUrlExtractionConfig"),
        "WasmUrlExtractionConfig must emit a delegating Default impl; content:\n{content}"
    );
    assert!(
        content.contains("<test_lib::UrlExtractionConfig as Default>::default().into()"),
        "Default impl must delegate to the core type; content:\n{content}"
    );
}

/// Regression test: constructor params and struct-literal field inits must stay in sync.
///
/// Three cases:
/// 1. Single-word optional field (`content: Option<String>`) — camelCase == snake_case, must work.
/// 2. Multi-word optional field (`total_tokens: Option<u64>`) — param becomes `totalTokens`,
///    struct-literal LHS stays `total_tokens`, RHS must be `totalTokens.unwrap_or_default()`.
///    Previously emitted `total_tokens.unwrap_or_default()` (E0425 — ident not in scope).
/// 3. Multi-word required field (`tool_call_id: String`) — param becomes `toolCallId`,
///    struct-literal must be `tool_call_id: toolCallId`.
#[test]
fn test_constructor_camel_case_param_sync_with_snake_case_field_init() {
    let backend = WasmBackend;

    // has_default=true triggers config_constructor_parts_with_options, which wraps all fields
    // as Option<T> with unwrap_or_default() in the assignments — the problematic path.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Usage".to_string(),
            rust_path: "test_lib::Usage".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field("total_tokens", TypeRef::Primitive(PrimitiveType::U64), false),
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
            doc: "Token usage counters".to_string(),
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
    assert!(result.is_ok(), "Generation must succeed: {:?}", result.err());
    let content = &result.unwrap()[0].content;

    // Case 2: multi-word field — RHS must be `totalTokens.unwrap_or_default()`.
    assert!(
        content.contains("total_tokens: totalTokens.unwrap_or_default()"),
        "multi-word optional field must emit 'total_tokens: totalTokens.unwrap_or_default()'; actual:\n{content}"
    );
    // The broken form must NOT appear.
    assert!(
        !content.contains("total_tokens: total_tokens.unwrap_or_default()"),
        "broken form 'total_tokens: total_tokens.unwrap_or_default()' must NOT appear; actual:\n{content}"
    );
}

/// Regression test: multi-word REQUIRED (non-has_default) field constructor.
///
/// `constructor_parts` emits the shorthand `tool_call_id` (no colon).
/// `convert_constructor_params_to_camel_case` must expand it to `tool_call_id: toolCallId`.
#[test]
fn test_constructor_camel_case_required_multi_word_field() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ToolCall".to_string(),
            rust_path: "test_lib::ToolCall".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("tool_call_id", TypeRef::String, false),
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
            doc: "A tool call with required fields".to_string(),
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
    assert!(result.is_ok(), "Generation must succeed: {:?}", result.err());
    let content = &result.unwrap()[0].content;

    // Case 3: multi-word required field — struct-literal must use explicit form.
    assert!(
        content.contains("tool_call_id: toolCallId"),
        "multi-word required field must emit 'tool_call_id: toolCallId'; actual:\n{content}"
    );
}

/// Regression test: From impl must use snake_case for struct field names on both sides.
///
/// The binding struct has snake_case field names (prompt_tokens, completion_tokens).
/// The From<WasmStruct> → CoreStruct impl must reference these with snake_case
/// (val.prompt_tokens, not val.promptTokens).
/// Previously, if the WASM backend applied camelCase conversions to param names,
/// it could leak into the From impl generation, causing E0425 "cannot find value".
#[test]
fn test_from_impl_uses_snake_case_field_names() {
    let backend = WasmBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "TokenCount".to_string(),
            rust_path: "test_lib::TokenCount".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("prompt_tokens", TypeRef::Primitive(PrimitiveType::U64), false),
                make_field("completion_tokens", TypeRef::Primitive(PrimitiveType::U64), false),
                make_field("chunks_processed", TypeRef::Primitive(PrimitiveType::Usize), false),
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
            doc: "Token counts with snake_case fields".to_string(),
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
    assert!(result.is_ok(), "Generation must succeed: {:?}", result.err());
    let content = &result.unwrap()[0].content;

    // The From<core::TokenCount> → WasmTokenCount impl must use snake_case on the RHS.
    // Pattern: `impl From<test_lib::TokenCount> for WasmTokenCount { fn from(val: ...) -> Self { Self { prompt_tokens: val.prompt_tokens, ... } } }`
    assert!(
        content.contains("impl From<test_lib::TokenCount> for WasmTokenCount"),
        "From impl header must exist"
    );
    assert!(
        content.contains("prompt_tokens: val.prompt_tokens"),
        "From impl must use 'prompt_tokens: val.prompt_tokens' (snake_case on both sides); actual:\n{content}"
    );
    assert!(
        content.contains("completion_tokens: val.completion_tokens"),
        "From impl must use 'completion_tokens: val.completion_tokens' (snake_case on both sides); actual:\n{content}"
    );
    // The camelCase form must NOT appear in the From impl.
    assert!(
        !content.contains("prompt_tokens: val.promptTokens"),
        "From impl must NOT use camelCase on RHS like 'prompt_tokens: val.promptTokens'; actual:\n{content}"
    );
    assert!(
        !content.contains("completion_tokens: val.completionTokens"),
        "From impl must NOT use camelCase on RHS like 'completion_tokens: val.completionTokens'; actual:\n{content}"
    );
}

/// A tagged-data enum variant whose field was originally `Vec<(String, String)>` but was
/// sanitized to `Vec<String>` (sanitized=true, original_type="Vec<(String, String)>") must:
///
/// - store the field as `Option<JsValue>` in the wasm binding struct (not `Option<Vec<String>>`)
/// - decode via `serde_wasm_bindgen::from_value::<Vec<(String, String)>>` in binding→core
/// - encode via `serde_wasm_bindgen::to_value` in core→binding
///
/// This preserves the `[["k","v"],...]` JSON wire format that serde produces for
/// `Vec<(String, String)>` and prevents the flat `["k","v",...]` that `Vec<String>` would give.
#[test]
fn test_sanitized_tuple_vec_field_uses_js_value_in_tagged_enum() {
    let backend = WasmBackend;

    let node_content_enum = EnumDef {
        name: "NodeContent".to_string(),
        rust_path: "test_lib::NodeContent".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                doc: String::new(),
                is_default: true,
                serde_rename: Some("text".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "MetadataBlock".to_string(),
                fields: vec![FieldDef {
                    name: "entries".to_string(),
                    // sanitized from Vec<(String, String)> → Vec<String>
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: true,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: Some("Vec<(String, String)>".to_string()),
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("metadata_block".to_string()),
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
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Add a function that accepts NodeContent so it is treated as an input type
    // and gen_tagged_enum_binding_to_core is generated.
    let visit_fn = FunctionDef {
        name: "visit_node".to_string(),
        rust_path: "test_lib::visit_node".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "content".to_string(),
            ty: TypeRef::Named("NodeContent".to_string()),
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
    };

    let api = ApiSurface {
        enums: vec![node_content_enum],
        functions: vec![visit_fn],
        ..Default::default()
    };
    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generate_bindings should not fail: {:?}", result.err());
    let files = result.unwrap();
    let lib_file = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs must be generated");
    let content = &lib_file.content;

    // The binding struct must store entries as JsValue, not Vec<String>.
    assert!(
        content.contains("entries: Option<JsValue>"),
        "sanitized Vec<(K,V)> field must be typed as Option<JsValue>;\nstruct body:\n{content}"
    );
    assert!(
        !content.contains("entries: Option<Vec<String>>"),
        "sanitized Vec<(K,V)> field must NOT be typed as Option<Vec<String>>;\nstruct body:\n{content}"
    );

    // Binding→core must decode via serde_wasm_bindgen.
    assert!(
        content.contains("serde_wasm_bindgen::from_value::<Vec<(String, String)>>"),
        "binding→core must decode entries via serde_wasm_bindgen::from_value::<Vec<(String, String)>>;\n{content}"
    );

    // Core→binding must encode via serde_wasm_bindgen.
    assert!(
        content.contains("serde_wasm_bindgen::to_value(&entries)"),
        "core→binding must encode entries via serde_wasm_bindgen::to_value;\n{content}"
    );
}

/// Regression: when a `clear_fn` is configured on a trait bridge, the function must appear
/// exactly once in the generated output.  Before the fix, the wasm backend emitted it as both
/// a top-level `#[wasm_bindgen]` export and inside the bridge module (which is glob-re-exported),
/// producing a duplicate-symbol compile error in wasm-bindgen.
#[test]
fn test_wasm_plugin_bridge_clear_fn_not_duplicated() {
    use alef::core::config::TraitBridgeConfig;

    let backend = WasmBackend;

    let trait_def = TypeDef {
        name: "TextBackend".to_string(),
        rust_path: "test_lib::TextBackend".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "run_ocr".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            receiver: Some(ReceiverKind::Ref),
            error_type: None,
            doc: String::new(),
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
    };

    // The clear_fn is a zero-parameter function in the public API surface.
    let clear_fn = FunctionDef {
        name: "clear_text_backends".to_string(),
        rust_path: "test_lib::clear_text_backends".to_string(),
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_def],
        functions: vec![clear_fn],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "TextBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("test_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_text_backend".to_string()),
        unregister_fn: Some("unregister_text_backend".to_string()),
        clear_fn: Some("clear_text_backends".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings with clear_fn bridge should succeed");

    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs");

    let content = &lib_file.content;

    // Not duplicated — exactly one wasm-bindgen export with this JS name.
    let occurrences = content.matches("clearTextBackends").count();
    assert_eq!(
        occurrences, 1,
        "clearTextBackends must appear exactly once in lib.rs (not duplicated by top-level + bridge glob re-export);\nfound {occurrences} occurrence(s)"
    );

    // Emitted via the bridge module, not silently dropped — the bridge module and its
    // glob re-export must both be present so callers can reach clearTextBackends.
    assert!(
        content.contains("mod __alef_wasm_bridge_textbackend"),
        "bridge module __alef_wasm_bridge_textbackend must be present in lib.rs"
    );
    assert!(
        content.contains("pub use __alef_wasm_bridge_textbackend::*"),
        "bridge module must be glob-re-exported so its symbols are reachable"
    );
}
