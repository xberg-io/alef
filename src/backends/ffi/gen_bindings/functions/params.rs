use crate::backends::ffi::type_map::is_void_return;
use crate::core::ir::{NewtypeContainer, NewtypeConversion, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

use super::super::helpers::ffi_null_return_value;
use super::signatures::c_symbol_component;

fn type_ref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Primitive(prim) => match prim {
            crate::core::ir::PrimitiveType::Bool => "bool".to_string(),
            crate::core::ir::PrimitiveType::U8 => "u8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "u16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "u32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "u64".to_string(),
            crate::core::ir::PrimitiveType::I8 => "i8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "i16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "i32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "i64".to_string(),
            crate::core::ir::PrimitiveType::F32 => "f32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "f64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "usize".to_string(),
            crate::core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Named(name) => format!("{core_import}::{name}"),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_rust_type(inner, core_import)),
        TypeRef::Map(key, val) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_rust_type(key, core_import),
            type_ref_to_rust_type(val, core_import)
        ),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_rust_type(inner, core_import)),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
        TypeRef::Unit => "()".to_string(),
    }
}

pub(in crate::backends::ffi::gen_bindings) struct ParamConversionContext<'a> {
    pub(in crate::backends::ffi::gen_bindings) has_error: bool,
    pub(in crate::backends::ffi::gen_bindings) is_bytes_result: bool,
    pub(in crate::backends::ffi::gen_bindings) return_type: &'a TypeRef,
    pub(in crate::backends::ffi::gen_bindings) ffi_return_type: Option<&'a str>,
    pub(in crate::backends::ffi::gen_bindings) core_import: &'a str,
    pub(in crate::backends::ffi::gen_bindings) path_map: &'a AHashMap<String, String>,
    pub(in crate::backends::ffi::gen_bindings) enum_names: &'a AHashSet<String>,
}

fn root_optional_transparent_constructor(wrapper: &str) -> Option<String> {
    let decoded = crate::core::ir::NewtypeWrapper::decode(wrapper).ok()?;
    let [metadata] = decoded.explicit_paths() else {
        return None;
    };
    if metadata.containers.as_slice() != [NewtypeContainer::Optional] {
        return None;
    }
    let NewtypeConversion::TransparentString { from, .. } = &metadata.conversion else {
        return None;
    };
    Some(format!("{}::{from}", metadata.rust_path))
}

fn transparent_newtype_shadow(param: &ParamDef, rs_name: &str, core_import: &str) -> String {
    let Some(wrapper) = param
        .newtype_wrapper
        .as_deref()
        .filter(|wrapper| crate::codegen::conversions::helpers::is_explicit_newtype(wrapper))
    else {
        return String::new();
    };
    let converted = root_optional_transparent_constructor(wrapper).map_or_else(
        || {
            crate::codegen::conversions::helpers::apply_field_newtype_to_core(
                rs_name,
                &param.ty,
                param.optional,
                wrapper,
            )
        },
        |constructor| format!("({rs_name}).map({constructor})"),
    );
    let target_type = transparent_newtype_core_type(param, wrapper, core_import);
    format!("    let {rs_name}: {target_type} = {converted};\n")
}

pub(in crate::backends::ffi::gen_bindings) fn param_has_explicit_newtype(param: &ParamDef) -> bool {
    param
        .newtype_wrapper
        .as_deref()
        .is_some_and(crate::codegen::conversions::helpers::is_explicit_newtype)
}

pub(in crate::backends::ffi::gen_bindings) fn optional_borrowed_param_call_arg(
    param: &ParamDef,
    rs_name: &str,
) -> String {
    if param_has_explicit_newtype(param) {
        format!("{rs_name}.as_ref()")
    } else {
        format!("{rs_name}.as_deref()")
    }
}

fn transparent_newtype_core_type(param: &ParamDef, wrapper: &str, core_import: &str) -> String {
    let decoded =
        crate::core::ir::NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated during extraction");
    if param.optional {
        let path = vec![crate::core::ir::NewtypeContainer::Optional];
        let inner = render_newtype_core_type(&param.ty, &path, param, decoded.explicit_paths(), core_import, true);
        format!("Option<{inner}>")
    } else {
        render_newtype_core_type(&param.ty, &[], param, decoded.explicit_paths(), core_import, true)
    }
}

fn render_newtype_core_type(
    ty: &TypeRef,
    path: &[crate::core::ir::NewtypeContainer],
    param: &ParamDef,
    metadata: &[crate::core::ir::NewtypeWrapperMetadata],
    core_import: &str,
    outer: bool,
) -> String {
    if let Some(wrapper) = metadata.iter().find(|item| item.containers.as_slice() == path) {
        return wrapper.rust_path.clone();
    }
    match ty {
        TypeRef::Optional(inner) => {
            let mut inner_path = path.to_vec();
            inner_path.push(crate::core::ir::NewtypeContainer::Optional);
            format!(
                "Option<{}>",
                render_newtype_core_type(inner, &inner_path, param, metadata, core_import, false)
            )
        }
        TypeRef::Vec(inner) => {
            let mut inner_path = path.to_vec();
            inner_path.push(crate::core::ir::NewtypeContainer::Vec);
            format!(
                "Vec<{}>",
                render_newtype_core_type(inner, &inner_path, param, metadata, core_import, false)
            )
        }
        TypeRef::Map(key, value) => {
            let mut key_path = path.to_vec();
            key_path.push(crate::core::ir::NewtypeContainer::MapKey);
            let mut value_path = path.to_vec();
            value_path.push(crate::core::ir::NewtypeContainer::MapValue);
            let key_type = if outer
                && param.map_key_is_cow
                && !metadata
                    .iter()
                    .any(|item| item.containers.as_slice() == key_path.as_slice())
            {
                "std::borrow::Cow<'static, str>".to_string()
            } else {
                render_newtype_core_type(key, &key_path, param, metadata, core_import, false)
            };
            let value_type = render_newtype_core_type(value, &value_path, param, metadata, core_import, false);
            let collection = if outer && param.map_is_ahash {
                "ahash::AHashMap"
            } else if outer && param.map_is_btree {
                "std::collections::BTreeMap"
            } else {
                "std::collections::HashMap"
            };
            format!("{collection}<{key_type}, {value_type}>")
        }
        _ => type_ref_to_rust_type(ty, core_import),
    }
}

pub(in crate::backends::ffi::gen_bindings) fn gen_param_conversion_with_enums(
    param: &ParamDef,
    conversion: &ParamConversionContext<'_>,
) -> String {
    let ParamConversionContext {
        has_error,
        is_bytes_result,
        return_type,
        ffi_return_type,
        core_import,
        path_map,
        enum_names,
    } = conversion;
    let name = &param.name;
    let rs_name = format!("{name}_rs");
    let mut out = String::with_capacity(2048);

    let fail_ret = if *is_bytes_result || (*has_error && is_void_return(return_type)) {
        "return -1;"
    } else if is_void_return(return_type) {
        "return;"
    } else {
        match ffi_null_return_value(return_type, *ffi_return_type) {
            "()" => "return;",
            v => {
                let ret = format!("return {};", v);
                Box::leak(ret.into_boxed_str()) as &str
            }
        }
    };

    if param.optional {
        match &param.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_string_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Path => {
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_path_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Json => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_json_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        turbofish => String::new(),
                    },
                ));
            }
            TypeRef::Named(type_name) if enum_names.contains(type_name.as_str()) => {
                let enum_snake = c_symbol_component(type_name);
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_enum_discriminant_match.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        enum_snake => enum_snake,
                        name => name.clone(),
                        error_message => format!("invalid enum discriminant for {type_name}"),
                        fail_ret => fail_ret,
                    },
                ));
            }
            TypeRef::Named(type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                        qualified => path_map.get(type_name).filter(|path| !path.is_empty()).cloned().unwrap_or_else(|| format!("{core_import}::{type_name}")),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                let TypeRef::Named(type_name) = inner.as_ref() else {
                    unreachable!("guarded by match condition")
                };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
                        qualified => path_map.get(type_name).filter(|path| !path.is_empty()).cloned().unwrap_or_else(|| format!("{core_import}::{type_name}")),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_bool_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                    },
                ));
            }
            TypeRef::Primitive(prim) => {
                let max_val = match prim {
                    crate::core::ir::PrimitiveType::U8 => "u8::MAX",
                    crate::core::ir::PrimitiveType::U16 => "u16::MAX",
                    crate::core::ir::PrimitiveType::U32 => "u32::MAX",
                    crate::core::ir::PrimitiveType::U64 => "u64::MAX",
                    crate::core::ir::PrimitiveType::I8 => "i8::MAX",
                    crate::core::ir::PrimitiveType::I16 => "i16::MAX",
                    crate::core::ir::PrimitiveType::I32 => "i32::MAX",
                    crate::core::ir::PrimitiveType::I64 => "i64::MAX",
                    crate::core::ir::PrimitiveType::F32 => "f32::NAN",
                    crate::core::ir::PrimitiveType::F64 => "f64::NAN",
                    crate::core::ir::PrimitiveType::Usize => "usize::MAX",
                    crate::core::ir::PrimitiveType::Isize => "isize::MAX",
                    crate::core::ir::PrimitiveType::Bool => unreachable!("handled above"),
                };
                let is_float = matches!(
                    prim,
                    crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64
                );
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_numeric_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        max_val => max_val,
                        is_float => is_float,
                    },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                let type_hint = match &param.ty {
                    TypeRef::Vec(_) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    TypeRef::Map(_, val_ty) if param.map_is_ahash => {
                        let val_rust = type_ref_to_rust_type(val_ty, core_import);
                        let key_rust = if param.map_key_is_cow {
                            "std::borrow::Cow<'static, str>".to_string()
                        } else {
                            "String".to_string()
                        };
                        format!("::<ahash::AHashMap<{key_rust}, {val_rust}>>")
                    }
                    TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push(' ');
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_vec_map_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        turbofish => type_hint,
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_bytes_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            _ => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_fallback.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
        }
    } else {
        match &param.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_string_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                    },
                ));
            }
            TypeRef::Path => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_path_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Json => {
                let turbofish = String::new();
                let mut_keyword = String::new();
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_json_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                        turbofish => turbofish,
                        mut_keyword => mut_keyword,
                    },
                ));
            }
            TypeRef::Primitive(prim) => {
                match prim {
                    crate::core::ir::PrimitiveType::Bool => {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "param_primitive_bool.jinja",
                            context! { rs_name => rs_name.clone(), name => name.clone() },
                        ));
                    }
                    _ => {
                        if let Some(newtype_path) = param
                            .newtype_wrapper
                            .as_deref()
                            .filter(|wrapper| !crate::codegen::conversions::helpers::is_explicit_newtype(wrapper))
                        {
                            out.push_str(&crate::backends::ffi::template_env::render("param_primitive_newtype.jinja", context! { rs_name => rs_name.clone(), newtype_path => newtype_path, name => name.clone() }));
                        } else {
                            out.push_str(&crate::backends::ffi::template_env::render(
                                "param_primitive_passthrough.jinja",
                                context! { rs_name => rs_name.clone(), name => name.clone() },
                            ));
                        }
                    }
                }
            }
            TypeRef::Named(type_name) if enum_names.contains(type_name.as_str()) => {
                let enum_snake = c_symbol_component(type_name);
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_enum_discriminant_match.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        enum_snake => enum_snake,
                        name => name.clone(),
                        error_message => format!("invalid enum discriminant for {type_name}"),
                        fail_ret => fail_ret,
                    },
                ));
            }
            TypeRef::Named(type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        is_ref => param.is_ref,
                        is_mut => param.is_mut,
                        qualified => path_map.get(type_name).filter(|path| !path.is_empty()).cloned().unwrap_or_else(|| format!("{core_import}::{type_name}")),
                    },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_bytes_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                    },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                let mut_keyword = if param.is_mut { "mut " } else { "" };
                let type_hint = match &param.ty {
                    TypeRef::Vec(_) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    TypeRef::Map(_, val_ty) if param.map_is_ahash => {
                        let val_rust = type_ref_to_rust_type(val_ty, core_import);
                        let key_rust = if param.map_key_is_cow {
                            "std::borrow::Cow<'static, str>".to_string()
                        } else {
                            "String".to_string()
                        };
                        format!("::<ahash::AHashMap<{key_rust}, {val_rust}>>")
                    }
                    TypeRef::Map(_, _) => {
                        format!("::<{}>", type_ref_to_rust_type(&param.ty, core_import))
                    }
                    _ => String::new(),
                };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_json_conversion.jinja",
                    context! {
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        rs_name => rs_name.clone(),
                        turbofish => type_hint,
                        mut_keyword => mut_keyword,
                    },
                ));
            }
            TypeRef::Optional(_) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_passthrough.jinja",
                    context! { rs_name => rs_name.clone(), name => name.clone() },
                ));
            }
            TypeRef::Duration => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_duration_conversion.jinja",
                    context! { rs_name => rs_name.clone(), name => name.clone() },
                ));
            }
            TypeRef::Unit => {}
        }
    }

    out.push_str(&transparent_newtype_shadow(param, &rs_name, core_import));

    out
}
