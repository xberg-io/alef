use super::call_args::needs_napi_cast;
use crate::codegen::generators;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

fn arc_wrap(val: &str, type_name: &str, mutex_types: &AHashSet<String>) -> String {
    if mutex_types.contains(type_name) {
        format!("Arc::new(std::sync::Mutex::new({val}))")
    } else {
        format!("Arc::new({val})")
    }
}

#[allow(clippy::too_many_arguments)]
/// NAPI-specific return wrapping for opaque instance methods.
/// Extends the shared `wrap_return` with i64 casts for u64/usize/isize primitives.
pub(in crate::backends::napi::gen_bindings) fn napi_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            let already_arc = expr == "self.inner"
                || expr == "self.inner.clone()"
                || expr.starts_with("self.inner.as_ref()")
                || expr.starts_with("self.inner.clone()");
            if already_arc {
                format!("Self {{ inner: {expr} }}")
            } else if returns_ref {
                format!(
                    "Self {{ inner: {} }}",
                    arc_wrap(&format!("{expr}.clone()"), n, mutex_types)
                )
            } else {
                format!("Self {{ inner: {} }}", arc_wrap(expr, n, mutex_types))
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            let already_arc = expr == "self.inner"
                || expr == "self.inner.clone()"
                || expr.starts_with("self.inner.as_ref()")
                || expr.starts_with("self.inner.clone()");
            if already_arc {
                format!("{prefix}{n} {{ inner: {expr} }}")
            } else if returns_ref {
                format!(
                    "{prefix}{n} {{ inner: {} }}",
                    arc_wrap(&format!("{expr}.clone()"), n, mutex_types)
                )
            } else {
                format!("{prefix}{n} {{ inner: {} }}", arc_wrap(expr, n, mutex_types))
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!(
                        "{expr}.map(|v| {prefix}{name} {{ inner: {} }})",
                        arc_wrap("v.clone()", name, mutex_types)
                    )
                } else {
                    format!(
                        "{expr}.map(|v| {prefix}{name} {{ inner: {} }})",
                        arc_wrap("v", name, mutex_types)
                    )
                }
            }
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if returns_ref {
                        format!(
                            "{expr}.map(|v| v.into_iter().map(|x| {prefix}{n} {{ inner: {} }}).collect())",
                            arc_wrap("x.clone()", n, mutex_types)
                        )
                    } else {
                        format!(
                            "{expr}.map(|v| v.into_iter().map(|x| {prefix}{n} {{ inner: {} }}).collect())",
                            arc_wrap("x", n, mutex_types)
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
                    false,
                ),
            },
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                false,
            ),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!(
                        "{expr}.into_iter().map(|v| {prefix}{name} {{ inner: {} }}).collect()",
                        arc_wrap("v.clone()", name, mutex_types)
                    )
                } else {
                    format!(
                        "{expr}.into_iter().map(|v| {prefix}{name} {{ inner: {} }}).collect()",
                        arc_wrap("v", name, mutex_types)
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
                false,
            ),
        },
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            false,
        ),
    }
}

/// NAPI-specific return wrapping for free functions (no type_name context).
pub(in crate::backends::napi::gen_bindings) fn napi_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    prefix: &str,
    capsule_types: Option<&std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>>,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_napi_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::Named(n) if capsule_types.is_some_and(|ct| ct.contains_key(n.as_str())) => expr.to_string(),
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!(
                    "{prefix}{n} {{ inner: {} }}",
                    arc_wrap(&format!("{expr}.clone()"), n, mutex_types)
                )
            } else {
                format!("{prefix}{n} {{ inner: {} }}", arc_wrap(expr, n, mutex_types))
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Bytes => format!("{expr}.into()"),
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if capsule_types.is_some_and(|ct| ct.contains_key(name.as_str())) => expr.to_string(),
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!(
                        "{expr}.map(|v| {prefix}{name} {{ inner: {} }})",
                        arc_wrap("v.clone()", name, mutex_types)
                    )
                } else {
                    format!(
                        "{expr}.map(|v| {prefix}{name} {{ inner: {} }})",
                        arc_wrap("v", name, mutex_types)
                    )
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Map(_, _) => {
                if returns_ref {
                    format!("{expr}.map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())")
                } else {
                    format!("{expr}.map(|m| m.into_iter().collect())")
                }
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if returns_ref {
                        format!(
                            "{expr}.map(|v| v.into_iter().map(|x| {prefix}{n} {{ inner: {} }}).collect())",
                            arc_wrap("x.clone()", n, mutex_types)
                        )
                    } else {
                        format!(
                            "{expr}.map(|v| v.into_iter().map(|x| {prefix}{n} {{ inner: {} }}).collect())",
                            arc_wrap("x", n, mutex_types)
                        )
                    }
                }
                TypeRef::Named(_) => {
                    if returns_ref {
                        format!("{expr}.map(|v| v.into_iter().map(|x| x.clone().into()).collect())")
                    } else {
                        format!("{expr}.map(|v| v.into_iter().map(Into::into).collect())")
                    }
                }
                _ => expr.to_string(),
            },
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Bytes => format!("{expr}.map(Into::into)"),
            _ => expr.to_string(),
        },
        TypeRef::Map(_, _) => {
            if returns_ref {
                format!("{expr}.iter().map(|(k, v)| (k.clone(), v.clone())).collect()")
            } else {
                format!("{expr}.into_iter().collect()")
            }
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                let target_ty = match p {
                    crate::core::ir::PrimitiveType::F32 => "f64",
                    _ => "i64",
                };
                format!("{expr}.into_iter().map(|v| v as {target_ty}).collect()")
            }
            TypeRef::Vec(inner2) => {
                if let TypeRef::Primitive(p) = inner2.as_ref() {
                    if needs_napi_cast(p) {
                        let target_ty = match p {
                            crate::core::ir::PrimitiveType::F32 => "f64",
                            _ => "i64",
                        };
                        return format!(
                            "{expr}.into_iter().map(|row| row.into_iter().map(|x| x as {target_ty}).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        );
                    }
                }
                expr.to_string()
            }
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    // (clippy::into_iter_on_ref under -D warnings).
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
