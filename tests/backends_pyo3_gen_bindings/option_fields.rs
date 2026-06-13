use super::*;

/// Regression test: struct fields with `Option<T>` must be emitted as `Option<T>` in constructor
/// signatures, not as bare `T`. This applies to any `T`: `Option<u64>`, `Option<String>`, etc.
#[test]
fn test_option_fields_in_constructor_signature() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OptionalFieldsType".to_string(),
            rust_path: "test_lib::OptionalFieldsType".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("opt_u64", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("opt_string", TypeRef::String, true),
                make_field("opt_duration", TypeRef::Duration, true),
                make_field("required_u32", TypeRef::Primitive(PrimitiveType::U32), false),
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
            doc: "Type with optional fields".to_string(),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // All optional fields must have Option<T> parameter types, not bare T
    assert!(
        lib_rs.content.contains("pub fn new("),
        "Constructor should exist; content:\n{}",
        lib_rs.content
    );

    // Check for Option<u64> — NOT bare u64
    assert!(
        lib_rs.content.contains("opt_u64: Option<u64>"),
        "Parameter opt_u64 must be Option<u64>, not bare u64; content:\n{}",
        lib_rs.content
    );

    // Check for Option<String> — NOT bare String
    assert!(
        lib_rs.content.contains("opt_string: Option<String>"),
        "Parameter opt_string must be Option<String>, not bare String; content:\n{}",
        lib_rs.content
    );

    // Check for Option<u64> (Duration maps to u64) — NOT bare u64
    assert!(
        lib_rs.content.contains("opt_duration: Option<u64>"),
        "Parameter opt_duration must be Option<u64>, not bare u64; content:\n{}",
        lib_rs.content
    );

    // Required field must be bare type, not optional
    assert!(
        lib_rs.content.contains("required_u32: u32"),
        "Parameter required_u32 must be u32 (not optional); content:\n{}",
        lib_rs.content
    );

    // Defaults should be None for optional fields
    assert!(
        lib_rs.content.contains("opt_u64=None") || lib_rs.content.contains("opt_u64 = None"),
        "Optional field opt_u64 should default to None; content:\n{}",
        lib_rs.content
    );
}

/// Test for Option fields on has_default types (the actual bug case in sample_crawler).
#[test]
fn test_option_fields_on_has_default_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigWithDefaults".to_string(),
            rust_path: "test_lib::ConfigWithDefaults".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("request_timeout", TypeRef::Primitive(PrimitiveType::U64), true),
                make_field("name", TypeRef::String, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true, // This is the key difference — has_default
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Config with defaults".to_string(),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    println!("Generated lib.rs for has_default type:\n{}\n", lib_rs.content);

    // The constructor must have Option<u64> parameters, NOT bare u64
    assert!(
        lib_rs.content.contains("timeout: Option<u64>"),
        "Parameter timeout must be Option<u64> for has_default type; content:\n{}",
        lib_rs.content
    );

    assert!(
        lib_rs.content.contains("request_timeout: Option<u64>"),
        "Parameter request_timeout must be Option<u64> for has_default type; content:\n{}",
        lib_rs.content
    );

    // Defaults should be None for optional fields
    assert!(
        lib_rs.content.contains("timeout=None") || lib_rs.content.contains("timeout = None"),
        "Optional field timeout should default to None; content:\n{}",
        lib_rs.content
    );
}

/// Test for Option fields on has_default types WITH serde_rename.
#[test]
fn test_option_fields_with_serde_rename_on_has_default() {
    let backend = Pyo3Backend;

    let mut timeout_field = make_field("timeout", TypeRef::Primitive(PrimitiveType::U64), true);
    timeout_field.serde_rename = Some("timeout_ms".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "RequestOptions".to_string(),
            rust_path: "test_lib::RequestOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![timeout_field, make_field("name", TypeRef::String, false)],
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
            doc: "Request options".to_string(),
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
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    println!(
        "Generated lib.rs for has_default type with serde_rename:\n{}\n",
        lib_rs.content
    );

    // The constructor parameter for serde-renamed optional field must still be Option<u64>
    assert!(
        lib_rs.content.contains("timeout_ms: Option<u64>"),
        "Parameter timeout_ms must be Option<u64> even with serde_rename; content:\n{}",
        lib_rs.content
    );

    // Verify it defaults to None
    assert!(
        lib_rs.content.contains("timeout_ms=None") || lib_rs.content.contains("timeout_ms = None"),
        "Optional field timeout_ms should default to None; content:\n{}",
        lib_rs.content
    );
}

#[test]
fn test_has_default_struct_with_nested_struct_field_accepts_none() {
    // This test verifies BLK-5 fix: a has_default struct with a non-optional
    // nested-struct field whose type also has has_default=true should accept None
    // in the constructor, with an unwrap_or_else falling back to the nested type's default.
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            // Nested struct that derives Default
            TypeDef {
                name: "PreprocessingOptions".to_string(),
                rust_path: "test_lib::PreprocessingOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("normalize", TypeRef::Primitive(PrimitiveType::Bool), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true, // This type derives Default
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Preprocessing options".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Parent struct with has_default=true owning non-optional PreprocessingOptions
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    // This is the critical case: non-optional nested struct field on a has_default type
                    // Should be emitted as Option<PreprocessingOptions> with default None
                    make_field(
                        "preprocessing",
                        TypeRef::Named("PreprocessingOptions".to_string()),
                        false,
                    ),
                    make_field("format", TypeRef::String, false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true, // Parent also derives Default
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Conversion options".to_string(),
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
        ..Default::default()
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings: {}", result.unwrap_err());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify that the ParseOptions constructor parameter 'preprocessing' is Option<PreprocessingOptions>
    // The parameter should be declared as Option<PreprocessingOptions>
    assert!(
        content.contains("preprocessing: Option<PreprocessingOptions>"),
        "Parameter 'preprocessing' must be Option<PreprocessingOptions> to accept None; content:\n{}",
        content
    );

    // Verify the default is None
    assert!(
        content.contains("preprocessing=None") || content.contains("preprocessing = None"),
        "Parameter 'preprocessing' should default to None; content:\n{}",
        content
    );

    // Verify the assignment uses unwrap_or_else to fall back to the nested type's default
    assert!(
        content.contains("preprocessing.unwrap_or_else(|| Self::default().preprocessing)"),
        "Assignment must use unwrap_or_else fallback; content:\n{}",
        content
    );
}

#[test]
fn test_options_field_bridge_field_not_duplicated_when_cfg_force_restored() {
    // Regression test: when a trait-bridge `bind_via = OptionsField` field is also
    // cfg-gated on a `has_default` type, the backend force-restores it into
    // `never_skip_cfg_field_names`. The constructor rewriter must filter it out of
    // `sorted_fields` (so it does not appear via the params iterator) and rely on
    // the existing `bridge_param` append at the end of the param list — otherwise
    // the field appears twice and rustc rejects with E0415
    // ("identifier 'visitor' is bound more than once in this parameter list").
    let backend = Pyo3Backend;

    let mut visitor_field = make_field(
        "visitor",
        TypeRef::Optional(Box::new(TypeRef::Named("VisitorHandle".to_string()))),
        true,
    );
    visitor_field.cfg = Some("feature = \"visitor\"".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "VisitorHandle".to_string(),
                rust_path: "test_lib::VisitorHandle".to_string(),
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
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ParseOptions".to_string(),
                rust_path: "test_lib::ParseOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("format", TypeRef::String, false), visitor_field],
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
        ..Default::default()
    };

    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "SyntaxWalker".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: alef::core::config::BridgeBinding::OptionsField,
        options_type: Some("ParseOptions".to_string()),
        options_field: Some("visitor".to_string()),
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }];

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Failed to generate bindings: {}", result.unwrap_err());

    let files = result.unwrap();
    let content = &files[0].content;

    let conversion_options_block = content
        .split("impl ParseOptions")
        .nth(1)
        .expect("ParseOptions impl block must exist");
    let constructor_body = conversion_options_block
        .split("pub fn new(")
        .nth(1)
        .and_then(|s| s.split(") -> Self").next())
        .expect("ParseOptions::new param list must exist");

    let visitor_param_count = constructor_body.matches("visitor:").count();
    assert_eq!(
        visitor_param_count, 1,
        "ParseOptions::new must declare `visitor:` exactly once, found {} in:\n{}",
        visitor_param_count, constructor_body
    );
}
