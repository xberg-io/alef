use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashSet;

use super::render;
use crate::backends::zig::gen_bindings::functions::optional_int_sentinel;

pub(super) fn method_param_needs_alloc(p: &ParamDef) -> bool {
    let inner = match &p.ty {
        TypeRef::Optional(t) => t.as_ref(),
        other => other,
    };
    matches!(
        inner,
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_)
    )
}

pub(super) fn method_param_needs_from_json(p: &ParamDef, struct_names: &HashSet<String>) -> bool {
    match &p.ty {
        TypeRef::Named(n) if struct_names.contains(n) => true,
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if struct_names.contains(n)),
        _ => false,
    }
}

/// Map a Rust FFI type string to the corresponding Zig type.
///
/// Only the types actually used in `client_constructors` configs are handled here.
pub(super) fn ffi_ty_to_zig(rust_ty: &str) -> &'static str {
    let normalized = rust_ty.trim();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return "[]const u8";
    }
    if matches!(normalized, "u8" | "uint8_t") {
        return "u8";
    }
    if matches!(normalized, "u16" | "uint16_t") {
        return "u16";
    }
    if matches!(normalized, "u32" | "uint32_t") {
        return "u32";
    }
    if matches!(normalized, "u64" | "uint64_t" | "usize") {
        return "u64";
    }
    if matches!(normalized, "i8" | "int8_t") {
        return "i8";
    }
    if matches!(normalized, "i16" | "int16_t") {
        return "i16";
    }
    if matches!(normalized, "i32" | "int32_t" | "c_int") {
        return "i32";
    }
    if matches!(normalized, "i64" | "int64_t" | "isize") {
        return "i64";
    }
    if matches!(normalized, "bool") {
        return "bool";
    }
    if matches!(normalized, "f32" | "float") {
        return "f32";
    }
    if matches!(normalized, "f64" | "double") {
        return "f64";
    }
    "*anyopaque"
}

/// Returns true if a Rust FFI type is a string/CStr pointer that needs
/// sentinel-copy conversion before passing to the C function.
pub(super) fn ffi_ty_needs_dupez(rust_ty: &str) -> bool {
    let normalized = rust_ty.trim();
    normalized.contains("c_char") || normalized.contains("CStr")
}

/// Zig type for a method parameter, including enum marshalling.
pub(super) fn param_zig_type_with_enums(
    ty: &TypeRef,
    optional: bool,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> String {
    let inner = match ty {
        TypeRef::Named(name) if enum_names.contains(name) => name.clone(),
        TypeRef::String | TypeRef::Path | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]const u8".to_string()
        }
        TypeRef::Named(name) if struct_names.contains(name) => "[]const u8".to_string(),
        TypeRef::Optional(inner) => {
            let inner_str = param_zig_type_with_enums(inner, false, struct_names, enum_names);
            return format!("?{inner_str}");
        }
        other => crate::backends::zig::gen_bindings::types::zig_field_type(other, false),
    };
    if optional { format!("?{inner}") } else { inner }
}

/// Emit allocation/conversion lines for a static method parameter before the C call.
pub(super) fn emit_static_method_param_conversion(
    p: &ParamDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    let name = &p.name;

    if let TypeRef::Named(type_name) = &p.ty
        && enum_names.contains(type_name)
    {
        out.push_str(&render(
            "opaque_param_enum_i32.jinja",
            crate::alef_context! {
                indent => "    ",
                name => name,
            },
        ));
        return;
    }

    if matches!(&p.ty, TypeRef::String | TypeRef::Path) {
        out.push_str(&render(
            "opaque_param_dupez.jinja",
            crate::alef_context! {
                indent => "    ",
                name => name,
            },
        ));
        return;
    }

    if p.optional
        && matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        )
    {
        out.push_str(&render(
            "opaque_param_optional_dupez.jinja",
            crate::alef_context! {
                indent => "    ",
                name => name,
                capture => "s",
            },
        ));
        return;
    }

    if let TypeRef::Named(n) = &p.ty
        && struct_names.contains(n)
    {
        let snake = AsSnakeCase(n).to_string();
        out.push_str(&render(
            "opaque_param_named_from_json.jinja",
            crate::alef_context! {
                indent => "    ",
                name => name,
                prefix => prefix,
                snake => &snake,
                json_error_return => "return error.InvalidJson;",
            },
        ));
        return;
    }

    if let TypeRef::Optional(inner) = &p.ty
        && let TypeRef::Named(n) = inner.as_ref()
        && struct_names.contains(n)
    {
        let snake = AsSnakeCase(n).to_string();
        out.push_str(&render(
            "opaque_param_optional_named_from_json.jinja",
            crate::alef_context! {
                indent => "    ",
                name => name,
                prefix => prefix,
                snake => &snake,
                json_error_return => "return error.InvalidJson;",
            },
        ));
    }
}

/// Build the C argument name(s) for a static method parameter.
pub(super) fn static_method_c_arg_names(
    p: &ParamDef,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> Vec<String> {
    if let TypeRef::Named(type_name) = &p.ty
        && enum_names.contains(type_name)
    {
        return vec![format!("{}_i32", p.name)];
    }

    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if optional_named.is_some() {
        return vec![format!("{}_handle", p.name)];
    }

    if let TypeRef::Named(n) = &p.ty
        && struct_names.contains(n.as_str())
    {
        return vec![format!("{}_handle", p.name)];
    }

    if p.optional
        && matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        )
    {
        return vec![format!("if ({0}_z) |z| z.ptr else null", p.name)];
    }

    if matches!(
        &p.ty,
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _)
    ) {
        return vec![format!("{}_z.ptr", p.name)];
    }

    if matches!(p.ty, TypeRef::Bytes) {
        return vec![format!("{}.ptr", p.name), format!("{}.len", p.name)];
    }

    vec![p.name.clone()]
}

pub(super) fn emit_method_param_conversion(
    p: &ParamDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
    json_error_return: &str,
    out: &mut String,
) {
    let name = &p.name;

    if let TypeRef::Named(type_name) = &p.ty
        && enum_names.contains(type_name)
    {
        out.push_str(&render(
            "opaque_param_enum_i32.jinja",
            crate::alef_context! {
                indent => "        ",
                name => name,
            },
        ));
        return;
    }

    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );

    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        out.push_str(&render(
            "opaque_param_optional_dupez.jinja",
            crate::alef_context! {
                indent => "        ",
                name => name,
                capture => "s",
            },
        ));
        return;
    }

    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if let Some(n) = optional_named {
        let snake = AsSnakeCase(n).to_string();
        out.push_str(&render(
            "opaque_param_optional_named_from_json.jinja",
            crate::alef_context! {
                indent => "        ",
                name => name,
                prefix => prefix,
                snake => &snake,
                json_error_return => json_error_return,
            },
        ));
        return;
    }

    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str(&render(
                "opaque_param_dupez.jinja",
                crate::alef_context! {
                    indent => "        ",
                    name => name,
                },
            ));
        }
        TypeRef::Named(n) if struct_names.contains(n) => {
            let snake = AsSnakeCase(n).to_string();
            out.push_str(&render(
                "opaque_param_named_from_json.jinja",
                crate::alef_context! {
                    indent => "        ",
                    name => name,
                    prefix => prefix,
                    snake => &snake,
                    json_error_return => json_error_return,
                },
            ));
        }
        TypeRef::Optional(inner) => {
            if let TypeRef::Vec(_) | TypeRef::Map(_, _) = inner.as_ref() {
                out.push_str(&render(
                    "opaque_param_optional_dupez.jinja",
                    crate::alef_context! {
                        indent => "        ",
                        name => name,
                        capture => "v",
                    },
                ));
            }
        }
        _ => {}
    }
}

/// Free allocations made in `emit_method_param_conversion`.
pub(super) fn emit_method_param_free(p: &ParamDef, struct_names: &HashSet<String>) {
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );

    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        return;
    }

    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if optional_named.is_some() {
        return;
    }

    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {}
        TypeRef::Named(n) if struct_names.contains(n) => {}
        TypeRef::Optional(_) => {}
        _ => {}
    }
}

/// Build the C argument name(s) for a method parameter.
pub(super) fn method_c_arg_names(
    p: &ParamDef,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> Vec<String> {
    if let TypeRef::Named(type_name) = &p.ty
        && enum_names.contains(type_name)
    {
        return vec![format!("{}_i32", p.name)];
    }

    let optional_named: Option<&str> = match &p.ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if struct_names.contains(n) => Some(n.as_str()),
            _ => None,
        },
        TypeRef::Named(n) if p.optional && struct_names.contains(n) => Some(n.as_str()),
        _ => None,
    };
    if optional_named.is_some() {
        return vec![format!("{}_handle", p.name)];
    }
    if let TypeRef::Named(n) = &p.ty
        && struct_names.contains(n.as_str())
    {
        return vec![format!("{}_handle", p.name)];
    }

    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string
        && matches!(
            match &p.ty {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            },
            TypeRef::String | TypeRef::Path
        )
    {
        return vec![format!("if ({0}_z) |z| z.ptr else null", p.name)];
    }

    let prim_opt: Option<&PrimitiveType> = match &p.ty {
        TypeRef::Optional(inner) => {
            if let TypeRef::Primitive(prim) = inner.as_ref() {
                Some(prim)
            } else {
                None
            }
        }
        TypeRef::Primitive(prim) if p.optional => Some(prim),
        _ => None,
    };
    if let Some(prim) = prim_opt
        && let Some(sentinel) = optional_int_sentinel(prim)
    {
        return vec![format!("if ({name}) |v| v else {sentinel}", name = p.name)];
    }

    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            vec![format!("{}_z", p.name)]
        }
        TypeRef::Bytes => vec![format!("{}.ptr", p.name), format!("{}.len", p.name)],
        _ => vec![p.name.clone()],
    }
}
