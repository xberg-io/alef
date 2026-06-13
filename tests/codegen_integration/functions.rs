use super::*;

// ==============================================================================
// Additional tests for functions.rs
// ==============================================================================

#[test]
fn test_gen_function_async_produces_async_signature() {
    let mut func = simple_function_def();
    func.is_async = true;

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        result.contains("pub async fn process"),
        "async function should have async keyword"
    );
}

#[test]
fn test_gen_function_with_error_type_wraps_in_result() {
    let mut func = simple_function_def();
    func.error_type = Some("MyError".to_string());

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        result.contains("-> Result"),
        "function with error_type should return Result"
    );
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc lint"
    );
}

#[test]
fn test_gen_function_named_ref_param_uses_from_conversion() {
    let func = FunctionDef {
        name: "process".to_string(),
        rust_path: "my_crate::process".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
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
            },
            ParamDef {
                name: "config".to_string(),
                ty: TypeRef::Named("ProcessConfig".to_string()),
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
            },
        ],
        return_type: TypeRef::Named("ProcessResult".to_string()),
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
    };
    let mapper = RustMapper;
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
    cfg.has_serde = true;
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("let config_core: my_crate::ProcessConfig = config.into();"));
    assert!(result.contains("my_crate::process(&source, &config_core)"));
    assert!(!result.contains("serde_json::to_string(&config)"));
}

#[test]
fn test_gen_function_with_no_params_generates_empty_param_list() {
    let func = FunctionDef {
        name: "get_version".to_string(),
        rust_path: "my_crate::get_version".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
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
    };

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(
        result.contains("pub fn get_version()"),
        "should have empty parameter list"
    );
    assert!(result.contains("-> String"), "should have String return type");
}

#[test]
fn test_gen_function_with_optional_param_wraps_in_option() {
    let func = FunctionDef {
        name: "search".to_string(),
        rust_path: "my_crate::search".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "query".to_string(),
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
                name: "limit".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
        ],
        return_type: TypeRef::Vec(Box::new(TypeRef::String)),
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
    };

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("query: String"), "required param should be plain type");
    assert!(
        result.contains("limit: Option<u32>"),
        "optional param should be wrapped in Option"
    );
}

#[test]
fn test_gen_function_uses_function_attr() {
    let func = simple_function_def();
    let mapper = RustMapper;
    let cfg = default_cfg(); // function_attr = "#[no_mangle]"
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_function(&func, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("#[no_mangle]"), "should include function_attr");
}

#[test]
fn test_collect_trait_imports_empty_when_no_trait_methods() {
    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![simple_type_def()],
        enums: vec![],
        functions: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let result = collect_trait_imports(&api);

    assert!(result.is_empty(), "no trait methods means no trait imports");
}

#[test]
fn test_collect_trait_imports_deduplicates_by_trait_name() {
    let mut typ1 = simple_type_def();
    typ1.methods = vec![MethodDef {
        name: "execute".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("my_crate::Executor".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];

    let mut typ2 = simple_type_def();
    typ2.name = "OtherType".to_string();
    typ2.methods = vec![MethodDef {
        name: "execute".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("my_crate::Executor".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];

    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![typ1, typ2],
        enums: vec![],
        functions: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let result = collect_trait_imports(&api);

    // Should deduplicate to one entry
    assert_eq!(result.len(), 1, "should deduplicate same trait path");
    assert_eq!(result[0], "my_crate::Executor");
}

#[test]
fn test_collect_explicit_core_imports_returns_type_and_enum_names() {
    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![simple_type_def()],
        enums: vec![simple_enum_def()],
        functions: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let result = collect_explicit_core_imports(&api);

    assert!(result.contains(&"MyConfig".to_string()), "should include type name");
    assert!(result.contains(&"OutputFormat".to_string()), "should include enum name");
}

#[test]
fn test_collect_explicit_core_imports_is_sorted() {
    let mut typ_b = simple_type_def();
    typ_b.name = "Bravo".to_string();
    let mut typ_a = simple_type_def();
    typ_a.name = "Alpha".to_string();

    let api = ApiSurface {
        crate_name: "my_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![typ_b, typ_a],
        enums: vec![],
        functions: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let result = collect_explicit_core_imports(&api);

    assert_eq!(
        result,
        vec!["Alpha", "Bravo"],
        "imports should be alphabetically sorted"
    );
}
