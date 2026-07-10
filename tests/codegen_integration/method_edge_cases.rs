use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_is_trait_method_name_known_names() {
    use alef::codegen::generators::is_trait_method_name;
    assert!(is_trait_method_name("from"), "from conflicts with From trait");
    assert!(is_trait_method_name("into"), "into conflicts with Into trait");
    assert!(is_trait_method_name("eq"), "eq conflicts with PartialEq");
    assert!(is_trait_method_name("default"), "default conflicts with Default trait");
    assert!(is_trait_method_name("add"), "add conflicts with Add trait");
    assert!(is_trait_method_name("deref"), "deref conflicts with Deref trait");
}

#[test]
fn test_is_trait_method_name_unknown_names() {
    use alef::codegen::generators::is_trait_method_name;
    assert!(!is_trait_method_name("process"), "process is not a trait method");
    assert!(!is_trait_method_name("new"), "new is not a conflicting trait method");
    assert!(!is_trait_method_name("build"), "build is not a trait method");
    assert!(!is_trait_method_name(""), "empty string is not a trait method");
}

#[test]
fn test_gen_method_trait_method_name_suppresses_clippy_lint() {
    // Methods named "from" should get #[allow(clippy::should_implement_trait)]
    let typ = simple_type_def();
    let method = MethodDef {
        name: "from".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
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
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(
        result.contains("should_implement_trait"),
        "should suppress should_implement_trait for method named 'from'"
    );
}

#[test]
fn test_gen_method_error_type_with_opaque_unit_return() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "update".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("MyError".to_string()),
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn update"), "should contain method name");
    assert!(result.contains("Ok(())"), "unit return with error should have Ok(())");
    assert!(result.contains("Result"), "should return Result type");
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc"
    );
}

#[test]
fn test_gen_method_opaque_delegation_string_return() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "get_label".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn get_label"), "should have method name");
    assert!(result.contains("self.inner"), "opaque delegation uses self.inner");
    assert!(result.contains("-> String"), "should have String return type");
}

#[test]
fn test_gen_method_opaque_delegation_returns_opaque_self() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "clone_with_prefix".to_string(),
        params: vec![ParamDef {
            name: "prefix".to_string(),
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
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn clone_with_prefix"), "should have method name");
    assert!(
        result.contains("Self { inner: Arc::new"),
        "opaque Self return wraps in Arc"
    );
}

#[test]
fn test_gen_method_with_mutex_opaque_type() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "get_count".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        is_static: false,
        error_type: None,
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());
    let mut mutex_types = AHashSet::new();
    mutex_types.insert("MyConfig".to_string());

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &mutex_types,
        &adapter_bodies,
    );

    assert!(result.contains("pub fn get_count"), "should have method name");
    assert!(
        result.contains("lock().unwrap()"),
        "mutex types acquire lock before calling method"
    );
}

#[test]
fn test_gen_method_trait_source_not_delegated() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "process".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("MyTrait".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        true,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(result.contains("pub fn process"), "should have method name");
    assert!(
        !result.contains("self.inner.process"),
        "trait methods are not delegated via self.inner"
    );
}

#[test]
fn test_gen_static_method_with_error_type_generates_result() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "parse".to_string(),
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
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: true,
        error_type: Some("ParseError".to_string()),
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = gen_static_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        &adapter_bodies,
        &opaque_types,
        &mutex_types,
    );

    assert!(result.contains("pub fn parse"), "should have method name");
    assert!(result.contains("input: String"), "should have input param");
    assert!(result.contains("Result"), "should return Result due to error_type");
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc lint"
    );
    assert!(!result.contains("&self"), "static methods should not have &self");
}

#[test]
fn test_gen_static_method_with_primitive_return() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "count".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();
    let mutex_types = AHashSet::new();

    let result = gen_static_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        &adapter_bodies,
        &opaque_types,
        &mutex_types,
    );

    assert!(result.contains("pub fn count"), "should have method name");
    assert!(result.contains("-> u32"), "should have u32 return type");
    assert!(!result.contains("&self"), "static methods have no receiver");
}

#[test]
fn test_gen_opaque_impl_block_generates_delegation() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    typ.methods = vec![MethodDef {
        name: "get_name".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: None,
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
    }];

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyConfig".to_string());

    let result = gen_opaque_impl_block(&typ, &mapper, &cfg, &opaque_types, &AHashSet::new(), &adapter_bodies);

    assert!(result.contains("impl MyConfig {"), "should contain impl block");
    assert!(result.contains("pub fn get_name"), "should contain delegated method");
    assert!(result.starts_with("impl"), "should start with impl");
    assert!(result.trim_end().ends_with("}"), "should end with closing brace");
}

#[test]
fn test_gen_opaque_impl_block_empty_when_all_sanitized() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    typ.methods = vec![MethodDef {
        name: "secret".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: true,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_opaque_impl_block(&typ, &mapper, &cfg, &opaque_types, &AHashSet::new(), &adapter_bodies);

    assert!(
        result.is_empty(),
        "impl block should be empty when all methods are sanitized"
    );
}

#[test]
fn test_gen_method_too_many_arguments_gets_clippy_allow() {
    // Methods with >7 params should get #[allow(clippy::too_many_arguments)]
    let typ = simple_type_def();
    let method = MethodDef {
        name: "complex".to_string(),
        params: vec![
            ParamDef {
                name: "a".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "b".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "c".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "d".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "e".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "f".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "g".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
                name: "h".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: false,
        is_static: false,
        error_type: None,
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
    };
    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_method(
        &method,
        &mapper,
        &cfg,
        &typ,
        false,
        &opaque_types,
        &AHashSet::new(),
        &adapter_bodies,
    );

    assert!(
        result.contains("too_many_arguments"),
        "should suppress too_many_arguments when >7 params"
    );
}
