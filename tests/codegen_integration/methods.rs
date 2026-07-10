use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_gen_constructor_produces_new_method() {
    let typ = simple_type_def();
    let mapper = RustMapper;
    let cfg = default_cfg();

    let result = gen_constructor(&typ, &mapper, &cfg);

    assert!(result.contains("pub fn new"), "should contain new function");
    assert!(result.contains("name: String"), "should accept name parameter");
    assert!(result.contains("count: Option<u32>"), "should accept count parameter");
    assert!(result.contains("Self {"), "should construct Self");
    assert!(result.contains("name"), "should include name field in struct literal");
    assert!(result.contains("count"), "should include count field in struct literal");
}

#[test]
fn test_gen_instance_method_with_ref_receiver() {
    let typ = simple_type_def();
    let method = MethodDef {
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

    assert!(result.contains("pub fn get_name"), "should contain method name");
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> String"), "should have String return type");
}

#[test]
fn test_gen_static_method_without_receiver() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "create".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
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

    assert!(result.contains("pub fn create"), "should contain static method name");
    assert!(!result.contains("&self"), "should not have &self");
    assert!(result.contains("config: String"), "should accept config parameter");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
}

#[test]
fn test_gen_async_method_generates_async_signature() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "process_async".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async: true,
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
        result.contains("pub fn process_async"),
        "should contain async method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(
        result.contains("u32") || result.contains("impl"),
        "should reference u32 return type"
    );
}

#[test]
fn test_gen_method_with_multiple_params() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "compute".to_string(),
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
                name: "label".to_string(),
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

    assert!(result.contains("pub fn compute"), "should contain method name");
    assert!(result.contains("a: u32"), "should have parameter a");
    assert!(result.contains("b: u32"), "should have parameter b");
    assert!(result.contains("label: String"), "should have parameter label");
    assert!(result.contains("-> u32"), "should have return type");
}

#[test]
fn test_gen_method_with_error_type() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "validate".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("ValidationError".to_string()),
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

    assert!(result.contains("pub fn validate"), "should contain method name");
    assert!(result.contains("Result"), "should return Result when error_type is set");
    assert!(result.contains("String"), "should contain return type in Result");
    // Should have #[allow(clippy::missing_errors_doc)] when returning Result
    assert!(
        result.contains("missing_errors_doc"),
        "should suppress missing_errors_doc lint"
    );
}

#[test]
fn test_gen_impl_block_with_constructor_and_methods() {
    let mut typ = simple_type_def();
    typ.methods = vec![
        MethodDef {
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
        },
        MethodDef {
            name: "create".to_string(),
            params: vec![],
            return_type: TypeRef::Named("MyConfig".to_string()),
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
        },
    ];

    let mapper = RustMapper;
    let cfg = default_cfg();
    let adapter_bodies = AdapterBodies::default();
    let opaque_types = AHashSet::new();

    let result = gen_impl_block(&typ, &mapper, &cfg, &adapter_bodies, &opaque_types);

    assert!(result.contains("impl MyConfig {"), "should contain impl block");
    assert!(result.contains("pub fn new"), "should contain constructor");
    assert!(result.contains("pub fn get_name"), "should contain instance method");
    assert!(result.contains("pub fn create"), "should contain static method");
    assert!(result.starts_with("impl"), "should start with impl");
    assert!(result.trim_end().ends_with("}"), "should end with closing brace");
}

#[test]
fn test_gen_method_with_optional_param() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "configure".to_string(),
        params: vec![ParamDef {
            name: "timeout".to_string(),
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
        }],
        return_type: TypeRef::Unit,
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

    assert!(result.contains("pub fn configure"), "should contain method name");
    assert!(result.contains("Option<u32>"), "should wrap optional param in Option");
    assert!(result.contains("-> ()"), "should return unit type");
}
