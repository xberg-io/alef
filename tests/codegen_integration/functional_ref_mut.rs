use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_gen_method_functional_ref_mut_unit_return() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "apply_update".to_string(),
        params: vec![ParamDef {
            name: "count".to_string(),
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
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
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

    assert!(result.contains("pub fn apply_update"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver, not &mut self");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    assert!(result.contains("-> Self"), "should return Self (functional pattern)");
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    assert!(
        result.contains("core_self.apply_update("),
        "should call core method on core_self"
    );
    assert!(
        result.contains("core_self.into()"),
        "should convert mutated core back to Self"
    );
}

#[test]
fn test_gen_method_functional_ref_mut_with_named_param() {
    let mut typ = simple_type_def();
    typ.name = "ConversionOptions".to_string();
    typ.rust_path = "my_crate::ConversionOptions".to_string();

    let method = MethodDef {
        name: "apply_update".to_string(),
        params: vec![ParamDef {
            name: "update".to_string(),
            ty: TypeRef::Named("ConversionOptionsUpdate".to_string()),
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
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
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

    assert!(result.contains("pub fn apply_update"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    assert!(result.contains("-> Self"), "should return Self");
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    assert!(
        result.contains("update.into()"),
        "should convert Named param via .into()"
    );
    assert!(
        result.contains("core_self.into()"),
        "should convert mutated core back to Self"
    );
}

#[test]
fn test_gen_method_functional_ref_mut_with_error_type() {
    let typ = simple_type_def();
    let method = MethodDef {
        name: "try_apply".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
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
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("MyError".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
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

    assert!(result.contains("pub fn try_apply"), "should contain method name");
    assert!(result.contains("&self"), "should use &self receiver");
    assert!(!result.contains("&mut self"), "should not use &mut self");
    assert!(result.contains("Result<Self"), "should return Result<Self>");
    assert!(result.contains("let mut core_self"), "should declare mutable core_self");
    assert!(result.contains("core_self.try_apply("), "should call core method");
    assert!(
        result.contains("Ok(core_self.into())"),
        "should return Ok(core_self.into()) on success"
    );
}
