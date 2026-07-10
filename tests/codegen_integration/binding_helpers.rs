use super::*;

// ==============================================================================
// ==============================================================================

#[test]
fn test_wrap_return_primitive_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Primitive(PrimitiveType::U32),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_unit_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::Unit, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_string_ref_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::String, "MyType", &opaque_types, false, true, false);
    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_string_owned_passthrough() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::String, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result");
}

#[test]
fn test_wrap_return_path_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return("result", &TypeRef::Path, "MyType", &opaque_types, false, false, false);
    assert_eq!(result, "result.to_string_lossy().to_string()");
}

#[test]
fn test_wrap_return_duration_conversion() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Duration,
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.as_millis() as u64");
}

#[test]
fn test_wrap_return_opaque_self_owned() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("MyType".to_string()),
        "MyType",
        &opaque_types,
        true,
        false,
        false,
    );
    assert_eq!(result, "Self { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_other_opaque_type() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("OtherType".to_string());
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("OtherType".to_string()),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "OtherType { inner: Arc::new(result) }");
}

#[test]
fn test_wrap_return_non_opaque_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Named("SomeType".to_string()),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.into()");
}

#[test]
fn test_wrap_return_optional_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Named("SomeType".to_string()))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.map(Into::into)");
}

#[test]
fn test_wrap_return_vec_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.into_iter().map(Into::into).collect()");
}

#[test]
fn test_wrap_return_optional_vec_named() {
    let opaque_types = AHashSet::new();
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(result, "result.map(|v| v.into_iter().map(Into::into).collect())");
}

#[test]
fn test_wrap_return_optional_vec_opaque_named() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let result = binding_helpers::wrap_return(
        "result",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))))),
        "MyType",
        &opaque_types,
        false,
        false,
        false,
    );
    assert_eq!(
        result,
        "result.map(|v| v.into_iter().map(|x| Item { inner: Arc::new(x) }).collect())"
    );
}

#[test]
fn test_gen_call_args_string_param() {
    let params = vec![ParamDef {
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "input");
}

#[test]
fn test_gen_call_args_primitive_param() {
    let params = vec![ParamDef {
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "count");
}

#[test]
fn test_gen_call_args_opaque_param() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "&obj.inner");
}

#[test]
fn test_gen_call_args_non_opaque_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "config.into()");
}

#[test]
fn test_gen_call_args_optional_non_opaque_ref_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
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
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "config.as_ref()");
}

#[test]
fn test_gen_call_args_path_param() {
    let params = vec![ParamDef {
        name: "file_path".to_string(),
        ty: TypeRef::Path,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "std::path::PathBuf::from(file_path)");
}

#[test]
fn test_gen_call_args_duration_param() {
    let params = vec![ParamDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "std::time::Duration::from_millis(timeout)");
}

#[test]
fn test_gen_call_args_multiple_params() {
    let opaque_types = AHashSet::new();
    let params = vec![
        ParamDef {
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
        },
        ParamDef {
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
        },
    ];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "name, count");
}

#[test]
fn test_gen_call_args_with_let_bindings_opaque() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "&obj.inner");
}

#[test]
fn test_gen_call_args_with_let_bindings_non_opaque() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "config_core");
}

#[test]
fn test_gen_named_let_bindings_empty_params() {
    let opaque_types = AHashSet::new();
    let params = vec![];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert_eq!(result, "");
}

#[test]
fn test_gen_named_let_bindings_non_opaque_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(result.contains("let config_core: my_crate::Config = config.into();"));
}

#[test]
fn test_gen_named_let_bindings_optional_ref_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(result.contains("let config_owned: Option<my_crate::Config> = config.map(Into::into);"));
    assert!(result.contains("let config_core = config_owned.as_ref();"));
}

#[test]
fn test_gen_call_args_with_let_bindings_optional_ref_param() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: true,
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
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "config_core");
}

#[test]
fn test_gen_call_args_with_let_bindings_optional_ref_vec_named() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        optional: true,
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
    }];

    let result = binding_helpers::gen_call_args_with_let_bindings(&params, &opaque_types);
    assert_eq!(result, "items_core.as_deref()");
}

#[test]
fn test_gen_named_let_bindings_opaque_skipped() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert_eq!(result, "");
}

#[test]
fn test_has_named_params_returns_true() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    assert!(binding_helpers::has_named_params(&params, &opaque_types));
}

#[test]
fn test_has_named_params_returns_false() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
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
    }];

    assert!(!binding_helpers::has_named_params(&params, &opaque_types));
}

#[test]
fn test_is_simple_non_opaque_param_string() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::String));
}

#[test]
fn test_is_simple_non_opaque_param_primitive() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Primitive(
        PrimitiveType::U32
    )));
}

#[test]
fn test_is_simple_non_opaque_param_path() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Path));
}

#[test]
fn test_is_simple_non_opaque_param_duration() {
    assert!(binding_helpers::is_simple_non_opaque_param(&TypeRef::Duration));
}

#[test]
fn test_is_simple_non_opaque_param_vec_is_false() {
    assert!(!binding_helpers::is_simple_non_opaque_param(&TypeRef::Vec(Box::new(
        TypeRef::String
    ))));
}

#[test]
fn test_is_simple_non_opaque_param_named_is_false() {
    assert!(!binding_helpers::is_simple_non_opaque_param(&TypeRef::Named(
        "Config".to_string()
    )));
}
