use super::*;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

#[test]
fn test_params_require_marshal_for_named_non_opaque() {
    let params = vec![make_param("options", TypeRef::Named("Config".to_string()))];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

#[test]
fn test_params_require_marshal_false_for_opaque() {
    let params = vec![make_param("client", TypeRef::Named("Client".to_string()))];
    let opaque: std::collections::HashSet<&str> = ["Client"].into();
    assert!(!params_require_marshal(&params, &opaque));
}

#[test]
fn test_is_bridge_param_matches_by_name() {
    let param = make_param("visitor", TypeRef::Named("VisitorHandle".to_string()));
    let bridge_names: HashSet<String> = ["visitor".to_string()].into();
    let aliases: HashSet<String> = HashSet::new();
    assert!(is_bridge_param(&param, &bridge_names, &aliases));
}

#[test]
fn test_params_require_marshal_for_vec() {
    let params = vec![make_param(
        "items",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
    )];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

fn make_bytes_result_func(name: &str, with_bytes_param: bool) -> FunctionDef {
    let params = if with_bytes_param {
        vec![ParamDef {
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }]
    } else {
        vec![]
    };
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params,
        return_type: TypeRef::Bytes,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
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

fn make_bytes_result_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        doc: String::new(),
        params: vec![ParamDef {
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Bytes,
        is_static: false,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
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
    }
}

#[test]
fn test_is_bytes_result_func_detects_bytes_with_error() {
    let func = make_bytes_result_func("process_image", true);
    assert!(is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_func_false_for_bytes_without_error() {
    let mut func = make_bytes_result_func("get_data", false);
    func.error_type = None;
    assert!(!is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_func_false_for_string_with_error() {
    let func = FunctionDef {
        name: "get_text".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
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
    };
    assert!(!is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_method_detects_correctly() {
    let method = make_bytes_result_method("render_page");
    assert!(is_bytes_result_method(&method));
}

#[test]
fn test_gen_function_wrapper_bytes_result_emits_out_params() {
    let func = make_bytes_result_func("process_image", true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let bridge_names: HashSet<String> = HashSet::new();
    let bridge_aliases: HashSet<String> = HashSet::new();
    let value_only_types: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let ffi_param_enum_names: HashSet<String> = HashSet::new();
    let reserved_type_names: HashSet<String> = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &bridge_names,
        &bridge_aliases,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
        &reserved_type_names,
    );
    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
    assert!(out.contains("outLen"), "missing outLen in:\n{out}");
    assert!(out.contains("outCap"), "missing outCap in:\n{out}");
    assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
    assert!(out.contains("&outLen"), "missing &outLen in:\n{out}");
    assert!(out.contains("&outCap"), "missing &outCap in:\n{out}");
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}

fn make_capsule_func(name: &str, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![make_param("name", TypeRef::String)],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: if fallible {
            Some("SampleCrateError".to_string())
        } else {
            None
        },
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

fn capsule_cfg() -> crate::core::config::HostCapsuleTypeConfig {
    crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: "github.com/example/go-my-lib".to_string(),
        package_version: "v1.0.0".to_string(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
    }
}

#[test]
fn test_capsule_fallible_returns_error_tuple_and_checks_last_error() {
    let func = make_capsule_func("get_language", true);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        out.contains("(*my_pkg.Language, error)"),
        "fallible capsule must return (host, error):\n{out}"
    );
    assert!(
        out.contains("lastError()"),
        "fallible capsule must check lastError():\n{out}"
    );
    assert!(
        out.contains("return nil, err"),
        "fallible capsule must propagate the error:\n{out}"
    );
}

#[test]
fn test_capsule_infallible_returns_bare_host_type() {
    let func = make_capsule_func("builtin_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        !out.contains(", error)"),
        "infallible capsule must not return an error:\n{out}"
    );
    assert!(
        !out.contains("lastError()"),
        "infallible capsule must not check lastError():\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_construct_expr_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: String::new(),
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("construct_expr"),
        "error must mention the missing field. Got:\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_host_type_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: String::new(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must mention the missing field. Got:\n{out}"
    );
}
