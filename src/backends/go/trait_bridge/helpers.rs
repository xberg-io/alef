use crate::backends::go::type_map::go_type;
use crate::core::ir::{MethodDef, TypeRef};
use std::collections::HashSet;

/// Recursively substitute `TypeRef::Named(n)` references for types excluded from the binding's
/// public surface with `TypeRef::Json`. For Go this lets trait-bridge interface signatures and
/// trampolines fall back to `json.RawMessage`, since the named Go type was never emitted into
/// `binding.go` and would otherwise produce `undefined: <Name>` build errors. Shared with the
/// other backends — see [`crate::codegen::shared::substitute_excluded_types`].
pub(super) use crate::codegen::shared::substitute_excluded_types;

/// Clone a `MethodDef`, substituting any excluded named-type references in its
/// parameters and return type with `TypeRef::Json`. See [`substitute_excluded_types`].
pub(super) fn method_with_excluded_substituted(method: &MethodDef, excluded: &HashSet<&str>) -> MethodDef {
    let mut m = method.clone();
    for p in &mut m.params {
        p.ty = substitute_excluded_types(&p.ty, excluded);
    }
    m.return_type = substitute_excluded_types(&m.return_type, excluded);
    m
}
/// Build the C trampoline function signature for extern declaration in the CGo preamble.
/// Uses actual C types (not Go CGo types like `C.int32_t`).
///
/// For simple primitives (bool, i32, etc.), the function returns the value directly
/// and does not use an out_result parameter. For complex types, uses the out_result + out_error pattern.
#[allow(dead_code)]
pub(super) fn c_trampoline_signature(_export_name: &str, method: &MethodDef) -> String {
    let mut params = vec!["void* user_data".to_string()];
    for p in &method.params {
        let cty = rust_to_plain_c_type(&p.ty);
        params.push(format!("{} {}", cty, p.name));
        if matches!(p.ty, TypeRef::Bytes) {
            params.push(format!("size_t {}_len", p.name));
        }
    }

    let is_simple_primitive = matches!(
        &method.return_type,
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize)
    );

    if is_simple_primitive {
        params.push("char** out_error".to_string());
    } else if !matches!(method.return_type, TypeRef::Unit) {
        params.push("char** out_result".to_string());
        params.push("char** out_error".to_string());
    } else {
        params.push("char** out_error".to_string());
    }

    params.join(", ")
}

/// Determine the C return type for a callback function.
/// Simple primitives return their value directly. Complex types return int32_t (status code).
#[allow(dead_code)]
pub(super) fn c_callback_return_type(method: &MethodDef) -> String {
    let is_simple_primitive = matches!(
        &method.return_type,
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::I64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::U64)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize)
            | TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize)
    );

    if is_simple_primitive {
        match &method.return_type {
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "int32_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I32) => "int32_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U32) => "uint32_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::I64) => "int64_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64) => "uint64_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize) => "size_t".to_string(),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Isize) => "intptr_t".to_string(),
            _ => "int32_t".to_string(),
        }
    } else {
        "int32_t".to_string()
    }
}

/// Convert a Rust TypeRef to a plain C type string (for CGo preamble extern declarations).
#[allow(dead_code)]
fn rust_to_plain_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType::*;
            match p {
                Bool => "int32_t",
                U8 => "uint8_t",
                U16 => "uint16_t",
                U32 => "uint32_t",
                U64 => "uint64_t",
                I8 => "int8_t",
                I16 => "int16_t",
                I32 => "int32_t",
                I64 => "int64_t",
                F32 => "float",
                F64 => "double",
                Usize => "size_t",
                Isize => "intptr_t",
            }
            .to_string()
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => "char*".to_string(),
        TypeRef::Bytes => "uint8_t*".to_string(),
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) => "char*".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Duration => "uint64_t".to_string(),
        _ => "char*".to_string(),
    }
}

/// Convert a Rust TypeRef to a Go type string.
/// Uses the type_map module for consistent type resolution, which handles Named types correctly.
pub(super) fn rust_to_go_type(ty: &TypeRef) -> String {
    go_type(ty).into_owned()
}

/// Convert a Rust TypeRef to a C type string.
pub(super) fn rust_to_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType::*;
            match p {
                Bool => "C.int32_t",
                U8 => "C.uint8_t",
                U16 => "C.uint16_t",
                U32 => "C.uint32_t",
                U64 => "C.uint64_t",
                I8 => "C.int8_t",
                I16 => "C.int16_t",
                I32 => "C.int32_t",
                I64 => "C.int64_t",
                F32 => "C.float",
                F64 => "C.double",
                Usize => "C.size_t",
                Isize => "C.intptr_t",
            }
            .to_string()
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => "*C.char".to_string(),
        TypeRef::Bytes => "*C.uint8_t".to_string(),
        TypeRef::Optional(_) => "*C.char".to_string(),
        TypeRef::Vec(_) => "*C.char".to_string(),
        TypeRef::Map(_, _) => "*C.char".to_string(),
        TypeRef::Unit => "C.void".to_string(),
        TypeRef::Duration => "C.uint64_t".to_string(),
        TypeRef::Named(_) => "*C.char".to_string(),
        _ => "*C.char".to_string(),
    }
}

/// Generate parameter conversion code (C to Go).
pub(super) fn gen_param_conversion(out: &mut String, param: &crate::core::ir::ParamDef) {
    let var_name = format!("go{}", capitalize(&param.name));
    match &param.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            out.push_str(&crate::backends::go::template_env::render(
                "go_string_cast.jinja",
                minijinja::context! {
                    name => capitalize(&param.name),
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
        }
        TypeRef::Bytes => {
            let name = &param.name;
            let len_name = format!("{name}Len");
            out.push_str(&crate::backends::go::template_env::render(
                "trampoline_bytes_param_decode.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    name => name,
                    len_name => &len_name,
                },
            ));
        }
        TypeRef::Vec(_) => {
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "json_unmarshal_simple.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                    var_name => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Named(_) => {
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "json_unmarshal_simple.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                    var_name => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Map(_, _) => {
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str("\t\tvar rawData interface{}\n");
            out.push_str(&crate::backends::go::template_env::render(
                "json_unmarshal_rawdata.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push_str("\t\tif m, ok := rawData.(map[string]interface{}); ok {\n");
            out.push_str(&crate::backends::go::template_env::render(
                "var_assign_m.jinja",
                minijinja::context! {
                    var => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t\t}\n");
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Optional(_) => {
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str("\t\tvar rawData interface{}\n");
            out.push_str(&crate::backends::go::template_env::render(
                "json_unmarshal_rawdata.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push_str("\t\tif m, ok := rawData.(map[string]interface{}); ok {\n");
            out.push_str(&crate::backends::go::template_env::render(
                "var_assign_m.jinja",
                minijinja::context! {
                    var => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t\t}\n");
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Json => {
            out.push_str(&crate::backends::go::template_env::render(
                "trampoline_raw_message_decode.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    param => param.name.as_str(),
                },
            ));
        }
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType::*;
            let cast = match p {
                Bool => format!("{} != 0", param.name),
                _ => {
                    let go_type = match p {
                        U8 => "uint8",
                        U16 => "uint16",
                        U32 => "uint32",
                        U64 => "uint64",
                        I8 => "int8",
                        I16 => "int16",
                        I32 => "int32",
                        I64 => "int64",
                        F32 => "float32",
                        F64 => "float64",
                        Usize => "uint",
                        Isize => "int",
                        _ => "",
                    };
                    format!("{}({})", go_type, param.name)
                }
            };
            out.push_str(&crate::backends::go::template_env::render(
                "var_assign_cast.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    cast => &cast,
                },
            ));
            out.push('\n');
            out.push('\n');
        }
        _ => {
            out.push_str(&crate::backends::go::template_env::render(
                "var_assign_cast.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    cast => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push('\n');
        }
    }
}

/// Capitalize the first character of a string.
pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
