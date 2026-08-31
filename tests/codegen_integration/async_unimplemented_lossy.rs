use super::*;

#[test]
fn test_gen_async_body_pyo3_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;

    let result = binding_helpers::gen_async_body("inner.process()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("pyo3_async_runtimes::tokio::future_into_py"));
    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
}

#[test]
fn test_gen_async_body_napi_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::NapiNativeAsync;

    let result = binding_helpers::gen_async_body("CoreType::process()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("await"));
    assert!(result.contains("map_err"));
    assert!(result.contains("napi::Error"));
}

#[test]
fn test_gen_async_body_wasm_with_error() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::WasmNativeAsync;

    let result = binding_helpers::gen_async_body("process_async()", &cfg, true, "result", false, "", false, None);

    assert!(result.contains("await"));
    assert!(result.contains("JsValue"));
}

#[test]
fn test_gen_async_body_with_inner_clone_line() {
    let mut cfg = default_cfg();
    cfg.async_pattern = AsyncPattern::Pyo3FutureIntoPy;

    let result = binding_helpers::gen_async_body(
        "inner.process()",
        &cfg,
        false,
        "()",
        false,
        "let inner = self.inner.clone();\n        ",
        true,
        None,
    );

    assert!(result.contains("let inner = self.inner.clone();"));
    assert!(result.contains("pyo3_async_runtimes::tokio::future_into_py"));
}

#[test]
fn test_gen_unimplemented_body_with_error() {
    let cfg = default_cfg();
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

    let empty_opaque = AHashSet::new();
    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::String,
        "unimplemented_fn",
        true,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert!(result.contains("let _ = input;"));
    assert_unimplemented_compile_error(&result, "unimplemented_fn");
}

#[test]
fn test_gen_unimplemented_body_string_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::String,
        "unimplemented_fn",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "unimplemented_fn");
}

#[test]
fn test_gen_unimplemented_body_bool_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Primitive(PrimitiveType::Bool),
        "is_valid",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "is_valid");
}

#[test]
fn test_gen_unimplemented_body_vec_return() {
    let cfg = default_cfg();
    let params = vec![];
    let empty_opaque = AHashSet::new();

    let result = binding_helpers::gen_unimplemented_body(
        &TypeRef::Vec(Box::new(TypeRef::String)),
        "list_items",
        false,
        &cfg,
        &params,
        &empty_opaque,
    );

    assert_unimplemented_compile_error(&result, "list_items");
}

#[test]
fn test_gen_lossy_binding_to_core_fields_sanitized() {
    let mut typ = simple_type_def();
    typ.fields[0].sanitized = true;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(result.contains("let core_self"));
    assert!(result.contains("name: Default::default(),"));
    assert!(result.contains("count:"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_non_sanitized() {
    let typ = simple_type_def();

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(result.contains("let core_self"));
    assert!(result.contains("my_crate::MyConfig {"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_map_named_applies_per_value_into() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        version: Default::default(),
        name: "patterns".to_string(),
        ty: TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("ExtractionPattern".to_string())),
        ),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("patterns: self.patterns.clone().into_iter().map(|(k, v)| (k.into(), v.into())).collect()"),
        "expected per-value .into() for Map<String, Named>; got:\n{result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_optional_map_named_applies_per_value_into() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        version: Default::default(),
        name: "extractions".to_string(),
        ty: TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("ExtractionPattern".to_string())),
        ),
        optional: true,
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains(
            "extractions: self.extractions.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
        ),
        "expected Option-preserving per-value .into() for Option<Map<String, Named>>; got:\n{result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_duration() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        version: Default::default(),
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(result.contains("timeout: std::time::Duration::from_millis(self.timeout),"));
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_duration_optional_flag() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        version: Default::default(),
        name: "request_timeout".to_string(),
        ty: TypeRef::Duration,
        optional: true,
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("request_timeout: self.request_timeout.map(std::time::Duration::from_millis),"),
        "got: {result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_with_optional_duration_type() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        version: Default::default(),
        name: "request_timeout".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Duration)),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("request_timeout: self.request_timeout.map(|v| std::time::Duration::from_millis(v as u64)),"),
        "got: {result}"
    );
}

#[test]
fn test_gen_method_builder_pattern_opaque() {
    let mut typ = simple_type_def();
    typ.is_opaque = true;
    typ.name = "MyConfig".to_string();

    let method = MethodDef {
        name: "with_name".to_string(),
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
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
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
    let opaque_types = {
        let mut set = AHashSet::new();
        set.insert("MyConfig".to_string());
        set
    };

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

    assert!(
        result.contains("pub fn with_name"),
        "should contain builder method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
    assert!(
        result.contains("Self { inner: Arc::new"),
        "should wrap result in Self with Arc"
    );
    assert!(!result.contains("compile_error!"), "should not emit compile_error");
}

#[test]
fn test_gen_method_builder_pattern_non_opaque() {
    let mut typ = simple_type_def();
    typ.is_opaque = false;
    typ.name = "MyConfig".to_string();

    let method = MethodDef {
        name: "with_count".to_string(),
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
        return_type: TypeRef::Named("MyConfig".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
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
        result.contains("pub fn with_count"),
        "should contain builder method name"
    );
    assert!(result.contains("&self"), "should have &self receiver");
    assert!(result.contains("-> MyConfig"), "should have MyConfig return type");
    assert!(result.contains(".into()"), "should convert result back to MyConfig");
    assert!(!result.contains("compile_error!"), "should not emit compile_error");
}
