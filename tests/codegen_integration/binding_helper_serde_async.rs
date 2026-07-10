use super::*;

#[test]
fn test_gen_serde_let_bindings_non_opaque_named_required() {
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
    let err_conv = ".map_err(|e| e.to_string())";
    let indent = "        ";

    let result = binding_helpers::gen_serde_let_bindings(&params, &opaque_types, "my_crate", err_conv, indent);

    assert!(
        result.contains("let config_json = serde_json::to_string(&config)"),
        "should serialize binding to JSON"
    );
    assert!(
        result.contains("let config_core: my_crate::Config = serde_json::from_str(&config_json)"),
        "should deserialize JSON to core type"
    );
}

#[test]
fn test_gen_serde_let_bindings_non_opaque_named_optional() {
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
    let err_conv = ".map_err(|e| e.to_string())";
    let indent = "        ";

    let result = binding_helpers::gen_serde_let_bindings(&params, &opaque_types, "my_crate", err_conv, indent);

    assert!(
        result.contains("let config_core: Option<my_crate::Config>"),
        "optional serde binding should wrap in Option"
    );
    assert!(result.contains(".map(|v| {"), "optional should use map pattern");
}

#[test]
fn test_gen_serde_let_bindings_vec_named() {
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
    let err_conv = ".map_err(|e| e.to_string())";
    let indent = "        ";

    let result = binding_helpers::gen_serde_let_bindings(&params, &opaque_types, "my_crate", err_conv, indent);

    assert!(
        result.contains("let items_json = serde_json::to_string(&items)"),
        "should serialize Vec binding to JSON"
    );
    assert!(
        result.contains("let items_core: Vec<my_crate::Item>"),
        "should deserialize to Vec of core type"
    );
}

#[test]
fn test_gen_serde_let_bindings_opaque_type_skipped() {
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
    let err_conv = ".map_err(|e| e.to_string())";
    let indent = "        ";

    let result = binding_helpers::gen_serde_let_bindings(&params, &opaque_types, "my_crate", err_conv, indent);

    assert!(result.is_empty(), "opaque types should not generate serde let bindings");
}

#[test]
fn test_gen_serde_let_bindings_empty_params() {
    let opaque_types = AHashSet::new();
    let params = vec![];
    let err_conv = ".map_err(|e| e.to_string())";
    let indent = "        ";

    let result = binding_helpers::gen_serde_let_bindings(&params, &opaque_types, "my_crate", err_conv, indent);

    assert!(result.is_empty(), "empty params should produce empty bindings");
}

#[test]
fn test_gen_async_body_tokio_block_on_with_error_opaque() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::TokioBlockOn;

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, true, "result", true, "", false, None);

    assert!(
        result.contains("tokio::runtime::Runtime::new()"),
        "should create tokio runtime"
    );
    assert!(result.contains("block_on"), "should call block_on");
    assert!(result.contains("map_err"), "should convert error");
    assert!(result.contains("result"), "should contain return_wrap expression");
}

#[test]
fn test_gen_async_body_tokio_block_on_no_error_opaque() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::TokioBlockOn;

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, false, "result", true, "", false, None);

    assert!(
        result.contains("tokio::runtime::Runtime::new()"),
        "should create runtime"
    );
    assert!(result.contains("block_on"), "should call block_on");
    assert!(result.contains("result"), "should include return wrap");
}

#[test]
fn test_gen_async_body_tokio_block_on_no_error_non_opaque() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::TokioBlockOn;

    let result = binding_helpers::gen_async_body("CoreType::process()", &cfg, false, "result", false, "", false, None);

    assert!(
        result.contains("tokio::runtime::Runtime::new()"),
        "should create runtime"
    );
    assert!(result.contains("block_on"), "should call block_on");
}

#[test]
fn test_gen_async_body_tokio_block_on_unit_return_opaque() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::TokioBlockOn;

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, false, "()", true, "", true, None);

    assert!(
        result.contains("tokio::runtime::Runtime::new()"),
        "should create runtime"
    );
    assert!(result.contains("block_on"), "should call block_on");
}

#[test]
fn test_gen_async_body_none_pattern() {
    let cfg = default_cfg();

    let result = binding_helpers::gen_async_body("process()", &cfg, false, "result", false, "", false, None);

    assert!(
        result.contains("compile_error!"),
        "AsyncPattern::None should emit compile-time diagnostic"
    );
}

#[test]
fn test_gen_async_body_napi_no_error_no_unit() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;

    let result = binding_helpers::gen_async_body("process()", &cfg, false, "wrapped_val", false, "", false, None);

    assert!(result.contains("await"), "should have await");
    assert!(result.contains("wrapped_val"), "should include return_wrap");
    assert!(!result.contains("Ok("), "no-error NAPI path should not wrap in Ok()");
}

#[test]
fn test_gen_async_body_pyo3_with_type_annotation() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;

    let result = binding_helpers::gen_async_body(
        "inner.process()",
        &cfg,
        false,
        "result.into()",
        false,
        "",
        false,
        Some("MyType"),
    );

    assert!(
        result.contains("let wrapped_result: MyType"),
        "should add explicit type annotation"
    );
}

#[test]
fn test_gen_unimplemented_body_optional_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Optional(Box::new(TypeRef::String)),
        "get_optional",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "get_optional");
}

#[test]
fn test_gen_unimplemented_body_map_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        "get_map",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "get_map");
}

#[test]
fn test_gen_unimplemented_body_duration_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::Duration, "get_timeout", false, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "get_timeout");
}

#[test]
fn test_gen_unimplemented_body_opaque_named_return_uses_compile_error() {
    let cfg = default_cfg();
    let params = vec![];
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("MyOpaque".to_string());

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Named("MyOpaque".to_string()),
        "get_opaque",
        false,
        &cfg,
        &params,
        &opaque_types,
    );

    assert!(
        result.contains("configure an adapter body or exclude this item"),
        "should contain actionable diagnostic"
    );
    assert_unimplemented_compile_error(&result, "get_opaque");
}

#[test]
fn test_gen_unimplemented_body_non_opaque_named_return_uses_default() {
    let cfg = default_cfg();
    let params = vec![];
    let opaque_types = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Named("Config".to_string()),
        "get_config",
        false,
        &cfg,
        &params,
        &opaque_types,
    );

    assert_unimplemented_compile_error(&result, "get_config");
}

#[test]
fn test_gen_unimplemented_body_json_return_without_error() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::Json, "get_json", false, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "get_json");
}

#[test]
fn test_gen_unimplemented_body_f32_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Primitive(PrimitiveType::F32),
        "get_float",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "get_float");
}

#[test]
fn test_gen_unimplemented_body_f64_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Primitive(PrimitiveType::F64),
        "get_score",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "get_score");
}

#[test]
fn test_gen_unimplemented_body_napi_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::String, "missing_fn", true, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "missing_fn");
}

#[test]
fn test_gen_unimplemented_body_wasm_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::WasmNativeAsync;
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::String, "missing_fn", true, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "missing_fn");
}

#[test]
fn test_gen_unimplemented_body_pyo3_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::String, "missing_fn", true, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "missing_fn");
}

#[test]
fn test_gen_unimplemented_body_multiple_params_suppressed() {
    let cfg = default_cfg();
    let params = vec![
        ParamDef {
            name: "a".to_string(),
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
    ];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::Unit, "multi_param_fn", false, &cfg, &params, &empty_opaque);

    assert!(
        result.contains("let _ = (a, b);"),
        "multiple params should use tuple suppression"
    );
}

#[test]
fn test_gen_unimplemented_body_bytes_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::Bytes, "get_bytes", false, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "get_bytes");
}

#[test]
fn test_gen_unimplemented_body_path_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result =
        binding_helpers::gen_unimplemented_body(&TypeRef::Path, "get_path", false, &cfg, &params, &empty_opaque);

    assert_unimplemented_compile_error(&result, "get_path");
}
