use super::*;
use crate::core::ir::{CoreWrapper, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn simple_method(name: &str, return_type: TypeRef, is_static: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        doc: String::new(),
        params: vec![],
        return_type,
        is_static,
        is_async: false,
        error_type: None,
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

fn simple_param(name: &str, ty: TypeRef) -> ParamDef {
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

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
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
    }
}

#[test]
fn test_gen_method_wrapper_opaque_free_method_emits_ptr_cast() {
    let typ = opaque_type("Client");
    let method = simple_method("close", TypeRef::Unit, false);
    let opaque: std::collections::HashSet<&str> = ["Client"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(
        out.contains("func (h *Client) Close("),
        "expected receiver+method in: {out}"
    );
    assert!(out.contains("unsafe.Pointer(h.ptr)"));
}

#[test]
fn test_gen_param_to_c_string_param_emits_cstring() {
    let param = simple_param("name", TypeRef::String);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);
    assert!(out.contains("C.CString("));
    assert!(out.contains("defer C.free("));
}

#[test]
fn test_gen_param_to_c_primitive_u64_emits_cgo_cast() {
    let param = simple_param("count", TypeRef::Primitive(PrimitiveType::U64));
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_param_to_c(&param, "", false, "krz", &opaque, &enum_names, &ffi_param_enum_names);
    assert!(out.contains("C.uint64_t("));
}

#[test]
fn test_gen_method_wrapper_non_opaque_static_emisample_package_func() {
    let mut typ = opaque_type("Config");
    typ.is_opaque = false;
    typ.fields = vec![simple_field("value", TypeRef::String)];
    let method = simple_method("default_value", TypeRef::String, true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains("func Config"));
}

#[test]
fn test_gen_method_wrapper_optional_string_getter_emits_nil_check_and_address() {
    let typ = opaque_type("GraphQLRouteConfig");
    let method = simple_method("get_description", TypeRef::Optional(Box::new(TypeRef::String)), false);
    let opaque: std::collections::HashSet<&str> = ["GraphQLRouteConfig"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "sample_router",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains(") *string {"), "expected *string return in:\n{out}");
    assert!(
        out.contains("if ptr == nil"),
        "missing nil check in optional-string getter body:\n{out}"
    );
    assert!(
        out.contains("return &s") || out.contains("return &result"),
        "missing take-address pattern in optional-string getter body:\n{out}"
    );
    assert!(
        !out.contains("\treturn C.GoString(ptr)\n"),
        "buggy bare `return C.GoString(ptr)` present:\n{out}"
    );
}

#[test]
fn test_gen_method_wrapper_bytes_result_emits_out_params() {
    let typ = opaque_type("Renderer");
    let method = MethodDef {
        name: "render_page".to_string(),
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
    };
    let opaque: std::collections::HashSet<&str> = ["Renderer"].into();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let ffi_param_enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_method_wrapper(
        &typ,
        &method,
        "krz",
        &opaque,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
    );
    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
    assert!(out.contains("outLen"), "missing outLen in:\n{out}");
    assert!(out.contains("outCap"), "missing outCap in:\n{out}");
    assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}
