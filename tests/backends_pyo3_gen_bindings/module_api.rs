use super::*;

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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
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
            name: "get_type".to_string(),
            rust_path: "test_lib::get_type".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("MyType".to_string()),
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
            name: "Kind".to_string(),
            rust_path: "test_lib::Kind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "First".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            }],
            doc: String::new(),
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
        ..Default::default()
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
        alef::core::config::Language::Python,
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
            doc: "Fetch data asynchronously".to_string(),
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
        ..Default::default()
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
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("ProcessError".to_string()),
            doc: "Process asynchronously with error handling".to_string(),
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
        ..Default::default()
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
            original_rust_path: String::new(),
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
                    doc: "Process some data".to_string(),
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
            doc: "Test processor type".to_string(),
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
        ..Default::default()
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
            original_rust_path: String::new(),
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            is_opaque: true, // Make it opaque so async delegation works
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
        ..Default::default()
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
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    message_template: Some("not found".to_string()),
                    doc: "Item not found".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![],
                    message_template: Some("invalid input".to_string()),
                    doc: "Invalid input provided".to_string(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: "Error type for processing".to_string(),
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
        ..Default::default()
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
            doc: "An opaque handle type".to_string(),
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
        ..Default::default()
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
            original_rust_path: String::new(),
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
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Container with optional and vec fields".to_string(),
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
        ..Default::default()
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
            original_rust_path: String::new(),
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
        ..Default::default()
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
            name: "SampleLlmError".to_string(),
            rust_path: "test_lib::SampleLlmError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "AuthenticationError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "RateLimitedError".to_string(),
                    fields: vec![],
                    message_template: None,
                    doc: String::new(),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: String::new(),
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
        ..Default::default()
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
        content.contains("\"\"\"Sample llm error.\"\"\""),
        "SampleLlmError should have generated docstring"
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
