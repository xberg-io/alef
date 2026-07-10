use super::*;

#[test]
fn test_gen_call_args_json_param() {
    let params = vec![ParamDef {
        name: "meta".to_string(),
        ty: TypeRef::Json,
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
    assert_eq!(
        result, "serde_json::from_str(&meta).unwrap_or_default()",
        "Json param should be parsed from string"
    );
}

#[test]
fn test_gen_call_args_json_param_optional() {
    let params = vec![ParamDef {
        name: "meta".to_string(),
        ty: TypeRef::Json,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "meta.as_ref().and_then(|s| serde_json::from_str(s).ok())",
        "Optional Json param should be conditionally parsed"
    );
}

#[test]
fn test_gen_call_args_bytes_param_is_ref() {
    let params = vec![ParamDef {
        name: "data".to_string(),
        ty: TypeRef::Bytes,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "&data", "Bytes is_ref should pass as reference");
}

#[test]
fn test_gen_call_args_bytes_param_owned() {
    let params = vec![ParamDef {
        name: "data".to_string(),
        ty: TypeRef::Bytes,
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
    assert_eq!(result, "data", "Bytes owned should pass directly");
}

#[test]
fn test_gen_call_args_bytes_optional_is_ref() {
    let params = vec![ParamDef {
        name: "data".to_string(),
        ty: TypeRef::Bytes,
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
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "data.as_deref()", "Optional Bytes is_ref should use as_deref()");
}

#[test]
fn test_gen_call_args_duration_optional() {
    let params = vec![ParamDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "timeout.map(std::time::Duration::from_millis)",
        "Optional Duration should be mapped"
    );
}

#[test]
fn test_gen_call_args_path_is_ref() {
    let params = vec![ParamDef {
        name: "file".to_string(),
        ty: TypeRef::Path,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "std::path::Path::new(&file)",
        "Path is_ref should use Path::new"
    );
}

#[test]
fn test_gen_call_args_path_optional_is_ref() {
    let params = vec![ParamDef {
        name: "file".to_string(),
        ty: TypeRef::Path,
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
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "file.as_deref().map(std::path::Path::new)",
        "Optional Path is_ref should use as_deref().map(Path::new)"
    );
}

#[test]
fn test_gen_call_args_path_optional_not_ref() {
    let params = vec![ParamDef {
        name: "dir".to_string(),
        ty: TypeRef::Path,
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "dir.map(std::path::PathBuf::from)",
        "Optional Path owned should map with PathBuf::from"
    );
}

#[test]
fn test_gen_call_args_string_is_ref() {
    let params = vec![ParamDef {
        name: "name".to_string(),
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
    }];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(result, "&name", "String is_ref should pass as &str reference");
}

#[test]
fn test_gen_call_args_string_optional_is_ref() {
    let params = vec![ParamDef {
        name: "label".to_string(),
        ty: TypeRef::String,
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
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "label.as_deref()",
        "Optional String is_ref should use as_deref()"
    );
}

#[test]
fn test_gen_call_args_vec_mut_ref() {
    let params = vec![ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: true,
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
    assert_eq!(result, "&mut items", "Vec mut should pass as &mut");
}

#[test]
fn test_gen_call_args_opaque_optional() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());
    let params = vec![ParamDef {
        name: "obj".to_string(),
        ty: TypeRef::Named("MyOpaque".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "obj.as_ref().map(|v| &v.inner)",
        "Optional opaque param should map to reference to inner"
    );
}

#[test]
fn test_gen_call_args_non_opaque_optional() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "cfg".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    let result = binding_helpers::gen_call_args(&params, &opaque_types);
    assert_eq!(
        result, "cfg.map(Into::into)",
        "Optional non-opaque Named should map with Into::into"
    );
}

#[test]
fn test_gen_named_let_bindings_no_promote_non_opaque() {
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

    let result = binding_helpers::gen_named_let_bindings_no_promote(&params, &opaque_types, "my_crate");
    assert!(
        result.contains("let config_core: my_crate::Config = config.into();"),
        "should generate let binding for non-opaque named param"
    );
}

#[test]
fn test_gen_named_let_bindings_optional_without_ref() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(
        result.contains("let config_core: Option<my_crate::Config> = config.map(Into::into);"),
        "should generate Option let binding for optional Named param"
    );
}

#[test]
fn test_gen_named_let_bindings_vec_named_non_opaque() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(
        result.contains("let items_core: Vec<_> = items.into_iter().map(Into::into).collect();"),
        "should generate Vec let binding converting elements"
    );
}

#[test]
fn test_gen_named_let_bindings_vec_string_is_ref() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "labels".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
    }];

    let result = binding_helpers::gen_named_let_bindings_pub(&params, &opaque_types, "my_crate");
    assert!(
        result.contains("let labels_refs: Vec<&str>"),
        "should generate Vec<&str> intermediate for Vec<String> is_ref=true"
    );
    assert!(
        result.contains(".iter().map(|s| s.as_str()).collect()"),
        "should collect str references"
    );
}

#[test]
fn test_gen_named_let_bindings_vec_string_is_ref_optional() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "tags".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
    assert!(
        result.contains("let tags_refs: Vec<&str>"),
        "should generate Vec<&str> intermediate for optional Vec<String> is_ref=true"
    );
}
