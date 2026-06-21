use super::*;

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
                    version: Default::default(),
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
            doc: "Text processor".to_string(),
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
            version: Default::default(),
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
            version: Default::default(),
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
fn test_cfg_gated_async_function() {
    let backend = PhpBackend;

    // Create an async function with a cfg condition
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "embed_texts_async".to_string(),
            rust_path: "test_lib::embed_texts_async".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "texts".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
            return_type: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Primitive(
                alef::core::ir::PrimitiveType::F32,
            ))))),
            is_async: true,
            error_type: Some("EmbedError".to_string()),
            doc: "Embed texts asynchronously".to_string(),
            cfg: Some("all(feature = \"embeddings\", feature = \"tokio-runtime\")".to_string()),
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
            name: "EmbedError".to_string(),
            rust_path: "test_lib::EmbedError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotAvailable".to_string(),
                fields: vec![],
                doc: "Feature not available".to_string(),
                message_template: Some("embeddings not available".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            }],
            doc: "Embed error".to_string(),
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
    assert!(result.is_ok(), "Cfg-gated async function generation should succeed");

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    let content = &lib_rs.content;

    // Cfg-gated async functions should use an always-true cfg condition
    // so ext-php-rs's #[php_impl] macro can see them unconditionally.
    assert!(
        content.contains("#[cfg(any(all(feature = \"embeddings\", feature = \"tokio-runtime\"), not(all(feature = \"embeddings\", feature = \"tokio-runtime\"))))]"),
        "Should contain always-true cfg condition for ext-php-rs compatibility; content:\n{content}"
    );

    // The method should still be generated with the correct PHP name
    assert!(
        content.contains("#[php(name = \"embedTextsAsync\")]"),
        "Extension binding should expose the PHP method as camelCase `embedTextsAsync`; content:\n{content}"
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
            doc: "Opaque handle to resource".to_string(),
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
