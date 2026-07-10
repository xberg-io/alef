use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};
use std::collections::{HashMap, HashSet};

pub fn format_type_ref(ty: &crate::core::ir::TypeRef, type_paths: &HashMap<String, String>) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "u8",
            PrimitiveType::U16 => "u16",
            PrimitiveType::U32 => "u32",
            PrimitiveType::U64 => "u64",
            PrimitiveType::I8 => "i8",
            PrimitiveType::I16 => "i16",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::Usize => "usize",
            PrimitiveType::Isize => "isize",
        }
        .to_string(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Vec(inner) => format!("Vec<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            format_type_ref(k, type_paths),
            format_type_ref(v, type_paths)
        ),
        TypeRef::Named(name) => type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone()),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Format a return type, wrapping in `Result` when an error type is present.
///
/// When `returns_ref` is `true` and the IR type is `Vec(T)`, the trait method
/// actually returns `&[T]` (the IR collapses `&[T]` into `Vec<T>` + `returns_ref`
/// flag). This function emits the correct reference slice type in that case so the
/// generated bridge impl signature matches the actual trait definition.
///
/// For the FFI bridge the concrete element type of a `Vec<String>` with `returns_ref`
/// is `&str`, yielding a return type of `&[&str]`.
pub fn format_return_type(
    ty: &crate::core::ir::TypeRef,
    error_type: Option<&str>,
    type_paths: &HashMap<String, String>,
    returns_ref: bool,
) -> String {
    let inner = if returns_ref {
        if let crate::core::ir::TypeRef::Vec(elem) = ty {
            let elem_str = match elem.as_ref() {
                crate::core::ir::TypeRef::String => "&str".to_string(),
                crate::core::ir::TypeRef::Bytes => "&[u8]".to_string(),
                crate::core::ir::TypeRef::Named(name) => {
                    let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                    format!("&{qualified}")
                }
                other => format_type_ref(other, type_paths),
            };
            format!("&[{elem_str}]")
        } else {
            format_type_ref(ty, type_paths)
        }
    } else {
        format_type_ref(ty, type_paths)
    };
    match error_type {
        Some(err) => format!("std::result::Result<{inner}, {err}>"),
        None => inner,
    }
}

/// Format a parameter type, respecting `is_ref`, `is_mut`, and `optional` from the IR.
///
/// Unlike [`format_type_ref`], this function produces reference types when the
/// original Rust parameter was a `&T` or `&mut T`, and wraps in `Option<>` when
/// `param.optional` is true:
/// - `String + is_ref` → `&str`
/// - `String + is_ref + optional` → `Option<&str>`
/// - `Bytes + is_ref` → `&[u8]`
/// - `Path + is_ref` → `&std::path::Path`
/// - `Vec<T> + is_ref` → `&[T]`
/// - `Named(n) + is_ref` → `&{qualified_name}`
pub fn format_param_type(param: &ParamDef, type_paths: &HashMap<String, String>) -> String {
    use crate::core::ir::TypeRef;
    let base = if param.is_ref {
        let mutability = if param.is_mut { "mut " } else { "" };
        match &param.ty {
            TypeRef::String => format!("&{mutability}str"),
            TypeRef::Bytes => format!("&{mutability}[u8]"),
            TypeRef::Path => format!("&{mutability}std::path::Path"),
            TypeRef::Vec(inner) => format!("&{mutability}[{}]", format_type_ref(inner, type_paths)),
            TypeRef::Named(name) => {
                let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                format!("&{mutability}{qualified}")
            }
            TypeRef::Optional(inner) => {
                let inner_type_str = match inner.as_ref() {
                    TypeRef::String => format!("&{mutability}str"),
                    TypeRef::Bytes => format!("&{mutability}[u8]"),
                    TypeRef::Path => format!("&{mutability}std::path::Path"),
                    TypeRef::Vec(v) => format!("&{mutability}[{}]", format_type_ref(v, type_paths)),
                    TypeRef::Named(name) => {
                        let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                        format!("&{mutability}{qualified}")
                    }
                    other => format_type_ref(other, type_paths),
                };
                return format!("Option<{inner_type_str}>");
            }
            other => format_type_ref(other, type_paths),
        }
    } else {
        format_type_ref(&param.ty, type_paths)
    };

    if param.optional {
        format!("Option<{base}>")
    } else {
        base
    }
}

/// Like [`format_param_type`] but emits `<'_>` after named types whose Rust definition
/// carries a lifetime parameter (e.g. `NodeContext<'a>`).
///
/// `lifetime_type_names` is the set of type names (from `TypeDef.has_lifetime_params`)
/// that require a lifetime argument in the trait impl method signature so it matches
/// the original trait definition exactly.  Pass an empty set to get the same output
/// as `format_param_type`.
pub fn format_param_type_with_lifetimes(
    param: &ParamDef,
    type_paths: &HashMap<String, String>,
    lifetime_type_names: &HashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;
    let base = if param.is_ref {
        let mutability = if param.is_mut { "mut " } else { "" };
        match &param.ty {
            TypeRef::String => format!("&{mutability}str"),
            TypeRef::Bytes => format!("&{mutability}[u8]"),
            TypeRef::Path => format!("&{mutability}std::path::Path"),
            TypeRef::Vec(inner) => format!("&{mutability}[{}]", format_type_ref(inner, type_paths)),
            TypeRef::Named(name) => {
                let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                let qualified = if lifetime_type_names.contains(name.as_str()) {
                    format!("{qualified}<'_>")
                } else {
                    qualified
                };
                format!("&{mutability}{qualified}")
            }
            TypeRef::Optional(inner) => {
                let inner_type_str = match inner.as_ref() {
                    TypeRef::String => format!("&{mutability}str"),
                    TypeRef::Bytes => format!("&{mutability}[u8]"),
                    TypeRef::Path => format!("&{mutability}std::path::Path"),
                    TypeRef::Vec(v) => format!("&{mutability}[{}]", format_type_ref(v, type_paths)),
                    TypeRef::Named(name) => {
                        let qualified = type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone());
                        let qualified = if lifetime_type_names.contains(name.as_str()) {
                            format!("{qualified}<'_>")
                        } else {
                            qualified
                        };
                        format!("&{mutability}{qualified}")
                    }
                    other => format_type_ref(other, type_paths),
                };
                return format!("Option<{inner_type_str}>");
            }
            other => format_type_ref(other, type_paths),
        }
    } else {
        format_type_ref(&param.ty, type_paths)
    };

    if param.optional {
        format!("Option<{base}>")
    } else {
        base
    }
}

/// Map a Rust primitive to its type string.
pub fn prim(p: &PrimitiveType) -> &'static str {
    use PrimitiveType::*;
    match p {
        Bool => "bool",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        F32 => "f32",
        F64 => "f64",
        Usize => "usize",
        Isize => "isize",
    }
}

/// Map a `TypeRef` to its Rust source type string for use in trait bridge method
/// signatures. `ci` is the core import path (e.g. `"sample_core"`), `tp` maps
/// type names to fully-qualified paths.
pub fn bridge_param_type(ty: &TypeRef, ci: &str, is_ref: bool, tp: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Bytes if is_ref => "&[u8]".into(),
        TypeRef::Bytes => "Vec<u8>".into(),
        TypeRef::String if is_ref => "&str".into(),
        TypeRef::String => "String".into(),
        TypeRef::Path if is_ref => "&std::path::Path".into(),
        TypeRef::Path => "std::path::PathBuf".into(),
        TypeRef::Named(n) => {
            let qualified = tp.get(n).cloned().unwrap_or_else(|| format!("{ci}::{n}"));
            if is_ref { format!("&{qualified}") } else { qualified }
        }
        TypeRef::Vec(inner) => format!("Vec<{}>", bridge_param_type(inner, ci, false, tp)),
        TypeRef::Optional(inner) => format!("Option<{}>", bridge_param_type(inner, ci, false, tp)),
        TypeRef::Primitive(p) => prim(p).into(),
        TypeRef::Unit => "()".into(),
        TypeRef::Char => "char".into(),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            bridge_param_type(k, ci, false, tp),
            bridge_param_type(v, ci, false, tp)
        ),
        TypeRef::Json => "serde_json::Value".into(),
        TypeRef::Duration => "std::time::Duration".into(),
    }
}

/// Map a visitor method parameter type to the correct Rust type string, handling
/// IR quirks:
/// - `ty=String, optional=true, is_ref=true` → `Option<&str>` (IR collapses `Option<&str>`)
/// - `ty=Vec<T>, is_ref=true` → `&[T]` (IR collapses `&[T]`)
/// - Everything else delegates to [`bridge_param_type`].
pub fn visitor_param_type(ty: &TypeRef, is_ref: bool, optional: bool, tp: &HashMap<String, String>) -> String {
    if optional && matches!(ty, TypeRef::String) && is_ref {
        return "Option<&str>".to_string();
    }
    if is_ref {
        if let TypeRef::Vec(inner) = ty {
            let inner_str = bridge_param_type(inner, "", false, tp);
            return format!("&[{inner_str}]");
        }
    }
    bridge_param_type(ty, "", is_ref, tp)
}

pub fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Signature parts for one bridge trait-impl method, shared between the wrapper's
/// trait impl and the default-delegate impls so both emit identical signatures.
pub struct TraitMethodSig {
    /// `"async "` or `""`.
    pub async_kw: &'static str,
    /// Full parameter list including the receiver, e.g. `"&self, path: &std::path::Path"`.
    pub all_params: String,
    /// Formatted return type (crate error type substituted).
    pub ret: String,
    /// Comma-joined argument names for forwarding calls, e.g. `"path, config"`.
    pub arg_names: String,
}

/// Build the Rust signature parts for a trait method as the bridge emits it.
pub fn trait_method_signature(method: &crate::core::ir::MethodDef, spec: &super::TraitBridgeSpec) -> TraitMethodSig {
    let async_kw = if method.is_async { "async " } else { "" };
    let receiver = match &method.receiver {
        Some(crate::core::ir::ReceiverKind::Ref) => "&self",
        Some(crate::core::ir::ReceiverKind::RefMut) => "&mut self",
        Some(crate::core::ir::ReceiverKind::Owned) => "self",
        None => "",
    };

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            format!(
                "{}: {}",
                p.name,
                format_param_type_with_lifetimes(p, &spec.type_paths, &spec.lifetime_type_names)
            )
        })
        .collect();

    let all_params = if receiver.is_empty() {
        params.join(", ")
    } else if params.is_empty() {
        receiver.to_string()
    } else {
        format!("{}, {}", receiver, params.join(", "))
    };

    let error_override = method.error_type.as_ref().map(|_| spec.error_path());
    let ret = format_return_type(
        &method.return_type,
        error_override.as_deref(),
        &spec.type_paths,
        method.returns_ref,
    );

    let arg_names: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();

    TraitMethodSig {
        async_kw,
        all_params,
        ret,
        arg_names: arg_names.join(", "),
    }
}
