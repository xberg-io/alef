use alef_core::ir::{ApiSurface, CoreWrapper, DefaultValue, FieldDef, FunctionDef, MethodDef, TypeRef};

pub(crate) const TEST_PREFIX: &str = "Htm";

pub(crate) fn make_param(name: &str, ty: TypeRef, optional: bool) -> alef_core::ir::ParamDef {
    alef_core::ir::ParamDef {
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
    }
}

pub(crate) fn make_method(
    name: &str,
    params: Vec<alef_core::ir::ParamDef>,
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
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

pub(crate) fn make_function(
    name: &str,
    params: Vec<alef_core::ir::ParamDef>,
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
    }
}

pub(crate) fn make_field(name: &str, ty: TypeRef, optional: bool, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
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
    }
}

pub(crate) fn make_test_config() -> alef_core::config::ResolvedCrateConfig {
    let cfg: alef_core::config::NewAlefConfig = toml::from_str(
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
    }
}
