use crate::backends::ffi::type_map::is_void_return;
use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;
use minijinja::context;

use super::super::helpers::null_return_value;
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

pub(super) fn gen_param_conversion_with_enums(
    param: &ParamDef,
    has_error: bool,
    is_bytes_result: bool,
    return_type: &TypeRef,
    core_import: &str,
    enum_names: &AHashSet<String>,
) -> String {
    let name = &param.name;
    let rs_name = format!("{name}_rs");
    let mut out = String::with_capacity(2048);

    let fail_ret = if is_bytes_result || (has_error && is_void_return(return_type)) {
        "return -1;"
    } else if is_void_return(return_type) {
        "return;"
    } else {
        match null_return_value(return_type) {
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
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        is_ref => param.is_ref,
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
            TypeRef::Primitive(prim) => match prim {
                crate::core::ir::PrimitiveType::Bool => {
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "param_primitive_bool.jinja",
                        context! { rs_name => rs_name.clone(), name => name.clone() },
                    ));
                }
                _ => {
                    if let Some(newtype_path) = &param.newtype_wrapper {
                        out.push_str(&crate::backends::ffi::template_env::render("param_primitive_newtype.jinja", context! { rs_name => rs_name.clone(), newtype_path => newtype_path.clone(), name => name.clone() }));
                    } else {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "param_primitive_passthrough.jinja",
                            context! { rs_name => rs_name.clone(), name => name.clone() },
                        ));
                    }
                }
            },
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
            TypeRef::Named(_type_name) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "param_non_optional_named_conversion.jinja",
                    context! {
                        rs_name => rs_name.clone(),
                        name => name.clone(),
                        fail_ret => fail_ret.to_string(),
                        is_ref => param.is_ref,
                        is_mut => param.is_mut,
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

    out
}
