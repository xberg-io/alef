use super::*;

#[test]
fn test_multiple_types_with_shared_error() {
    let backend = PhpBackend;

    let shared_error = ErrorDef {
        name: "SharedError".to_string(),
        rust_path: "test_lib::SharedError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "IoError".to_string(),
                error_code: None,
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
                error_code: None,
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
                    cfg: None,
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: "File reader".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
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
                    cfg: None,
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
                serde_container_default: false,
                serde_container_conversion: Default::default(),
                super_traits: vec![],
                doc: "Content parser".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![shared_error],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

    assert!(
        content.contains("Reader") && content.contains("Parser"),
        "Should contain both Reader and Parser types"
    );

    // Should contain #[php_class] for both
    let php_class_count = content.matches("#[php_class]").count();
    assert!(php_class_count >= 2, "Should have #[php_class] for both types");

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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();

    assert!(!files.is_empty(), "Should generate stubs file");
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    assert!(
        content.contains("class TestLibException extends \\RuntimeException"),
        "Exception should extend \\RuntimeException; content:\n{content}"
    );

    assert!(
        content.contains("class TestLibApi"),
        "Should generate TestLibApi class; content:\n{content}"
    );

    assert!(
        content.contains("createThing") || content.contains("create_thing"),
        "Should have createThing method in TestLibApi; content:\n{content}"
    );

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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).unwrap();

    assert!(!files.is_empty(), "Should generate public API file");
    let facade = files.first().unwrap();
    let content = &facade.content;

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
                cfg: None,
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

/// `gen_stub_return` (`gen_bindings/functions/stubs.rs`) used to fabricate a type-appropriate
/// default (`None` for `Optional`, `Vec::new()` for `Vec`) for a sanitized, non-fallible function
/// it cannot auto-delegate. That silently shipped fake data from a function that looks callable —
/// exactly the placeholder anti-pattern removed repo-wide; PHP's `has_error: false` branch now has
/// exactly one safe value (`()` for `TypeRef::Unit`) and `compile_error!`s for everything else, so
/// the generated crate fails to build instead of returning fabricated output at runtime. See the
/// `gen_stub_return_tests` unit tests next to `gen_stub_return` for the same contract in isolation. ~keep
#[test]
fn test_sanitized_function_generates_stub_not_direct_call() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    assert!(
        !content.contains("test_lib::extension_ambiguity("),
        "extension_ambiguity must not delegate to core (type mismatch); content:\n{content}"
    );
    assert!(
        !content.contains("test_lib::split_code("),
        "split_code must not delegate to core (type mismatch); content:\n{content}"
    );

    // ~keep: scope the fabricated-value guards to each stub body. The unconditional
    // ext-php-rs module-startup boilerplate contains `::std::option::Option::None` and
    // `None => 1,`, so a whole-file `contains("None")` check matches emitter boilerplate
    // that has nothing to do with stub returns.
    let stub_body = |func: &str| -> String {
        let Some((_, rest)) = content.split_once(&format!("pub fn {func}(")) else {
            panic!("{func} stub must be emitted; content:\n{content}");
        };
        let Some((body, _)) = rest.split_once("\n    }") else {
            panic!("{func} stub body must be brace-terminated; content:\n{content}");
        };
        body.to_string()
    };

    let ambiguity_body = stub_body("extension_ambiguity");
    assert!(
        !ambiguity_body.contains("None"),
        "extension_ambiguity (Option<String>, no Result) has no safe fabricated value and must not \
         silently return `None`; stub body:\n{ambiguity_body}"
    );
    let split_code_body = stub_body("split_code");
    assert!(
        !split_code_body.contains("Vec::new()"),
        "split_code (Vec<String>, no Result) has no safe fabricated value and must not silently \
         return `Vec::new()`; stub body:\n{split_code_body}"
    );
    assert!(
        content.contains("alef cannot generate PHP binding for extension_ambiguity;"),
        "extension_ambiguity (Option<String>, no Result) has no safe fabricated value, so alef must \
         fail the generated crate's build with compile_error! rather than ship fake data; content:\n{content}"
    );
    assert!(
        content.contains("alef cannot generate PHP binding for split_code;"),
        "split_code (Vec<String>, no Result) has no safe fabricated value, so alef must fail the \
         generated crate's build with compile_error! rather than ship fake data; content:\n{content}"
    );
    assert!(
        !content.contains("Err(ext_php_rs::exception::PhpException::default(\"Not implemented: extension_ambiguity"),
        "extension_ambiguity must not emit PhpException (no error_type); content:\n{content}"
    );
    assert!(
        !content.contains("Err(ext_php_rs::exception::PhpException::default(\"Not implemented: split_code"),
        "split_code must not emit PhpException (no error_type); content:\n{content}"
    );
}

#[test]
fn php_exclude_functions_omits_facade_method() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "hidden_function".to_string(),
                rust_path: "test_lib::hidden_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                error_type: None,
                doc: "Function excluded from the PHP facade.".to_string(),
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
                name: "visible_function".to_string(),
                rust_path: "test_lib::visible_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Function emitted into the PHP facade.".to_string(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_php_excludes();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let facade = files
        .iter()
        .find(|file| file.path.ends_with("TestLib.php"))
        .expect("public API should include TestLib.php");

    assert!(
        !facade.content.contains("hiddenFunction"),
        "excluded function must not appear in PHP facade; content:\n{}",
        facade.content
    );
    assert!(
        facade.content.contains("visibleFunction"),
        "non-excluded function must still appear in PHP facade; content:\n{}",
        facade.content
    );
}
