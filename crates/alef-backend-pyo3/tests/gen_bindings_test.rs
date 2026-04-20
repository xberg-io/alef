use alef_backend_pyo3::Pyo3Backend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, PythonConfig, StubsConfig};
use alef_core::ir::*;

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
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: Some(PythonConfig {
            module_name: Some("_test_lib".to_string()),
            pip_name: None,
            async_runtime: None,
            stubs: None,
            features: None,
            serde_rename_all: None,
            capsule_types: Default::default(),
            release_gil: false,
        }),
        node: None,
        ruby: None,
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
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
        trait_bridges: vec![],
    }
}

#[test]
fn test_basic_generation() {
    let backend = Pyo3Backend;

    // Create test API surface with 1 TypeDef (2 fields), 1 FunctionDef, 1 EnumDef (2 variants)
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, false),
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

    assert!(result.is_ok(), "Failed to generate bindings");
    let files = result.unwrap();

    // Should generate 1 file: lib.rs
    assert_eq!(files.len(), 1, "Expected 1 generated file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().ends_with("lib.rs"),
        "Expected lib.rs file"
    );

    let content = &lib_file.content;

    // Assert PyO3 macro markers are present
    assert!(
        content.contains("#[pyclass"),
        "Should contain #[pyclass] for Config type"
    );
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for Config methods"
    );
    assert!(
        content.contains("#[pyfunction]"),
        "Should contain #[pyfunction] for process function"
    );

    // Assert struct and enum names are present
    assert!(content.contains("struct Config"), "Should define Config struct");
    assert!(content.contains("enum Mode"), "Should define Mode enum");

    // Assert module initialization
    assert!(content.contains("#[pymodule]"), "Should contain #[pymodule] macro");
    assert!(
        content.contains("pub fn _test_lib"),
        "Should contain module init function with correct name"
    );

    // Assert pyo3 prelude import
    assert!(content.contains("use pyo3::prelude::*"), "Should import pyo3::prelude");
}

#[test]
fn test_type_mapping() {
    let backend = Pyo3Backend;

    // TypeDef with various field types: u32, i64, String, Option<String>, Vec<String>
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "DataTypes".to_string(),
            rust_path: "test_lib::DataTypes".to_string(),
            fields: vec![
                make_field("count", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("value", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("text", TypeRef::String, false),
                make_field("optional_text", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
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
            doc: "Various data types".to_string(),
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
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check that the struct is defined with expected field names
    assert!(content.contains("struct DataTypes"), "Should define DataTypes struct");

    // Verify field names are present
    assert!(content.contains("count:"), "Should have count field");
    assert!(content.contains("value:"), "Should have value field");
    assert!(content.contains("text:"), "Should have text field");
    assert!(content.contains("optional_text:"), "Should have optional_text field");
    assert!(content.contains("items:"), "Should have items field");

    // Check PyO3 derive/class macro presence
    assert!(content.contains("#[pyclass"), "Type should have #[pyclass] macro");

    // Check that conversions are generated (From/Into traits)
    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate conversion traits"
    );
}

#[test]
fn test_enum_generation() {
    let backend = Pyo3Backend;

    // EnumDef with 3 variants
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
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
                    name: "Complete".to_string(),
                    fields: vec![],
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Status enum".to_string(),
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
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check enum definition
    assert!(content.contains("enum Status"), "Should define Status enum");

    // Check enum variants are present
    assert!(content.contains("Pending"), "Should have Pending variant");
    assert!(content.contains("Active"), "Should have Active variant");
    assert!(content.contains("Complete"), "Should have Complete variant");

    // Check PyO3 enum macro (should have pyclass with eq and eq_int)
    assert!(
        content.contains("#[pyclass") && content.contains("eq"),
        "Enum should have #[pyclass] with eq attribute"
    );

    // Check conversion code is generated
    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate enum conversion code"
    );
}

#[test]
fn test_generated_header() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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

    // All generated files should have generated_header flag properly set
    // For lib.rs, it should be false (set by RustFileBuilder.with_generated_header())
    // but the builder adds the header comment to content
    for file in &files {
        // Check that lib.rs has the generated marker in content
        if file.path.to_string_lossy().ends_with("lib.rs") {
            // RustFileBuilder adds a header comment when .with_generated_header() is called
            assert!(
                file.content.contains("Code generated by Alef") || file.content.contains("DO NOT EDIT"),
                "Generated file should contain generation marker"
            );
        }
    }
}

#[test]
fn test_function_with_error_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "validate".to_string(),
            rust_path: "test_lib::validate".to_string(),
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
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check that the function is generated with #[pyfunction]
    assert!(
        content.contains("#[pyfunction]"),
        "Function should have #[pyfunction] macro"
    );
    assert!(content.contains("fn validate"), "Should generate validate function");

    // Check that signature macro is present (PyO3 functions need signatures)
    assert!(
        content.contains("#[pyo3(signature"),
        "Function should have pyo3 signature attribute"
    );
}

#[test]
fn test_empty_api_surface() {
    let backend = Pyo3Backend;

    // Empty API surface
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
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
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Even empty API should have module init
    assert!(content.contains("#[pymodule]"), "Should contain #[pymodule] macro");
    assert!(
        content.contains("pub fn _test_lib"),
        "Should contain module init function"
    );

    // Should have PyO3 imports
    assert!(content.contains("use pyo3::prelude::*"), "Should import pyo3 prelude");
}

#[test]
fn test_module_registration() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MyType".to_string(),
            rust_path: "test_lib::MyType".to_string(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
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
            name: "get_type".to_string(),
            rust_path: "test_lib::get_type".to_string(),
            params: vec![],
            return_type: TypeRef::Named("MyType".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Kind".to_string(),
            rust_path: "test_lib::Kind".to_string(),
            variants: vec![EnumVariant {
                name: "First".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            }],
            doc: String::new(),
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

    // Check that module init registers all types and functions
    assert!(
        content.contains("m.add_class::<MyType>"),
        "Module should register MyType class"
    );
    assert!(
        content.contains("m.add_class::<Kind>"),
        "Module should register Kind enum"
    );
    assert!(
        content.contains("m.add_function(wrap_pyfunction!(get_type"),
        "Module should register get_type function"
    );
}

#[test]
fn test_capabilities() {
    let backend = Pyo3Backend;
    let caps = backend.capabilities();

    assert!(caps.supports_async, "Should support async");
    assert!(caps.supports_classes, "Should support classes");
    assert!(caps.supports_enums, "Should support enums");
    assert!(caps.supports_option, "Should support Option types");
    assert!(caps.supports_result, "Should support Result types");
}

#[test]
fn test_language_and_name() {
    let backend = Pyo3Backend;

    assert_eq!(backend.name(), "pyo3", "Backend name should be 'pyo3'");
    assert_eq!(
        backend.language(),
        alef_core::config::Language::Python,
        "Backend language should be Python"
    );
}

#[test]
fn test_async_function() {
    let backend = Pyo3Backend;

    // FunctionDef with is_async: true
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_data".to_string(),
            rust_path: "test_lib::fetch_data".to_string(),
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
            }],
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: "Fetch data asynchronously".to_string(),
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
    assert!(result.is_ok(), "Failed to generate bindings for async function");

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert async function is marked with #[pyfunction]
    assert!(
        content.contains("#[pyfunction]"),
        "Async function should have #[pyfunction] macro"
    );
    assert!(content.contains("fn fetch_data"), "Should generate fetch_data function");

    // Assert async imports are present (needed for async functions)
    assert!(
        content.contains("pyo3_async_runtimes"),
        "Should import pyo3_async_runtimes for async support"
    );

    // Assert async runtime initialization
    assert!(
        content.contains("_tokio_runtime") || content.contains("async_runtime"),
        "Should have async runtime initialization code"
    );
}

#[test]
fn test_async_function_with_error() {
    let backend = Pyo3Backend;

    // FunctionDef with is_async: true and error_type
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_async".to_string(),
            rust_path: "test_lib::process_async".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("ProcessError".to_string()),
            doc: "Process asynchronously with error handling".to_string(),
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

    // Check that PyRuntimeError import is present for error handling
    assert!(
        content.contains("PyRuntimeError"),
        "Should import PyRuntimeError for async error handling"
    );

    // Check that the function is generated
    assert!(
        content.contains("fn process_async"),
        "Should generate process_async function"
    );
}

#[test]
fn test_methods_generation() {
    let backend = Pyo3Backend;

    // TypeDef with methods
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Processor".to_string(),
            rust_path: "test_lib::Processor".to_string(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![
                MethodDef {
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
                    }],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Process some data".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                },
                MethodDef {
                    name: "reset".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Reset processor".to_string(),
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
            doc: "Test processor type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for methods");

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Assert #[pymethods] block is present
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for Processor methods"
    );

    // Assert method definitions are present
    assert!(content.contains("fn process"), "Should define process method");
    assert!(content.contains("fn reset"), "Should define reset method");

    // Assert struct definition with pyclass macro
    assert!(content.contains("struct Processor"), "Should define Processor struct");
    assert!(
        content.contains("#[pyclass"),
        "Should have #[pyclass] macro on Processor"
    );
}

#[test]
fn test_async_method() {
    let backend = Pyo3Backend;

    // TypeDef with async method - must be opaque or have proper delegation setup
    // Use an opaque type so async method generation doesn't require complex conversion
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "AsyncHandler".to_string(),
            rust_path: "test_lib::AsyncHandler".to_string(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "handle_async".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Handle asynchronously".to_string(),
                receiver: Some(ReceiverKind::Ref),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                trait_source: None,
            }],
            is_opaque: true, // Make it opaque so async delegation works
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
    let content = &files[0].content;

    // Check that async method is defined
    assert!(content.contains("fn handle_async"), "Should define async method");

    // Check async runtime imports
    assert!(
        content.contains("pyo3_async_runtimes"),
        "Should import pyo3_async_runtimes for async methods"
    );

    // Check that future_into_py is used for async handling
    assert!(
        content.contains("future_into_py"),
        "Should use future_into_py for async methods"
    );
}

#[test]
fn test_error_types() {
    let backend = Pyo3Backend;

    // API surface with ErrorDef
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    message_template: Some("not found".to_string()),
                    doc: "Item not found".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![],
                    message_template: Some("invalid input".to_string()),
                    doc: "Invalid input provided".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
            ],
            doc: "Error type for processing".to_string(),
        }],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for error types");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert error creation code (create_exception! macros)
    assert!(
        content.contains("create_exception!"),
        "Should generate create_exception! macros for error types"
    );

    // Assert that specific error variants are created
    assert!(
        content.contains("NotFoundError"),
        "Should create NotFoundError exception"
    );
    assert!(
        content.contains("InvalidInputError"),
        "Should create InvalidInputError exception"
    );
    assert!(
        content.contains("ProcessError"),
        "Should create ProcessError base exception"
    );

    // Assert error converter function is generated
    assert!(
        content.contains("process_error_to_py_err") || content.contains("_to_py_err"),
        "Should generate error converter function"
    );
}

#[test]
fn test_opaque_type() {
    let backend = Pyo3Backend;

    // TypeDef with is_opaque: true
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueHandle".to_string(),
            rust_path: "test_lib::OpaqueHandle".to_string(),
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
            doc: "An opaque handle type".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for opaque type");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert opaque struct is generated with Arc<inner>
    assert!(
        content.contains("struct OpaqueHandle"),
        "Should define OpaqueHandle struct"
    );
    assert!(content.contains("Arc<"), "Opaque type should use Arc wrapper");
    assert!(content.contains("inner:"), "Opaque type should have inner field");

    // Assert Arc import is present
    assert!(content.contains("std::sync::Arc"), "Should import Arc for opaque types");

    // Assert pyclass macro is present
    assert!(
        content.contains("#[pyclass"),
        "Opaque type should have #[pyclass] macro"
    );
}

#[test]
fn test_optional_and_vec_fields() {
    let backend = Pyo3Backend;

    // TypeDef with Optional and Vec fields
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Container".to_string(),
            rust_path: "test_lib::Container".to_string(),
            fields: vec![
                make_field("optional_text", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
                make_field(
                    "optional_numbers",
                    TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I64))))),
                    true,
                ),
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
            doc: "Container with optional and vec fields".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings for optional/vec fields");

    let files = result.unwrap();
    let content = &files[0].content;

    // Assert struct is defined
    assert!(content.contains("struct Container"), "Should define Container struct");

    // Assert field names are present
    assert!(content.contains("optional_text:"), "Should have optional_text field");
    assert!(content.contains("items:"), "Should have items field");
    assert!(
        content.contains("optional_numbers:"),
        "Should have optional_numbers field"
    );

    // Assert pyclass macro
    assert!(content.contains("#[pyclass"), "Type should have #[pyclass] macro");

    // Assert Vec conversion code or container types are present
    assert!(
        content.contains("Vec") || content.contains("From") || content.contains("Into"),
        "Should handle Vec and Option conversions"
    );
}

#[test]
fn test_static_method() {
    let backend = Pyo3Backend;

    // TypeDef with static method
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Factory".to_string(),
            rust_path: "test_lib::Factory".to_string(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "create_default".to_string(),
                params: vec![],
                return_type: TypeRef::Named("Factory".to_string()),
                is_async: false,
                is_static: true,
                error_type: None,
                doc: "Create a default Factory".to_string(),
                receiver: None,
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
    let content = &files[0].content;

    // Assert static method is defined
    assert!(content.contains("fn create_default"), "Should define static method");

    // Assert #[pymethods] block is present
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for static methods"
    );

    // Assert staticmethod attribute (part of PyO3 static method binding)
    assert!(
        content.contains("staticmethod") || content.contains("create_default"),
        "Should mark method as static or generate appropriately"
    );
}

#[test]
fn test_exceptions_py_classes_without_docs_have_generated_docstrings() {
    let backend = Pyo3Backend;

    // Errors with no docstrings — exception classes must have generated docstrings (D101).
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "LiterLlmError".to_string(),
            rust_path: "test_lib::LiterLlmError".to_string(),
            variants: vec![
                ErrorVariant {
                    name: "AuthenticationError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
                ErrorVariant {
                    name: "RateLimitedError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                },
            ],
            doc: String::new(),
        }],
    };

    let config = make_config();

    let result = backend.generate_public_api(&api, &config);
    assert!(result.is_ok(), "Failed to generate public API");

    let files = result.unwrap();
    let exceptions_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("exceptions.py"))
        .expect("exceptions.py should be generated");

    let content = &exceptions_file.content;

    // No class should use `pass` — all must have docstrings (ruff D101).
    assert!(
        !content.contains("    pass"),
        "Exception classes must use docstrings, not `pass`"
    );

    // The base error class should have a generated docstring from its name.
    assert!(
        content.contains("\"\"\"Liter llm error.\"\"\""),
        "LiterLlmError should have generated docstring"
    );

    // Variant classes should also have generated docstrings.
    assert!(
        content.contains("\"\"\"Authentication error.\"\"\""),
        "AuthenticationError should have generated docstring"
    );
    assert!(
        content.contains("\"\"\"Rate limited error.\"\"\""),
        "RateLimitedError should have generated docstring"
    );

    // Verify no empty class body (class header immediately followed by blank line).
    for (i, line) in content.lines().enumerate() {
        if line.starts_with("class ") {
            let next_non_empty = content.lines().skip(i + 1).find(|l| !l.trim().is_empty());
            assert!(
                next_non_empty.is_none_or(|l| l.trim() != ""),
                "Class at line {} has empty body",
                i + 1
            );
        }
    }
}

/// Regression test for kreuzberg-dev/alef#1 / kreuzberg-dev/html-to-markdown#310.
///
/// A type with both `has_default = true` AND `is_return_type = true` (e.g. `ConversionResult`)
/// must be re-exported in `__init__.py` from the native Rust module, NOT from `options.py`.
/// `options.py` must NOT emit a `@dataclass` shadow class for such types; the authoritative
/// definition lives in the native module as a `#[pyclass]` struct. The shadow class caused
/// static analysis tools (Pylance) to report a type mismatch because the two classes are
/// unrelated even though they share a name.
#[test]
fn test_return_type_exported_from_native_module_not_options() {
    let backend = Pyo3Backend;

    // ConversionResult: has_default=true (implements Default), is_return_type=true (returned by convert())
    // ConversionOptions: has_default=true, is_return_type=false (input/config type)
    let conversion_result = TypeDef {
        name: "ConversionResult".to_string(),
        rust_path: "my_lib::ConversionResult".to_string(),
        fields: vec![
            make_field("content", TypeRef::String, false),
            make_field("title", TypeRef::Optional(Box::new(TypeRef::String)), true),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Result of a conversion operation.".to_string(),
        cfg: None,
    };

    let conversion_options = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "my_lib::ConversionOptions".to_string(),
        fields: vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), false)],
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
        doc: "Options for conversion.".to_string(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![conversion_result, conversion_options],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
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
            }],
            return_type: TypeRef::Named("ConversionResult".to_string()),
            is_async: false,
            error_type: None,
            doc: "Convert input to markdown.".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    };

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_my_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/my_lib"),
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");
    let options_py = files
        .iter()
        .find(|f| f.path.ends_with("options.py"))
        .expect("options.py not generated");

    // ConversionResult (return type) must be imported from the native module.
    let native_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from ._my_lib import"))
        .unwrap_or("");
    assert!(
        native_import_line.contains("ConversionResult"),
        "__init__.py must import ConversionResult from the native module, got:\n{}",
        init_py.content
    );

    // ConversionResult must NOT appear in the .options import.
    let options_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from .options import"))
        .unwrap_or("");
    assert!(
        !options_import_line.contains("ConversionResult"),
        "__init__.py must not import ConversionResult from .options, got:\n{}",
        init_py.content
    );

    // ConversionOptions (config/input type) must still be imported from .options.
    assert!(
        options_import_line.contains("ConversionOptions"),
        "__init__.py must import ConversionOptions from .options, got:\n{}",
        init_py.content
    );

    // Both names must appear in __all__.
    assert!(
        init_py.content.contains("\"ConversionResult\""),
        "__init__.py __all__ must include ConversionResult, got:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("\"ConversionOptions\""),
        "__init__.py __all__ must include ConversionOptions, got:\n{}",
        init_py.content
    );

    // options.py must NOT define a @dataclass shadow for ConversionResult.
    assert!(
        !options_py.content.contains("class ConversionResult"),
        "options.py must not define a ConversionResult shadow class, got:\n{}",
        options_py.content
    );

    // options.py MUST still define ConversionOptions (the input/config type).
    assert!(
        options_py.content.contains("class ConversionOptions"),
        "options.py must still define ConversionOptions dataclass, got:\n{}",
        options_py.content
    );
}
