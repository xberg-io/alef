use super::*;

#[test]
fn test_gen_method_error_type_napi_async_pattern() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "validate".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("ValidErr".to_string()),
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
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;
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

    assert!(result.contains("pub fn validate"), "should have method name");
    assert!(
        result.contains("napi::Error"),
        "napi pattern should use napi::Error for error conversion"
    );
}

#[test]
fn test_gen_method_error_type_pyo3_async_pattern() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "validate".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("ValidErr".to_string()),
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
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
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

    assert!(result.contains("pub fn validate"), "should have method name");
    assert!(
        result.contains("PyRuntimeError"),
        "pyo3 pattern should use PyRuntimeError for error conversion"
    );
}

#[test]
fn test_gen_static_method_adapter_body_used() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "create_special".to_string(),
        params: vec![],
        return_type: TypeRef::Json,
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
    let mut adapter_bodies = AdapterBodies::default();
    adapter_bodies.insert(
        "MyConfig.create_special".to_string(),
        "MyConfig::create_impl_special()".to_string(),
    );
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

    assert!(
        result.contains("MyConfig::create_impl_special()"),
        "should use adapter body instead of generated body"
    );
}

#[test]
fn test_gen_method_adapter_body_used() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "custom_method".to_string(),
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
    let mut adapter_bodies = AdapterBodies::default();
    adapter_bodies.insert(
        "MyConfig.custom_method".to_string(),
        "\"custom adapter result\".to_string()".to_string(),
    );
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

    assert!(result.contains("\"custom adapter result\""), "should use adapter body");
}

#[test]
fn test_gen_impl_block_with_type_name_prefix() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let mut cfg = default_cfg();
    cfg.type_name_prefix = "Js";
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_impl_block(&typ, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("impl JsMyConfig {"), "should use type_name_prefix");
}

#[test]
fn test_gen_impl_block_with_method_block_attr() {
    let mut typ = simple_type_def();
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
    let mut cfg = default_cfg();
    cfg.method_block_attr = Some("pymethods");
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_impl_block(&typ, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("#[pymethods]"), "should include method_block_attr");
}

#[test]
fn test_gen_constructor_more_than_7_fields_gets_clippy_allow() {
    // Types with >7 fields should get #[allow(clippy::too_many_arguments)] on constructor
    let mut typ = simple_type_def();
    for i in 0..8 {
        typ.fields.push(FieldDef {
            name: format!("extra_{i}"),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        });
    }
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_constructor(&typ, &mapper, &cfg);

    assert!(
        result.contains("too_many_arguments"),
        "should suppress too_many_arguments when >7 fields"
    );
}

#[test]
fn test_gen_static_method_async_napi_pattern() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "load_async".to_string(),
        params: vec![],
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: true,
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
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;
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

    assert!(result.contains("pub fn load_async"), "should have method name");
    assert!(result.contains("await"), "async method should await the core call");
}

#[test]
fn test_gen_method_opaque_with_error_non_unit_return() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    let method = MethodDef {
        name: "transform".to_string(),
        params: vec![],
        return_type: TypeRef::String,
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

    assert!(result.contains("pub fn transform"), "should have method name");
    assert!(result.contains("Ok("), "should wrap result in Ok()");
    assert!(result.contains("self.inner"), "should delegate to self.inner");
}
