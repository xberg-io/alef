use super::*;

#[test]
fn test_stubs_non_void_methods_have_return_statements() {
    // PHPStan at level 9 rejects non-void methods with empty `{ }` bodies.
    // All stub methods with non-void return types must emit a body that
    // satisfies the static analyser — `throw new \RuntimeException(...)`.
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("label", TypeRef::String, true),
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
        functions: vec![FunctionDef {
            name: "create_config".to_string(),
            rust_path: "test_lib::create_config".to_string(),
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
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // getErrorCode(): int must NOT be bare `{ }` — PHPStan rejects it.
    assert!(
        !content.contains("getErrorCode(): int { }"),
        "getErrorCode stub must not have an empty body `{{ }}`; content:\n{content}"
    );

    // Non-void getter stubs must contain a throw or return, not bare `{ }`.
    // The pattern `): int { }` or `): string { }` or `): ?string { }` are all wrong.
    assert!(
        !content.contains("): int { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );
    assert!(
        !content.contains("): string { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );
    assert!(
        !content.contains("): ?string { }"),
        "no non-void stub method may have an empty body `{{ }}`; content:\n{content}"
    );

    // The static method stub in the Api class must also not be empty.
    assert!(
        !content.contains("): \\Test\\Lib\\Config { }"),
        "Api class stub method must not have an empty body; content:\n{content}"
    );

    // Stubs should use throw to satisfy PHPStan.
    assert!(
        content.contains("throw new \\RuntimeException"),
        "stub bodies must throw \\RuntimeException to satisfy PHPStan level 9; content:\n{content}"
    );
}

#[test]
fn test_static_stubs_promote_parameters_after_first_optional() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "submit_form".to_string(),
            rust_path: "test_lib::submit_form".to_string(),
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
            return_type: TypeRef::String,
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
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let content = &files.first().unwrap().content;

    assert!(
        content.contains("submitForm(string $path, ?string $json = null, ?string $multipart = null): string"),
        "static stub should keep PHP syntax valid when a required Rust param follows an optional one; content:\n{content}"
    );
}

#[test]
fn test_vec_named_struct_parameter() {
    let backend = PhpBackend;

    // Create test API with Vec<Item> parameter
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Item".to_string(),
            rust_path: "test_lib::Item".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("id", TypeRef::Primitive(PrimitiveType::U32), false),
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
        functions: vec![FunctionDef {
            name: "batch_process".to_string(),
            rust_path: "test_lib::batch_process".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
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
            doc: "Batch process items".to_string(),
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

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);
    assert!(
        result.is_ok(),
        "Generation should succeed for Vec<NamedStruct> parameter"
    );

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();

    // Should contain the Item type definition
    assert!(
        lib_rs.content.contains("pub struct Item"),
        "Should contain Item struct definition"
    );

    // Should contain the batch_process function with Vec<Item> parameter handling
    assert!(
        lib_rs.content.contains("batch_process"),
        "Should contain batch_process function"
    );

    // The generated code should contain array iteration logic for Vec<Item>
    // (looking for the manual conversion pattern we implemented)
    assert!(
        lib_rs.content.contains("items_core") || lib_rs.content.contains(".iter()"),
        "Should contain array iteration logic for Vec<Item> parameter conversion"
    );

    // Should NOT contain a panic stub or empty body for the function
    assert!(
        !lib_rs
            .content
            .contains(&"fn batch_process() {\n        unimplemented!()".to_string()),
        "Should NOT generate unimplemented stub for batch_process"
    );
}

#[test]
fn test_dto_stubs_use_final_class_with_readonly_promoted_params() {
    // PHP 8.3+ idiom: DTOs must be emitted as `final class` with constructor property
    // promotion (`public readonly`) and no redundant getter methods.
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SystemMessage".to_string(),
            rust_path: "test_lib::SystemMessage".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field("name", TypeRef::String, true),
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

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // (a) DTOs must use `final class`, not bare `class`
    assert!(
        content.contains("final class SystemMessage"),
        "DTO stub must use `final class`; content:\n{content}"
    );

    // (b) Constructor parameters must use `public readonly` promotion
    assert!(
        content.contains("public readonly string $content"),
        "Required field must use `public readonly` promotion; content:\n{content}"
    );
    assert!(
        content.contains("public readonly ?string $name"),
        "Optional field must use `public readonly` promotion with nullable type; content:\n{content}"
    );

    // (c) No redundant getFoo() getter methods alongside public readonly properties
    assert!(
        !content.contains("getContent()"),
        "Redundant getter `getContent()` must not be emitted; content:\n{content}"
    );
    assert!(
        !content.contains("getName()"),
        "Redundant getter `getName()` must not be emitted; content:\n{content}"
    );

    // (d) No separate property declarations (they are promoted into the constructor)
    assert!(
        !content.contains("    public string $content;"),
        "Separate property declaration must not be emitted; content:\n{content}"
    );
}

#[test]
fn test_dto_properties_use_camel_case_php_names() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("device_id", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("include_headers", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field("strip_text", TypeRef::String, true),
                make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U64), true),
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
    let stubs_result = backend.generate_type_stubs(&api, &config);
    assert!(stubs_result.is_ok(), "Stub generation should succeed");

    let stubs_files = stubs_result.unwrap();
    let stubs = stubs_files.first().expect("Should generate stubs file");
    let content = &stubs.content;

    // Verify camelCase conversion in PHP stubs: snake_case Rust names → camelCase PHP names
    assert!(
        content.contains("$deviceId"),
        "Property device_id should be converted to $deviceId (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$includeHeaders"),
        "Property include_headers should be converted to $includeHeaders (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$stripText"),
        "Property strip_text should be converted to $stripText (camelCase)\nContent:\n{content}"
    );
    assert!(
        content.contains("$timeoutMs"),
        "Property timeout_ms should be converted to $timeoutMs (camelCase)\nContent:\n{content}"
    );

    // Verify snake_case names are NOT present in stubs
    assert!(
        !content.contains("$device_id"),
        "Property name should NOT be in snake_case: $device_id\nContent:\n{content}"
    );
    assert!(
        !content.contains("$include_headers"),
        "Property name should NOT be in snake_case: $include_headers\nContent:\n{content}"
    );
}

#[test]
fn test_unit_enums_emit_native_php_81_backed_enums() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "test_lib::OutputFormat".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Markdown".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Html".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Output format options".to_string(),
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
    };

    let config = make_config();
    let stubs_result = backend.generate_type_stubs(&api, &config);
    assert!(stubs_result.is_ok(), "Stub generation should succeed");

    let stubs_files = stubs_result.unwrap();
    let stubs = stubs_files.first().expect("Should generate stubs file");
    let content = &stubs.content;

    // Unit-variant enums should emit as native PHP 8.1+ backed enums
    assert!(
        content.contains("enum OutputFormat: string"),
        "Unit-variant enum should be emitted as PHP 8.1+ native enum with string backing\nContent:\n{content}"
    );
    assert!(
        content.contains("case Text = "),
        "Enum case Text should be present with value\nContent:\n{content}"
    );
    assert!(
        content.contains("case Markdown = "),
        "Enum case Markdown should be present with value\nContent:\n{content}"
    );
    assert!(
        content.contains("case Html = "),
        "Enum case Html should be present with value\nContent:\n{content}"
    );

    // Should NOT emit as a class with constants
    assert!(
        !content.contains("final class OutputFormat") && !content.contains("public const Text"),
        "Unit-variant enum should NOT be emitted as a class with constants\nContent:\n{content}"
    );
}
