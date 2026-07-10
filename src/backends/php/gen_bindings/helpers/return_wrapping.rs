use crate::core::ir::TypeRef;

use super::primitives::needs_i64_cast;

fn php_expr_is_already_arc(expr: &str) -> bool {
    let trimmed = expr.trim();
    trimmed == "self.inner"
        || trimmed == "self.inner.clone()"
        || trimmed.starts_with("self.inner.as_ref()")
        || trimmed.starts_with("self.inner.clone()")
}

#[allow(clippy::too_many_arguments)]
/// PHP-specific return wrapping that handles i64 casts for u64/usize/isize primitives.
/// Extends the shared `wrap_return` with type conversions for primitives that are i64 in PHP.
///
/// For enum returns:
/// - json_string_enum_names: externally-tagged data enums (have data variants); need serde_json::to_string()
/// - string_enum_names: pure unit enums; use serde_json::to_value().as_str() path
pub(crate) fn php_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &ahash::AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    mutex_types: &ahash::AHashSet<String>,
    json_string_enum_names: &ahash::AHashSet<String>,
    string_enum_names: &ahash::AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Bytes => {
            let vec_expr = if returns_ref {
                format!("{expr}.to_vec()")
            } else {
                format!("Vec::<u8>::from({expr})")
            };
            format!("String::from_utf8_lossy(&{vec_expr}).into_owned()")
        }
        TypeRef::Primitive(p) if needs_i64_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if php_expr_is_already_arc(expr) {
                return format!("Self {{ inner: {expr} }}");
            }
            let wrapper = if mutex_types.contains(type_name) {
                |v: String| format!("Arc::new(std::sync::Mutex::new({v}))")
            } else {
                |v: String| format!("Arc::new({v})")
            };
            if returns_cow {
                format!("Self {{ inner: {} }}", wrapper(format!("{expr}.into_owned()")))
            } else if returns_ref {
                format!("Self {{ inner: {} }}", wrapper(format!("{expr}.clone()")))
            } else {
                format!("Self {{ inner: {} }}", wrapper(expr.to_string()))
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if php_expr_is_already_arc(expr) {
                return format!("{n} {{ inner: {expr} }}");
            }
            let wrapper = if mutex_types.contains(n) {
                |v: String| format!("Arc::new(std::sync::Mutex::new({v}))")
            } else {
                |v: String| format!("Arc::new({v})")
            };
            if returns_cow {
                format!("{n} {{ inner: {} }}", wrapper(format!("{expr}.into_owned()")))
            } else if returns_ref {
                format!("{n} {{ inner: {} }}", wrapper(format!("{expr}.clone()")))
            } else {
                format!("{n} {{ inner: {} }}", wrapper(expr.to_string()))
            }
        }
        TypeRef::Named(n) => {
            if json_string_enum_names.contains(n.as_str()) {
                if returns_cow {
                    format!("serde_json::to_string(&{expr}.into_owned()).unwrap_or_default()")
                } else if returns_ref {
                    format!("serde_json::to_string(&{expr}.clone()).unwrap_or_default()")
                } else {
                    format!("serde_json::to_string(&{expr}).unwrap_or_default()")
                }
            } else if string_enum_names.contains(n.as_str()) {
                if returns_cow {
                    format!(
                        "serde_json::to_value(&{expr}.into_owned()).ok().and_then(|v| v.as_str().map(std::string::ToString::to_string)).unwrap_or_default()"
                    )
                } else if returns_ref {
                    format!(
                        "serde_json::to_value(&{expr}.clone()).ok().and_then(|v| v.as_str().map(std::string::ToString::to_string)).unwrap_or_default()"
                    )
                } else {
                    format!(
                        "serde_json::to_value(&{expr}).ok().and_then(|v| v.as_str().map(std::string::ToString::to_string)).unwrap_or_default()"
                    )
                }
            } else {
                if returns_cow {
                    format!("{expr}.into_owned().into()")
                } else if returns_ref {
                    format!("{expr}.clone().into()")
                } else {
                    format!("{expr}.into()")
                }
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_i64_cast(p) => {
                format!("{expr}.map(|v| v as i64)")
            }
            TypeRef::Duration => format!("{expr}.map(|d| d.as_millis() as i64)"),
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if mutex_types.contains(n) {
                    if returns_ref {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(std::sync::Mutex::new(v.clone())) }})")
                    } else {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(std::sync::Mutex::new(v)) }})")
                    }
                } else {
                    if returns_ref {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(v.clone()) }})")
                    } else {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(v) }})")
                    }
                }
            }
            TypeRef::Named(n) => {
                if json_string_enum_names.contains(n.as_str()) {
                    if returns_ref {
                        format!("{expr}.map(|v| serde_json::to_string(&v.clone()).unwrap_or_default())")
                    } else {
                        format!("{expr}.map(|v| serde_json::to_string(&v).unwrap_or_default())")
                    }
                } else if string_enum_names.contains(n.as_str()) {
                    if returns_ref {
                        format!(
                            "{expr}.map(|v| serde_json::to_value(&v.clone()).ok().and_then(|j| j.as_str().map(std::string::ToString::to_string)).unwrap_or_default())"
                        )
                    } else {
                        format!(
                            "{expr}.map(|v| serde_json::to_value(&v).ok().and_then(|j| j.as_str().map(std::string::ToString::to_string)).unwrap_or_default())"
                        )
                    }
                } else {
                    if returns_ref {
                        format!("{expr}.map(|v| v.clone().into())")
                    } else {
                        format!("{expr}.map(Into::into)")
                    }
                }
            }
            _ => {
                use crate::codegen::generators;
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
        },
        TypeRef::Map(_, _) => {
            if returns_ref {
                format!(
                    "{expr}.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::HashMap<_, _>>()"
                )
            } else {
                format!("{expr}.into_iter().collect::<std::collections::HashMap<_, _>>()")
            }
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_i64_cast(p) => {
                format!("{expr}.into_iter().map(|v| v as i64).collect()")
            }
            TypeRef::Vec(inner2) => {
                if let TypeRef::Primitive(p) = inner2.as_ref() {
                    if needs_i64_cast(p) {
                        return format!(
                            "{expr}.into_iter().map(|row| row.into_iter().map(|x| x as i64).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        );
                    }
                }
                use crate::codegen::generators;
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
            // clippy::into_iter_on_ref — `.into_iter()` on a slice reference is equivalent to
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => {
                if json_string_enum_names.contains(n.as_str()) {
                    if returns_ref {
                        format!("{expr}.iter().map(|v| serde_json::to_string(v).unwrap_or_default()).collect()")
                    } else {
                        format!("{expr}.into_iter().map(|v| serde_json::to_string(&v).unwrap_or_default()).collect()")
                    }
                } else if string_enum_names.contains(n.as_str()) {
                    if returns_ref {
                        format!(
                            "{expr}.iter().map(|v| serde_json::to_value(v).ok().and_then(|j| j.as_str().map(std::string::ToString::to_string)).unwrap_or_default()).collect()"
                        )
                    } else {
                        format!(
                            "{expr}.into_iter().map(|v| serde_json::to_value(&v).ok().and_then(|j| j.as_str().map(std::string::ToString::to_string)).unwrap_or_default()).collect()"
                        )
                    }
                } else {
                    if returns_ref {
                        format!("{expr}.iter().cloned().map(Into::into).collect()")
                    } else {
                        format!("{expr}.into_iter().map(Into::into).collect()")
                    }
                }
            }
            _ => {
                use crate::codegen::generators;
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
        },
        _ => {
            use crate::codegen::generators;
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
}
