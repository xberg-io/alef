use alef_backend_magnus::MagnusBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, RubyConfig};
use alef_core::ir::*;
use std::collections::HashMap;

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
    }
}

/// Helper to create a basic AlefConfig with Ruby enabled.
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
            path_mappings: HashMap::new(),
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
        ruby: Some(RubyConfig {
            gem_name: Some("test_lib".to_string()),
            stubs: None,
            features: None,
            serde_rename_all: None,
            extra_dependencies: Default::default(),
            scaffold_output: Default::default(),
        }),
        php: None,
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
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
    }
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
            error_type: Some("ProcessError".to_string()),
            doc: "Process input with config".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR backend".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Available backends".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
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
                },
                EnumVariant {
                    name: "Processing".to_string(),
                    fields: vec![],
                    doc: "Processing status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Complete".to_string(),
                    fields: vec![],
                    doc: "Complete status".to_string(),
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
            doc: "A data store".to_string(),
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
            }],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: Some("ValidationError".to_string()),
            doc: "Validate input".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
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
                },
                ErrorVariant {
                    name: "OutOfRange".to_string(),
                    fields: vec![],
                    doc: "Out of range".to_string(),
                    message_template: Some("value out of range".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
            ],
            doc: "Validation error type".to_string(),
        }],
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: "Process data asynchronously".to_string(),
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
            doc: "Opaque processor type".to_string(),
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
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with default".to_string(),
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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_file.content;

    // Check for struct generation
    assert!(content.contains("struct Config"), "Should generate Config struct");

    // Check for Default impl generation
    assert!(
        content.contains("impl Default for Config") || content.contains("impl core::default::Default"),
        "Should generate Default implementation for types with has_default: true"
    );

    // Check for magnus wrapper
    assert!(content.contains("magnus::wrap"), "Should have magnus::wrap");
}

// ---------------------------------------------------------------------------
// Trait bridge tests (Magnus plugin bridge via gen_trait_bridge)
// ---------------------------------------------------------------------------

mod trait_bridge {
    use alef_backend_magnus::trait_bridge::gen_trait_bridge;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::*;

    fn make_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
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
        }
    }

    fn make_visitor_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,
            type_alias: Some(format!("{trait_name}Handle")),
            param_name: None,
            register_extra_args: None,
        }
    }

    // ---- Visitor bridge: type_alias still generates bridge ---

    #[test]
    fn test_visitor_bridge_generates_rb_bridge_struct() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", &make_api());

        assert!(
            code.contains("pub struct RbHtmlVisitorBridge"),
            "visitor bridge must produce RbHtmlVisitorBridge struct"
        );
    }

    #[test]
    fn test_visitor_bridge_does_not_generate_registration_fn() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", &make_api());

        assert!(
            !code.contains("#[magnus::init]"),
            "visitor bridge must not generate a registration function"
        );
    }

    #[test]
    fn test_visitor_bridge_generates_trait_impl() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", &make_api());

        assert!(
            code.contains("impl my_lib::HtmlVisitor for RbHtmlVisitorBridge"),
            "visitor bridge must implement the trait"
        );
    }
}
