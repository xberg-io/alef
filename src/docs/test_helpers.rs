use crate::core::ir::{ApiSurface, CoreWrapper, DefaultValue, FieldDef, FunctionDef, MethodDef, TypeRef};

pub(crate) const TEST_PREFIX: &str = "Htm";

/// Crate name paired with [`TEST_PREFIX`]: `exception_class_name("htm")` is `HtmRsException`,
/// which is what the Java signature tests assert. ~keep
pub(crate) const TEST_CRATE_NAME: &str = "htm";

pub(crate) fn make_param(name: &str, ty: TypeRef, optional: bool) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
        name: name.to_string(),
        ty,
        optional,
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

/// A param the Rust source declared as `&T`, which `extract_params` records as `is_ref`. The
/// Rust reference page is the only surface that renders the borrow. ~keep
pub(crate) fn make_ref_param(name: &str, ty: TypeRef, optional: bool) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
        is_ref: true,
        ..make_param(name, ty, optional)
    }
}

pub(crate) fn make_method(
    name: &str,
    params: Vec<crate::core::ir::ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    is_static: bool,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        receiver: None,
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
    }
}

pub(crate) fn make_function(
    name: &str,
    params: Vec<crate::core::ir::ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    error_type: Option<&str>,
) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async,
        error_type: error_type.map(str::to_string),
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
    }
}

pub(crate) fn make_field(name: &str, ty: TypeRef, optional: bool, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default,
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
    }
}

pub(crate) fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

pub(crate) fn make_test_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

pub(crate) fn make_minimal_api(version: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".to_string(),
        version: version.to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}
