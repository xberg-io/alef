use super::*;

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
        version: Default::default(),
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
                doc: "File reader".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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
                doc: "Content parser".to_string(),
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
        errors: vec![shared_error],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
            version: Default::default(),
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
                version: Default::default(),
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
                version: Default::default(),
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
        ..Default::default()
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
