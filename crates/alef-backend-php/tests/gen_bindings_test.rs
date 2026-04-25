use alef_backend_php::PhpBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, PhpConfig};
use alef_core::ir::*;

/// Helper to create a config with a specific extension name for namespace testing.
#[allow(dead_code)]
fn make_config_with_extension(extension_name: &str) -> AlefConfig {
    AlefConfig {
        alef: Default::default(),
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
        php: Some(PhpConfig {
            extension_name: Some(extension_name.to_string()),
            feature_gate: None,
            stubs: None,
            features: None,
            serde_rename_all: None,
            exclude_functions: vec![],
            exclude_types: vec![],
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
            rename_fields: Default::default(),
        }),
        elixir: None,
        wasm: None,
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
    }
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
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        alef: Default::default(),
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
        php: Some(PhpConfig {
            extension_name: Some("test_lib".to_string()),
            feature_gate: None,
            stubs: None,
            features: None,
            serde_rename_all: None,
            exclude_functions: vec![],
            exclude_types: vec![],
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
            rename_fields: Default::default(),
        }),
        elixir: None,
        wasm: None,
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
    }
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
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Extraction configuration".to_string(),
            cfg: None,
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
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text from file".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR backend".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Available OCR backends".to_string(),
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
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: "Inactive status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing status".to_string(),
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
            doc: "Text processor".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input: {reason}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                },
            ],
            doc: "Errors during processing".to_string(),
        }],
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("FetchError".to_string()),
            doc: "Fetch data asynchronously".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
            }],
            doc: "Fetch error".to_string(),
        }],
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
            }],
            is_opaque: true,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Opaque handle to resource".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
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
            },
            ErrorVariant {
                name: "ParseError".to_string(),
                fields: vec![],
                doc: "Parse error".to_string(),
                message_template: Some("Parse failed".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
            },
        ],
        doc: "Shared error type".to_string(),
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
                doc: "File reader".to_string(),
                cfg: None,
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
                doc: "Content parser".to_string(),
                cfg: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![shared_error],
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
            }],
            return_type: TypeRef::Named("Config".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Do some work".to_string(),
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
                }],
                return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
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
                }],
                return_type: TypeRef::Vec(Box::new(TypeRef::String)),
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: true,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            },
        ],
        enums: vec![],
        errors: vec![],
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

    // The generated bodies should emit PHP error stubs for sanitized functions.
    assert!(
        content.contains("Not implemented: extension_ambiguity") || content.contains("Not implemented: split_code"),
        "sanitized functions should emit PhpException error stubs; content:\n{content}"
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
    }
}

fn make_api_php() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

fn make_plugin_bridge_cfg_php(trait_name: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    }
}

fn make_visitor_bridge_cfg_php(trait_name: &str, type_alias: &str) -> alef_core::config::TraitBridgeConfig {
    alef_core::config::TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some(type_alias.to_string()),
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// PHP trait bridge tests
// ---------------------------------------------------------------------------

#[test]
fn test_php_visitor_bridge_produces_visitor_struct() {
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
fn test_php_plugin_bridge_validates_required_methods() {
    use alef_backend_php::trait_bridge::gen_trait_bridge;

    let trait_def = make_trait_def_php(
        "Analyzer",
        vec![
            make_method_php("analyze", TypeRef::String, true, false), // required
            make_method_php("describe", TypeRef::String, false, true), // optional
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
        exclude_languages: Vec::new(),
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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
    use alef_backend_php::trait_bridge::gen_trait_bridge;

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
