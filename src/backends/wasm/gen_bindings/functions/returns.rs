//! Return conversion helpers for WASM generated bindings.

use crate::codegen::generators;
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;

pub(super) fn to_turbofish_from(type_name: &str) -> String {
    if let Some(idx) = type_name.find('<') {
        format!("{}::{}", &type_name[..idx], &type_name[idx..])
    } else {
        type_name.to_string()
    }
}

/// Generate a free function binding with deduplication of input DTOs.
/// Returns a string containing any generated Input DTO structs (not in emitted_input_dtos set)
/// followed by the function code.
pub(in crate::backends::wasm::gen_bindings) fn gen_wasm_unimplemented_body(
    return_type: &TypeRef,
    fn_name: &str,
    has_error: bool,
) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(JsValue::from_str(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                crate::core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!(
                "compile_error!(\"alef cannot generate WASM binding for {fn_name}; \
                 configure wasm.exclude_functions or make the return type fallible\")"
            ),
        }
    }
}

/// Detect whether the core-call expression already evaluates to `Arc<T>` for the
/// binding's `inner` field. Mirrors `expr_is_already_arc` in `alef-codegen`.
pub(super) fn wasm_expr_is_already_arc(expr: &str) -> bool {
    let trimmed = expr.trim();
    trimmed == "self.inner"
        || trimmed == "self.inner.clone()"
        || trimmed.starts_with("self.inner.as_ref()")
        || trimmed.starts_with("self.inner.clone()")
}

/// WASM-specific return wrapping for opaque methods (adds prefix for opaque Named returns).
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::wasm::gen_bindings) fn wasm_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if wasm_expr_is_already_arc(expr) {
                format!("Self {{ inner: {expr} }}")
            } else if mutex_types.contains(type_name) {
                generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                )
            } else if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if wasm_expr_is_already_arc(expr) {
                format!("{prefix}{n} {{ inner: {expr} }}")
            } else if mutex_types.contains(n.as_str()) {
                let wrapped = generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                );
                if wrapped.starts_with(&format!("{n} {{")) {
                    format!("{prefix}{}{}", n, &wrapped[n.len()..])
                } else {
                    wrapped
                }
            } else if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        type_name,
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.map(|v| {prefix}{name} {{ {wrap_inner} }})")
                } else if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        type_name,
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ {wrap_inner} }}).collect()")
                } else if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        TypeRef::Map(_, _) => {
            if returns_ref {
                format!("serde_wasm_bindgen::to_value({expr}).unwrap_or(JsValue::NULL)")
            } else {
                generators::wrap_return(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    self_is_opaque,
                    returns_ref,
                    returns_cow,
                )
            }
        }
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            returns_cow,
        ),
    }
}

/// WASM-specific return wrapping for free functions (no type_name context, adds prefix).
pub(super) fn wasm_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if wasm_expr_is_already_arc(expr) {
                format!("{prefix}{n} {{ inner: {expr} }}")
            } else if mutex_types.contains(n.as_str()) {
                let wrapped = generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    "",
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                );
                if wrapped.starts_with(&format!("{n} {{")) {
                    format!("{prefix}{}{}", n, &wrapped[n.len()..])
                } else {
                    wrapped
                }
            } else if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
            if returns_cow && matches!(return_type, TypeRef::Bytes) {
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        "",
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.map(|v| {prefix}{name} {{ {wrap_inner} }})")
                } else if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        "",
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ {wrap_inner} }}).collect()")
                } else if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    // avoid clippy::into_iter_on_ref under -D warnings.
                    format!("{expr}.iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char => {
                if returns_ref {
                    format!("{expr}.iter().map(|s| s.to_string()).collect()")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.iter().map(|b| b.to_vec()).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Lookup whether a named type has `Default` impl in the IR.
/// Returns true if the type is found and `has_default` is true, false otherwise.
pub(super) fn type_has_default(type_name: &str, api: &ApiSurface) -> bool {
    api.types
        .iter()
        .find(|t| t.name == type_name)
        .map(|t| t.has_default)
        .unwrap_or(false)
}
