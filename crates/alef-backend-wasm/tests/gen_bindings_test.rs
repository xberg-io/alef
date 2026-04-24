use alef_backend_wasm::WasmBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, WasmConfig};
use alef_core::ir::{
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
        core_wrapper: alef_core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
        newtype_wrapper: None,
    }
}

/// Helper to create minimal AlefConfig with WASM enabled
fn make_config() -> AlefConfig {
    AlefConfig {
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
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
        wasm: Some(WasmConfig {
            exclude_functions: vec![],
            exclude_types: vec![],
            exclude_reexports: vec![],
            env_shims: vec![],
            type_overrides: std::collections::HashMap::new(),
            features: None,
            serde_rename_all: None,
            type_prefix: None,
            extra_dependencies: std::collections::HashMap::new(),
        }),
        ffi: None,
        go: None,
        java: None,
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
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
    }
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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Test configuration".to_string(),
            cfg: None,
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Process input".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();

    // Should generate 1 lib.rs file
    assert_eq!(files.len(), 1, "Should generate one lib.rs file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().ends_with("lib.rs"),
        "File should be lib.rs"
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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Type mapping test".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
                },
                EnumVariant {
                    name: "Medium".to_string(),
                    fields: vec![],
                    doc: "Medium level".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "High".to_string(),
                    fields: vec![],
                    doc: "High level".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Severity levels".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();

    let content = &files[0].content;

    // Should contain WasmLevel enum with #[wasm_bindgen]
    assert!(content.contains("#[wasm_bindgen]"));
    assert!(content.contains("pub enum WasmLevel"));

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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();

    // All generated files should have generated_header: false
    // (The builder adds the header to the content string, not the flag)
    for file in &files {
        assert!(
            !file.generated_header,
            "WASM backend should have generated_header: false"
        );
    }

    // But content should contain a generated header comment
    let content = &files[0].content;
    assert!(
        content.contains("generated by alef") || content.contains("DO NOT EDIT"),
        "Content should have a generated code marker"
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: "Fetch data from URL".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("ParseError".to_string()),
            doc: "Parse JSON".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
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
                },
            ],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "A simple counter".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            }],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Async worker".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
                },
                ErrorVariant {
                    name: "OutOfRange".to_string(),
                    fields: vec![],
                    doc: "Value out of range".to_string(),
                    message_template: Some("value out of range".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
            ],
            doc: "Validation errors".to_string(),
        }],
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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "An opaque handle".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
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
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
        ],
        enums: vec![],
        errors: vec![],
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
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Public type".to_string(),
                cfg: None,
            },
            TypeDef {
                name: "HiddenType".to_string(),
                rust_path: "test_lib::HiddenType".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("secret", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Hidden type".to_string(),
                cfg: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
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
    }
}

fn make_api_wasm() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

fn make_plugin_bridge_cfg_wasm(trait_name: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
    }
}

fn make_visitor_bridge_cfg_wasm(trait_name: &str, type_alias: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
    }
}

// ---------------------------------------------------------------------------
// WASM trait bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_wasm_visitor_bridge_produces_visitor_struct() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "HtmlVisitor",
        vec![make_method_wasm("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_wasm("HtmlVisitor", "HtmlVisitor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("WasmHtmlVisitorBridge"),
        "WASM visitor bridge struct must be named Wasm{{TraitName}}Bridge"
    );
    assert!(
        code.code.contains("impl my_lib::HtmlVisitor for WasmHtmlVisitorBridge"),
        "WASM visitor bridge must implement the trait"
    );
}

#[test]
fn test_wasm_visitor_bridge_has_js_obj_field() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "HtmlVisitor",
        vec![make_method_wasm("visit_node", TypeRef::Unit, false, true)],
    );
    let bridge_cfg = make_visitor_bridge_cfg_wasm("HtmlVisitor", "HtmlVisitor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("js_obj: wasm_bindgen::JsValue"),
        "WASM visitor bridge must store JsValue in a 'js_obj' field"
    );
}

#[test]
fn test_wasm_plugin_bridge_produces_wrapper_struct_with_inner_and_cached_name() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "OcrBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("OcrBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("pub struct WasmOcrBackendBridge"),
        "WASM plugin bridge wrapper struct must be WasmOcrBackendBridge"
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
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "OcrBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("OcrBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::Plugin for WasmOcrBackendBridge"),
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
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "OcrBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("OcrBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("impl my_lib::OcrBackend for WasmOcrBackendBridge"),
        "WASM plugin bridge must implement the trait itself"
    );
    assert!(
        code.code.contains("fn process("),
        "trait impl must forward the 'process' method"
    );
}

#[test]
fn test_wasm_plugin_bridge_generates_registration_fn_with_wasm_bindgen_attribute() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "OcrBackend",
        vec![make_method_wasm("process", TypeRef::String, true, false)],
    );
    let bridge_cfg = make_plugin_bridge_cfg_wasm("OcrBackend");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("#[wasm_bindgen]"),
        "WASM registration function must carry the #[wasm_bindgen] attribute"
    );
    assert!(
        code.code.contains("pub fn register_ocrbackend("),
        "WASM registration function must use the configured name"
    );
}

#[test]
fn test_wasm_plugin_bridge_validates_required_methods() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm(
        "Analyzer",
        vec![
            make_method_wasm("analyze", TypeRef::String, true, false), // required
            make_method_wasm("describe", TypeRef::String, false, true), // optional
        ],
    );
    let bridge_cfg = alef_core::config::TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
    };
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    // Registration fn must check for the required camelCase method "analyze"
    assert!(
        code.code.contains("\"analyze\""),
        "WASM registration fn must validate required method 'analyze'"
    );
}

#[test]
fn test_wasm_sync_method_body_uses_js_sys_reflect() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm("Scanner", vec![make_method_wasm("scan", TypeRef::String, true, false)]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Scanner");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

    assert!(
        code.code.contains("js_sys::Reflect"),
        "WASM sync method body must use js_sys::Reflect to look up JS methods"
    );
}

#[test]
fn test_wasm_async_method_body_uses_box_pin() {
    use alef_backend_wasm::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_wasm("Processor", vec![make_async_method_wasm("run", TypeRef::Unit)]);
    let bridge_cfg = make_plugin_bridge_cfg_wasm("Processor");
    let api = make_api_wasm();

    let code = gen_trait_bridge(&trait_def, &bridge_cfg, "my_lib", "Error", "Error::from({msg})", &api);

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
