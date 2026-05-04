use crate::conversions::helpers::{core_prim_str, needs_f64_cast, needs_i32_cast};
use crate::generators::{AsyncPattern, RustBindingConfig};
use ahash::AHashSet;
use alef_core::ir::{CoreWrapper, ParamDef, TypeDef, TypeRef};
use std::fmt::Write;

/// Helper: wrap an opaque inner value in the correct smart pointer expression.
///
/// - Plain opaque types use `Arc::new(val)`.
/// - Mutex-wrapped opaque types use `Arc::new(std::sync::Mutex::new(val))`.
fn arc_wrap(val: &str, name: &str, mutex_types: &AHashSet<String>) -> String {
    if mutex_types.contains(name) {
        format!("Arc::new(std::sync::Mutex::new({val}))")
    } else {
        format!("Arc::new({val})")
    }
}

/// Wrap a core-call result for opaque delegation methods.
///
/// - `TypeRef::Named(n)` where `n == type_name` → re-wrap in `Self { inner: Arc::new(...) }`
/// - `TypeRef::Named(n)` where `n` is another opaque type → wrap in `{n} { inner: Arc::new(...) }`
/// - `TypeRef::Named(n)` where `n` is a non-opaque type → `todo!()` placeholder (From may not exist)
/// - Everything else (primitives, String, Vec, etc.) → pass through unchanged
/// - `TypeRef::Unit` → pass through unchanged
///
/// When `returns_cow` is true the core method returns `Cow<'_, T>`. `.into_owned()` is emitted
/// before any further type conversion to obtain an owned `T`.
///
/// `mutex_types` identifies opaque types that use `Arc<Mutex<T>>` instead of `Arc<T>`, so
/// constructor expressions use `Arc::new(Mutex::new(...))` where needed.
#[allow(clippy::too_many_arguments)]
pub fn wrap_return_with_mutex(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
) -> String {
    let self_arc = arc_wrap("", type_name, mutex_types); // used for pattern matching only
    let _ = self_arc; // just to reference mutex_types in context
    match return_type {
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            let inner = if returns_cow {
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.clone()")
            } else {
                expr.to_string()
            };
            format!("Self {{ inner: {} }}", arc_wrap(&inner, type_name, mutex_types))
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            let inner = if returns_cow {
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.clone()")
            } else {
                expr.to_string()
            };
            format!("{n} {{ inner: {} }}", arc_wrap(&inner, n, mutex_types))
        }
        TypeRef::Named(_) => {
            // Non-opaque Named return type — use .into() for core→binding From conversion.
            // When the core returns a Cow, call .into_owned() first to get an owned T.
            // When the core returns a reference, clone first since From<&T> typically doesn't exist.
            // NOTE: If this type was sanitized to String in the binding, From won't exist.
            // The calling backend should check method.sanitized before delegating.
            // This code assumes non-sanitized Named types have From impls.
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        // String: only convert when the core returns a reference (&str→String).
        TypeRef::String => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        // Bytes: always use .to_vec() which works for both &Bytes and owned Bytes.
        // &Bytes does not implement From<&Bytes> for Vec<u8>, so .into() fails.
        TypeRef::Bytes => format!("{expr}.to_vec()"),
        // Path: PathBuf→String needs to_string_lossy, &Path→String too
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        // Duration: core returns std::time::Duration, binding uses u64 (millis)
        TypeRef::Duration => format!("{expr}.as_millis() as u64"),
        // Json: serde_json::Value needs serialization to string
        TypeRef::Json => format!("{expr}.to_string()"),
        // Optional: wrap inner conversion in .map(...)
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let wrap = arc_wrap("v", n, mutex_types);
                if returns_ref {
                    format!(
                        "{expr}.map(|v| {n} {{ inner: {} }})",
                        arc_wrap("v.clone()", n, mutex_types)
                    )
                } else {
                    format!("{expr}.map(|v| {n} {{ inner: {wrap} }})")
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
            TypeRef::String | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Duration => format!("{expr}.map(|d| d.as_millis() as u64)"),
            TypeRef::Json => format!("{expr}.map(ToString::to_string)"),
            // Optional<Vec<Named>>: convert each element in the inner Vec
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if returns_ref {
                        let wrap = arc_wrap("x.clone()", n, mutex_types);
                        format!("{expr}.map(|v| v.into_iter().map(|x| {n} {{ inner: {wrap} }}).collect())")
                    } else {
                        let wrap = arc_wrap("x", n, mutex_types);
                        format!("{expr}.map(|v| v.into_iter().map(|x| {n} {{ inner: {wrap} }}).collect())")
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
            _ => expr.to_string(),
        },
        // Vec: map each element through the appropriate conversion
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if returns_ref {
                    let wrap = arc_wrap("v.clone()", n, mutex_types);
                    format!("{expr}.into_iter().map(|v| {n} {{ inner: {wrap} }}).collect()")
                } else {
                    let wrap = arc_wrap("v", n, mutex_types);
                    format!("{expr}.into_iter().map(|v| {n} {{ inner: {wrap} }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Wrap a core-call result for opaque delegation methods.
///
/// This is the backward-compatible wrapper that passes an empty `mutex_types` set.
/// Use `wrap_return_with_mutex` when the type set contains mutex-wrapped opaque types.
pub fn wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
) -> String {
    wrap_return_with_mutex(
        expr,
        return_type,
        type_name,
        opaque_types,
        &AHashSet::new(),
        self_is_opaque,
        returns_ref,
        returns_cow,
    )
}

/// Unwrap a newtype return value when `return_newtype_wrapper` is set.
///
/// Core function returns a newtype (e.g. `NodeIndex(u32)`), but the binding return type
/// is the inner type (e.g. `u32`). Access `.0` to unwrap the newtype.
pub fn apply_return_newtype_unwrap(expr: &str, return_newtype_wrapper: &Option<String>) -> String {
    match return_newtype_wrapper {
        Some(_) => format!("({expr}).0"),
        None => expr.to_string(),
    }
}

/// Build call argument expressions from parameters.
/// - Opaque Named types: unwrap Arc wrapper via `(*param.inner).clone()`
/// - Non-opaque Named types: `.into()` for From conversion
/// - String/Path/Bytes: `&param` since core functions typically take `&str`/`&Path`/`&[u8]`
/// - Params with `newtype_wrapper` set: re-wrap the raw value in the newtype constructor
///   (e.g., `NodeIndex(parent)`) since the binding resolved `NodeIndex(u32)` → `u32`.
///
/// NOTE: This function does not perform serde-based conversion. For Named params that lack
/// From impls (e.g., due to sanitized fields), use `gen_serde_let_bindings` instead when
/// `cfg.has_serde` is true, or fall back to `gen_unimplemented_body`.
pub fn gen_call_args(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let promoted = crate::shared::is_promoted_optional(params, idx);
            // If a required param was promoted to optional, unwrap it before use.
            // Note: promoted params that are not Optional<T> will NOT call .expect() because
            // promoted refers to the PyO3 signature constraint, not the actual Rust type.
            // The function_params logic wraps promoted params in Option<T>, making them truly optional.
            let unwrap_suffix = if promoted && p.optional {
                format!(".expect(\"'{}' is required\")", p.name)
            } else {
                String::new()
            };
            // If this param's type was resolved from a newtype (e.g. NodeIndex(u32) → u32),
            // re-wrap the raw value back into the newtype when calling core.
            if let Some(newtype_path) = &p.newtype_wrapper {
                return if p.optional {
                    format!("{}.map({newtype_path})", p.name)
                } else if promoted {
                    format!("{newtype_path}({}{})", p.name, unwrap_suffix)
                } else {
                    format!("{newtype_path}({})", p.name)
                };
            }
            match &p.ty {
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    // Opaque type: borrow through Arc to get &CoreType
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else if promoted {
                        format!("{}{}.inner.as_ref()", p.name, unwrap_suffix)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => {
                    if p.optional {
                        if p.is_ref {
                            // Option<T> (binding) -> Option<&CoreT>: use as_ref() only
                            // The Into conversion must happen in a let binding to avoid E0716
                            format!("{}.as_ref()", p.name)
                        } else {
                            format!("{}.map(Into::into)", p.name)
                        }
                    } else if promoted {
                        format!("{}{}.into()", p.name, unwrap_suffix)
                    } else {
                        format!("{}.into()", p.name)
                    }
                }
                // String → &str for core function calls when is_ref=true,
                // or pass owned when is_ref=false (core takes String/impl Into<String>).
                // For optional params: as_deref() when is_ref=true, pass owned when is_ref=false.
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if promoted {
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                // Path → PathBuf/&Path for core function calls
                TypeRef::Path => {
                    if p.optional && p.is_ref {
                        format!("{}.as_deref().map(std::path::Path::new)", p.name)
                    } else if p.optional {
                        format!("{}.map(std::path::PathBuf::from)", p.name)
                    } else if promoted {
                        format!("std::path::PathBuf::from({}{})", p.name, unwrap_suffix)
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{})", p.name)
                    } else {
                        format!("std::path::PathBuf::from({})", p.name)
                    }
                }
                TypeRef::Bytes => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if promoted {
                        // is_ref=true: pass &Vec<u8> (core takes &[u8])
                        // is_ref=false: pass Vec<u8> (core takes owned Vec<u8>)
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else {
                        // is_ref=true: pass &Vec<u8> (core takes &[u8])
                        // is_ref=false: pass Vec<u8> (core takes owned Vec<u8>)
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.clone()
                        }
                    }
                }
                // Duration: binding uses u64 (millis), core uses std::time::Duration
                TypeRef::Duration => {
                    if p.optional {
                        format!("{}.map(std::time::Duration::from_millis)", p.name)
                    } else if promoted {
                        format!("std::time::Duration::from_millis({}{})", p.name, unwrap_suffix)
                    } else {
                        format!("std::time::Duration::from_millis({})", p.name)
                    }
                }
                TypeRef::Json => {
                    // JSON params: binding has String, core expects serde_json::Value
                    if p.optional {
                        format!("{}.as_ref().and_then(|s| serde_json::from_str(s).ok())", p.name)
                    } else if promoted {
                        format!("serde_json::from_str(&{}{}).unwrap_or_default()", p.name, unwrap_suffix)
                    } else {
                        format!("serde_json::from_str(&{}).unwrap_or_default()", p.name)
                    }
                }
                TypeRef::Vec(inner) => {
                    // Vec<Named>: convert each element via Into::into when used with let bindings
                    if matches!(inner.as_ref(), TypeRef::Named(_)) {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.as_deref()", p.name)
                            } else {
                                p.name.clone()
                            }
                        } else if promoted {
                            if p.is_ref {
                                format!("&{}{}", p.name, unwrap_suffix)
                            } else {
                                format!("{}{}", p.name, unwrap_suffix)
                            }
                        } else if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if promoted {
                        format!("{}{}", p.name, unwrap_suffix)
                    } else if p.is_mut && p.optional {
                        format!("{}.as_deref_mut()", p.name)
                    } else if p.is_mut {
                        format!("&mut {}", p.name)
                    } else if p.is_ref && p.optional {
                        format!("{}.as_deref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => {
                    if promoted {
                        format!("{}{}", p.name, unwrap_suffix)
                    } else if p.is_mut && p.optional {
                        format!("{}.as_deref_mut()", p.name)
                    } else if p.is_mut {
                        format!("&mut {}", p.name)
                    } else if p.is_ref && p.optional {
                        // Optional ref params: use as_deref() for slice/str coercion
                        // Option<Vec<T>> -> Option<&[T]>, Option<String> -> Option<&str>
                        format!("{}.as_deref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build call argument expressions using pre-bound let bindings for non-opaque Named params.
/// Non-opaque Named params use `&{name}_core` references instead of `.into()`.
pub fn gen_call_args_with_let_bindings(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let promoted = crate::shared::is_promoted_optional(params, idx);
            let unwrap_suffix = if promoted {
                format!(".expect(\"'{}' is required\")", p.name)
            } else {
                String::new()
            };
            // If this param's type was resolved from a newtype, re-wrap when calling core.
            if let Some(newtype_path) = &p.newtype_wrapper {
                return if p.optional {
                    format!("{}.map({newtype_path})", p.name)
                } else if promoted {
                    format!("{newtype_path}({}{})", p.name, unwrap_suffix)
                } else {
                    format!("{newtype_path}({})", p.name)
                };
            }
            match &p.ty {
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else if promoted {
                        format!("{}{}.inner.as_ref()", p.name, unwrap_suffix)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => {
                    if p.optional && p.is_ref {
                        // Let binding already created Option<&T> via .as_ref()
                        format!("{}_core", p.name)
                    } else if p.is_ref {
                        // Let binding created T, need reference for call
                        format!("&{}_core", p.name)
                    } else {
                        format!("{}_core", p.name)
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if promoted {
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Path => {
                    if promoted {
                        format!("std::path::PathBuf::from({}{})", p.name, unwrap_suffix)
                    } else if p.optional && p.is_ref {
                        format!("{}.as_deref().map(std::path::Path::new)", p.name)
                    } else if p.optional {
                        format!("{}.map(std::path::PathBuf::from)", p.name)
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{})", p.name)
                    } else {
                        format!("std::path::PathBuf::from({})", p.name)
                    }
                }
                TypeRef::Bytes => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if promoted {
                        // is_ref=true: pass &Vec<u8> (core takes &[u8])
                        // is_ref=false: pass Vec<u8> (core takes owned Vec<u8>)
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else {
                        // is_ref=true: pass &Vec<u8> (core takes &[u8])
                        // is_ref=false: pass Vec<u8> (core takes owned Vec<u8>)
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.clone()
                        }
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{}.map(std::time::Duration::from_millis)", p.name)
                    } else if promoted {
                        format!("std::time::Duration::from_millis({}{})", p.name, unwrap_suffix)
                    } else {
                        format!("std::time::Duration::from_millis({})", p.name)
                    }
                }
                TypeRef::Vec(inner) => {
                    // Sanitized Vec<tuple>: binding accepts Vec<String> (JSON-encoded tuples).
                    // Let binding created {name}_core via JSON deserialization.
                    if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                        if p.optional && p.is_ref {
                            format!("{}_core.as_deref()", p.name)
                        } else if p.optional {
                            format!("{}_core", p.name)
                        } else if p.is_ref {
                            format!("&{}_core", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else if matches!(inner.as_ref(), TypeRef::Named(_)) {
                        // Vec<Named>: use let binding that converts each element
                        if p.optional && p.is_ref {
                            // Let binding creates Option<Vec<CoreType>>, use as_deref() to get Option<&[CoreType]>
                            format!("{}_core.as_deref()", p.name)
                        } else if p.optional {
                            // Let binding creates Option<Vec<CoreType>>, no ref needed
                            format!("{}_core", p.name)
                        } else if p.is_ref {
                            format!("&{}_core", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref {
                        // Vec<String> with is_ref=true: core expects &[&str].
                        // Convert via _refs intermediate binding.
                        if p.optional {
                            format!("{}.as_deref()", p.name)
                        } else {
                            format!("&{}_refs", p.name)
                        }
                    } else if promoted {
                        format!("{}{}", p.name, unwrap_suffix)
                    } else if p.is_ref && p.optional {
                        format!("{}.as_deref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => {
                    if promoted {
                        format!("{}{}", p.name, unwrap_suffix)
                    } else if p.is_ref && p.optional {
                        format!("{}.as_deref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for non-opaque Named params, converting them to core types.
pub fn gen_named_let_bindings_pub(params: &[ParamDef], opaque_types: &AHashSet<String>, core_import: &str) -> String {
    gen_named_let_bindings(params, opaque_types, core_import)
}

/// Like `gen_named_let_bindings_pub` but without optional-promotion semantics.
/// Use this for backends (e.g. WASM) that do not promote non-optional params to `Option<T>`.
pub fn gen_named_let_bindings_no_promote(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    gen_named_let_bindings_inner(params, opaque_types, core_import, false)
}

pub(super) fn gen_named_let_bindings(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    gen_named_let_bindings_inner(params, opaque_types, core_import, true)
}

/// Variant of `gen_named_let_bindings` for backends where Named non-opaque params
/// are passed by reference (`&T`) in the function signature (e.g. extendr).
/// Uses `.clone().into()` instead of `.into()` to convert the borrowed value.
pub(super) fn gen_named_let_bindings_by_ref(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let promoted = crate::shared::is_promoted_optional(params, idx);
                let core_type_path = format!("{core_import}::{name}");
                if p.optional {
                    // Nullable<&T>: use into_option() then clone+convert each element
                    write!(
                        bindings,
                        "let {name}_core: Option<{core_type_path}> = {name}.into_option().map(|v| v.clone().into());\n    ",
                        name = p.name
                    )
                    .ok();
                } else if promoted {
                    // Promoted-optional (Nullable<&T>): expect not-null then clone+convert
                    write!(
                        bindings,
                        "let {name}_core: {core_type_path} = {name}.into_option().expect(\"'{name}' is required\").clone().into();\n    ",
                        name = p.name
                    )
                    .ok();
                } else {
                    // Required &T: clone and convert
                    write!(
                        bindings,
                        "let {name}_core: {core_type_path} = {name}.clone().into();\n    ",
                        name = p.name
                    )
                    .ok();
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())) => {
                if p.optional {
                    write!(
                        bindings,
                        "let {name}_core = {name}.as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect()).unwrap_or_default();\n    ",
                        name = p.name
                    )
                    .ok();
                } else {
                    let promoted = crate::shared::is_promoted_optional(params, idx);
                    if promoted {
                        write!(
                            bindings,
                            "let {name}_core: Vec<_> = {name}.expect(\"'{name}' is required\").into_iter().map(Into::into).collect();\n    ",
                            name = p.name
                        )
                        .ok();
                    } else {
                        write!(
                            bindings,
                            "let {name}_core: Vec<_> = {name}.into_iter().map(Into::into).collect();\n    ",
                            name = p.name
                        )
                        .ok();
                    }
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                if p.optional {
                    write!(
                        bindings,
                        "let {name}_refs: Vec<&str> = {name}.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect()).unwrap_or_default();\n    ",
                        name = p.name
                    )
                    .ok();
                } else {
                    write!(
                        bindings,
                        "let {name}_refs: Vec<&str> = {name}.iter().map(|s| s.as_str()).collect();\n    ",
                        name = p.name
                    )
                    .ok();
                }
            }
            _ => {}
        }
    }
    bindings
}

fn gen_named_let_bindings_inner(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    promote: bool,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let promoted = promote && crate::shared::is_promoted_optional(params, idx);
                let core_type_path = format!("{}::{}", core_import, name);
                if p.optional {
                    if p.is_ref {
                        // Option<T> (binding) -> Option<&CoreT> (core expects reference to core type)
                        // Split into two bindings to avoid temporary value dropped while borrowed (E0716)
                        write!(
                            bindings,
                            "let {name}_owned: Option<{core_type_path}> = {name}.map(Into::into);\n    let {name}_core = {name}_owned.as_ref();\n    ",
                            name = p.name
                        )
                        .ok();
                    } else {
                        write!(
                            bindings,
                            "let {}_core: Option<{core_type_path}> = {}.map(Into::into);\n    ",
                            p.name, p.name
                        )
                        .ok();
                    }
                } else if promoted {
                    // Promoted-optional: unwrap then convert. Add explicit type annotation to help type inference.
                    write!(
                        bindings,
                        "let {}_core: {core_type_path} = {}.expect(\"'{}' is required\").into();\n    ",
                        p.name, p.name, p.name
                    )
                    .ok();
                } else {
                    // Non-optional: add explicit type annotation to help type inference
                    write!(
                        bindings,
                        "let {}_core: {core_type_path} = {}.into();\n    ",
                        p.name, p.name
                    )
                    .ok();
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())) => {
                let promoted = promote && crate::shared::is_promoted_optional(params, idx);
                if p.optional && p.is_ref {
                    // Option<Vec<Named>> with is_ref: convert to Option<Vec<CoreType>>, then use as_deref()
                    // This ensures elements are converted from binding to core type.
                    write!(
                        bindings,
                        "let {}_core: Option<Vec<_>> = {}.as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect());\n    ",
                        p.name, p.name
                    )
                    .ok();
                } else if p.optional {
                    // Option<Vec<Named>> without is_ref: convert to concrete Vec
                    write!(
                        bindings,
                        "let {}_core = {}.as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect()).unwrap_or_default();\n    ",
                        p.name, p.name
                    )
                    .ok();
                } else if promoted {
                    // Promoted-optional: unwrap then convert
                    write!(
                        bindings,
                        "let {}_core: Vec<_> = {}.expect(\"'{}' is required\").into_iter().map(Into::into).collect();\n    ",
                        p.name, p.name, p.name
                    )
                    .ok();
                } else if p.is_ref {
                    // Non-optional Vec<Named> with is_ref=true: generate let binding for conversion
                    write!(
                        bindings,
                        "let {}_core: Vec<_> = {}.into_iter().map(Into::into).collect();\n    ",
                        p.name, p.name
                    )
                    .ok();
                } else {
                    // Vec<Named>: convert each element
                    write!(
                        bindings,
                        "let {}_core: Vec<_> = {}.into_iter().map(Into::into).collect();\n    ",
                        p.name, p.name
                    )
                    .ok();
                }
            }
            // Vec<String> with is_ref=true: core expects &[&str].
            // Convert Vec<String> to Vec<&str> via intermediate binding.
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                write!(
                    bindings,
                    "let {n}_refs: Vec<&str> = {n}.iter().map(|s| s.as_str()).collect();\n    ",
                    n = p.name,
                )
                .ok();
            }
            // Sanitized Vec<String> (originally Vec<tuple>): deserialize each JSON string.
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() =>
            {
                if p.optional {
                    write!(
                        bindings,
                        "let {n}_core: Option<Vec<_>> = {n}.map(|strs| \
                         strs.into_iter()\n    \
                         .filter_map(|s| serde_json::from_str(&s).ok())\n    \
                         .collect()\n    \
                         );\n    ",
                        n = p.name,
                    )
                    .ok();
                } else {
                    write!(
                        bindings,
                        "let {n}_core: Vec<_> = {n}.into_iter()\n    \
                         .filter_map(|s| serde_json::from_str(&s).ok())\n    \
                         .collect();\n    ",
                        n = p.name,
                    )
                    .ok();
                }
            }
            _ => {}
        }
    }
    bindings
}

/// Generate serde-based let bindings for non-opaque Named params.
/// Serializes binding types to JSON and deserializes to core types.
/// Used when From impls don't exist (e.g., types with sanitized fields).
/// `indent` is the whitespace prefix for each generated line (e.g., "    " for functions, "        " for methods).
/// NOTE: This function should only be called when `cfg.has_serde` is true.
/// The caller (functions.rs, methods.rs) must gate the call behind a `has_serde` check.
pub fn gen_serde_let_bindings(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    err_conv: &str,
    indent: &str,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        let promoted = crate::shared::is_promoted_optional(params, idx);
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let core_path = format!("{}::{}", core_import, name);
                if p.optional {
                    write!(
                        bindings,
                        "let {name}_core: Option<{core_path}> = {name}.map(|v| {{\n\
                         {indent}    let json = serde_json::to_string(&v){err_conv}?;\n\
                         {indent}    serde_json::from_str(&json){err_conv}\n\
                         {indent}}}).transpose()?;\n{indent}",
                        name = p.name,
                        core_path = core_path,
                        err_conv = err_conv,
                        indent = indent,
                    )
                    .ok();
                } else if promoted {
                    // Promoted-optional: param is required in core but wrapped in Option<T>
                    // in the binding because an earlier param is optional. Use unwrap_or_default()
                    // so JS callers can omit it (pass undefined/null) to get default behaviour.
                    write!(
                        bindings,
                        "let {name}_core: {core_path} = {name}.map(|v| {{\n\
                         {indent}    let json = serde_json::to_string(&v){err_conv}?;\n\
                         {indent}    serde_json::from_str::<{core_path}>(&json){err_conv}\n\
                         {indent}}}).transpose()?{indent}.unwrap_or_default();\n{indent}",
                        name = p.name,
                        core_path = core_path,
                        err_conv = err_conv,
                        indent = indent,
                    )
                    .ok();
                } else {
                    write!(
                        bindings,
                        "let {name}_json = serde_json::to_string(&{name}){err_conv}?;\n\
                         {indent}let {name}_core: {core_path} = serde_json::from_str(&{name}_json){err_conv}?;\n{indent}",
                        name = p.name,
                        core_path = core_path,
                        err_conv = err_conv,
                        indent = indent,
                    )
                    .ok();
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        let core_path = format!("{}::{}", core_import, name);
                        if p.optional {
                            write!(
                                bindings,
                                "let {name}_core: Option<Vec<{core_path}>> = {name}.map(|v| {{\n\
                                 {indent}    let json = serde_json::to_string(&v){err_conv}?;\n\
                                 {indent}    serde_json::from_str(&json){err_conv}\n\
                                 {indent}}}).transpose()?;\n{indent}",
                                name = p.name,
                                core_path = core_path,
                                err_conv = err_conv,
                                indent = indent,
                            )
                            .ok();
                        } else {
                            write!(
                                bindings,
                                "let {name}_json = serde_json::to_string(&{name}){err_conv}?;\n\
                                 {indent}let {name}_core: Vec<{core_path}> = serde_json::from_str(&{name}_json){err_conv}?;\n{indent}",
                                name = p.name,
                                core_path = core_path,
                                err_conv = err_conv,
                                indent = indent,
                            )
                            .ok();
                        }
                    }
                } else if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                    // Sanitized Vec<tuple>: binding accepts Vec<String> (JSON-encoded tuple items).
                    // Deserialize each JSON string as a tuple using serde_json.
                    if p.optional {
                        write!(
                            bindings,
                            "let {n}_core: Option<Vec<_>> = {n}.map(|strs| {{\n\
                             {indent}    strs.into_iter()\n\
                             {indent}    .map(|s| serde_json::from_str::<_>(&s){err_conv})\n\
                             {indent}    .collect::<Result<Vec<_>, _>>()\n\
                             {indent}}}).transpose()?;\n{indent}",
                            n = p.name,
                            err_conv = err_conv,
                            indent = indent,
                        )
                        .ok();
                    } else {
                        write!(
                            bindings,
                            "let {n}_core: Vec<_> = {n}.into_iter()\n\
                             {indent}.map(|s| serde_json::from_str::<_>(&s){err_conv})\n\
                             {indent}.collect::<Result<Vec<_>, _>>()?;\n{indent}",
                            n = p.name,
                            err_conv = err_conv,
                            indent = indent,
                        )
                        .ok();
                    }
                }
            }
            _ => {}
        }
    }
    bindings
}

/// Check if params contain any non-opaque Named types that need let bindings.
/// This includes direct Named types, Vec<Named> types, Vec<String> params
/// with is_ref=true (which need a Vec<&str> intermediate to pass as &[&str]),
/// and sanitized Vec<String> params (which are JSON-deserialized to tuples).
pub fn has_named_params(params: &[ParamDef], opaque_types: &AHashSet<String>) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => true,
        TypeRef::Vec(inner) => {
            // Vec<Named> always needs a conversion let binding.
            // Sanitized Vec<String> needs JSON deserialization via let binding.
            // Vec<String> with is_ref=true needs a _refs let binding for &[&str] conversion.
            matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name.as_str()))
                || (matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref)
                || (matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some())
        }
        _ => false,
    })
}

/// Check if a param type is safe for non-opaque delegation (no complex conversions needed).
/// Vec and Map params can cause type mismatches (e.g. Vec<String> vs &[&str]).
pub fn is_simple_non_opaque_param(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Optional(inner) => is_simple_non_opaque_param(inner),
        _ => false,
    }
}

/// Generate a lossy binding→core struct literal for non-opaque delegation.
/// Sanitized fields use `Default::default()`, non-sanitized fields are cloned and converted.
/// Fields are accessed via `self.` (behind &self), so all non-Copy types need `.clone()`.
///
/// `opaque_types` is the set of opaque type names (Arc-wrapped handles, trait bridge aliases,
/// etc.). Fields whose `TypeRef::Named` type is in this set have no `From` impl in the binding
/// layer, so `Default::default()` is emitted for them instead of `.clone().into()`.
///
/// NOTE: This assumes all binding struct fields implement Clone. If a field type does not
/// implement Clone (e.g., `Mutex<T>`), it should be marked as `sanitized=true` so that
/// `Default::default()` is used instead of calling `.clone()`. Backends that exclude types
/// should mark such fields appropriately.
pub fn gen_lossy_binding_to_core_fields(
    typ: &TypeDef,
    core_import: &str,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        false,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
    )
}

/// Same as `gen_lossy_binding_to_core_fields` but declares `core_self` as mutable.
pub fn gen_lossy_binding_to_core_fields_mut(
    typ: &TypeDef,
    core_import: &str,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        true,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
    )
}

fn gen_lossy_binding_to_core_fields_inner(
    typ: &TypeDef,
    core_import: &str,
    needs_mut: bool,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    let core_path = crate::conversions::core_type_path(typ, core_import);
    let mut_kw = if needs_mut { "mut " } else { "" };
    // When has_stripped_cfg_fields is true we emit ..Default::default() at the end of the
    // struct literal to fill cfg-gated fields that were stripped from the binding IR.
    // Suppress clippy::needless_update because the fields only exist when the corresponding
    // feature is enabled — without the feature, clippy thinks the spread is redundant.
    let allow = if typ.has_stripped_cfg_fields {
        "#[allow(clippy::needless_update)]\n        "
    } else {
        ""
    };
    let mut out = format!("{allow}let {mut_kw}core_self = {core_path} {{\n");
    for field in &typ.fields {
        let name = &field.name;
        if field.sanitized && field.core_wrapper != CoreWrapper::Cow {
            writeln!(out, "            {name}: Default::default(),").ok();
            continue;
        }
        // Opaque-type fields (Arc-wrapped handles, trait bridge aliases) have no From impl
        // in the binding layer. Emit Default::default() so the apply_update / clone-mutate
        // paths compile without needing From<Arc<Py<PyAny>>> for VisitorHandle, etc.
        // This covers both bare Named opaque fields and Optional<Named opaque> fields.
        let is_opaque_named = match &field.ty {
            TypeRef::Named(n) => opaque_types.contains(n.as_str()),
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if opaque_types.contains(n.as_str()))
            }
            _ => false,
        };
        if is_opaque_named {
            writeln!(out, "            {name}: Default::default(),").ok();
            continue;
        }
        let expr = match &field.ty {
            TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                let core_ty = core_prim_str(p);
                if field.optional {
                    format!("self.{name}.map(|v| v as {core_ty})")
                } else {
                    format!("self.{name} as {core_ty}")
                }
            }
            TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                let core_ty = core_prim_str(p);
                if field.optional {
                    format!("self.{name}.map(|v| v as {core_ty})")
                } else {
                    format!("self.{name} as {core_ty}")
                }
            }
            TypeRef::Primitive(_) => format!("self.{name}"),
            TypeRef::Duration => {
                if field.optional {
                    format!("self.{name}.map(std::time::Duration::from_millis)")
                } else if option_duration_on_defaults && typ.has_default {
                    // When option_duration_on_defaults is true, non-optional Duration fields
                    // on has_default types are stored as Option<u64> in the binding struct.
                    // Use .map(...).unwrap_or_default() so that None falls back to the core
                    // type's Default (e.g. Duration::from_secs(30)) rather than Duration::ZERO.
                    format!("self.{name}.map(std::time::Duration::from_millis).unwrap_or_default()")
                } else {
                    format!("std::time::Duration::from_millis(self.{name})")
                }
            }
            TypeRef::String => {
                if field.core_wrapper == CoreWrapper::Cow {
                    format!("self.{name}.clone().into()")
                } else {
                    format!("self.{name}.clone()")
                }
            }
            // Bytes: binding stores Vec<u8>. When core_wrapper == Bytes, core expects
            // bytes::Bytes so we must call .into() to convert Vec<u8> → Bytes.
            // When core_wrapper == None, the core field is also Vec<u8> (plain clone).
            TypeRef::Bytes => {
                if field.core_wrapper == CoreWrapper::Bytes {
                    format!("self.{name}.clone().into()")
                } else {
                    format!("self.{name}.clone()")
                }
            }
            TypeRef::Char => {
                if field.optional {
                    format!("self.{name}.as_ref().and_then(|s| s.chars().next())")
                } else {
                    format!("self.{name}.chars().next().unwrap_or('*')")
                }
            }
            TypeRef::Path => {
                if field.optional {
                    format!("self.{name}.clone().map(Into::into)")
                } else {
                    format!("self.{name}.clone().into()")
                }
            }
            TypeRef::Named(_) => {
                if field.optional {
                    format!("self.{name}.clone().map(Into::into)")
                } else {
                    format!("self.{name}.clone().into()")
                }
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(_) => {
                    if field.optional {
                        // Option<Vec<Named(T)>>: map over the Option, then convert each element
                        format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(Into::into).collect()")
                    }
                }
                _ => format!("self.{name}.clone()"),
            },
            TypeRef::Optional(inner) => {
                // When field.optional is also true, the binding field was flattened from
                // Option<Option<T>> to Option<T>. Core expects Option<Option<T>>, so wrap
                // with .map(Some) to reconstruct the double-optional.
                let base = match inner.as_ref() {
                    TypeRef::Named(_) => {
                        format!("self.{name}.clone().map(Into::into)")
                    }
                    TypeRef::Duration => {
                        format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                    }
                    TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                    }
                    _ => format!("self.{name}.clone()"),
                };
                if field.optional {
                    format!("({base}).map(Some)")
                } else {
                    base
                }
            }
            TypeRef::Map(_, v) => match v.as_ref() {
                TypeRef::Json => {
                    // HashMap<String, String> (binding) → HashMap<K, Value> (core).
                    // Emit `k.into()` so wrapped string keys (`Cow`, `Box<str>`, `Arc<str>`)
                    // — which the type resolver collapses to `TypeRef::String` — convert
                    // correctly. For a real `String` core key it is a no-op.
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| \
                                 (k.into(), serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)))).collect())"
                        )
                    } else {
                        format!(
                            "self.{name}.clone().into_iter().map(|(k, v)| \
                                 (k.into(), serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)))).collect()"
                        )
                    }
                }
                // Named values: each value needs Into conversion to bridge the binding wrapper
                // type into the core type (e.g. PyExtractionPattern → ExtractionPattern).
                TypeRef::Named(_) => {
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
                    }
                }
                // Collect to handle HashMap↔BTreeMap conversion
                _ => {
                    if field.optional {
                        format!("self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v)).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v)).collect()")
                    }
                }
            },
            TypeRef::Unit => format!("self.{name}.clone()"),
            TypeRef::Json => {
                // String (binding) → serde_json::Value (core)
                if field.optional {
                    format!("self.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())")
                } else {
                    format!("serde_json::from_str(&self.{name}).unwrap_or_default()")
                }
            }
        };
        // Newtype wrapping: when the field was resolved from a newtype (e.g. NodeIndex → u32),
        // re-wrap the binding value into the newtype for the core struct literal.
        // When `optional=true` and `ty` is a plain Primitive (not TypeRef::Optional), the core
        // field is actually `Option<NewtypeT>`, so we must use `.map(NewtypeT)` not `NewtypeT(...)`.
        let expr = if let Some(newtype_path) = &field.newtype_wrapper {
            match &field.ty {
                TypeRef::Optional(_) => format!("({expr}).map({newtype_path})"),
                TypeRef::Vec(_) => format!("({expr}).into_iter().map({newtype_path}).collect::<Vec<_>>()"),
                _ if field.optional => format!("({expr}).map({newtype_path})"),
                _ => format!("{newtype_path}({expr})"),
            }
        } else {
            expr
        };
        writeln!(out, "            {name}: {expr},").ok();
    }
    // Use ..Default::default() to fill cfg-gated fields stripped from the IR
    if typ.has_stripped_cfg_fields {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str("        };\n        ");
    out
}

/// Generate the body for an async call, unified across methods, static methods, and free functions.
///
/// - `core_call`: the expression to await, e.g. `inner.method(args)` or `CoreType::fn(args)`.
///   For Pyo3FutureIntoPy opaque methods this should reference `inner` (the Arc clone);
///   for all other patterns it may reference `self.inner` or a static call expression.
/// - `cfg`: binding configuration (determines which async pattern to emit)
/// - `has_error`: whether the core call returns a `Result`
/// - `return_wrap`: expression to produce the binding return value from `result`,
///   e.g. `"result"` or `"TypeName::from(result)"`
///
/// - `is_opaque`: whether the binding type is Arc-wrapped (affects TokioBlockOn wrapping)
/// - `inner_clone_line`: optional statement emitted before the pattern-specific body,
///   e.g. `"let inner = self.inner.clone();\n        "` for opaque instance methods, or `""`.
///   Required when `core_call` references `inner` (Pyo3FutureIntoPy opaque case).
#[allow(clippy::too_many_arguments)]
pub fn gen_async_body(
    core_call: &str,
    cfg: &RustBindingConfig,
    has_error: bool,
    return_wrap: &str,
    is_opaque: bool,
    inner_clone_line: &str,
    is_unit_return: bool,
    return_type: Option<&str>,
) -> String {
    let pattern_body = match cfg.async_pattern {
        AsyncPattern::Pyo3FutureIntoPy => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n            \
                     .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let (ok_expr, extra_binding) = if is_unit_return && !has_error {
                ("()".to_string(), String::new())
            } else if return_wrap.contains(".into()") || return_wrap.contains("::from(") {
                // When return_wrap contains type conversions like .into() or ::from(),
                // bind to a variable to help type inference for the generic future_into_py.
                // This avoids E0283 "type annotations needed".
                let wrapped_var = "wrapped_result";
                let binding = if let Some(ret_type) = return_type {
                    // Add explicit type annotation to help type inference
                    format!("let {wrapped_var}: {ret_type} = {return_wrap};\n            ")
                } else {
                    format!("let {wrapped_var} = {return_wrap};\n            ")
                };
                (wrapped_var.to_string(), binding)
            } else {
                (return_wrap.to_string(), String::new())
            };
            format!(
                "pyo3_async_runtimes::tokio::future_into_py(py, async move {{\n            \
                 {result_handling}\n            \
                 {extra_binding}Ok({ok_expr})\n        }})"
            )
        }
        AsyncPattern::WasmNativeAsync => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let ok_expr = if is_unit_return && !has_error {
                "()"
            } else {
                return_wrap
            };
            format!(
                "{result_handling}\n        \
                 Ok({ok_expr})"
            )
        }
        AsyncPattern::NapiNativeAsync => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n            \
                     .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            if !has_error && !is_unit_return {
                // No error type: return value directly without Ok() wrapper
                format!(
                    "{result_handling}\n            \
                     {return_wrap}"
                )
            } else {
                let ok_expr = if is_unit_return && !has_error {
                    "()"
                } else {
                    return_wrap
                };
                format!(
                    "{result_handling}\n            \
                     Ok({ok_expr})"
                )
            }
        }
        AsyncPattern::TokioBlockOn => {
            if has_error {
                if is_opaque {
                    format!(
                        "let rt = tokio::runtime::Runtime::new()?;\n        \
                         let result = rt.block_on(async {{ {core_call}.await.map_err(|e| e.into()) }})?;\n        \
                         {return_wrap}"
                    )
                } else {
                    format!(
                        "let rt = tokio::runtime::Runtime::new()?;\n        \
                         rt.block_on(async {{ {core_call}.await.map_err(|e| e.into()) }})"
                    )
                }
            } else if is_opaque {
                if is_unit_return {
                    format!(
                        "let rt = tokio::runtime::Runtime::new()?;\n        \
                         rt.block_on(async {{ {core_call}.await }});"
                    )
                } else {
                    format!(
                        "let rt = tokio::runtime::Runtime::new()?;\n        \
                         let result = rt.block_on(async {{ {core_call}.await }});\n        \
                         {return_wrap}"
                    )
                }
            } else {
                format!(
                    "let rt = tokio::runtime::Runtime::new()?;\n        \
                     rt.block_on(async {{ {core_call}.await }})"
                )
            }
        }
        AsyncPattern::None => "todo!(\"async not supported by backend\")".to_string(),
    };
    if inner_clone_line.is_empty() {
        pattern_body
    } else {
        format!("{inner_clone_line}{pattern_body}")
    }
}

/// Generate a compilable body for functions that can't be auto-delegated.
/// Returns a default value or error instead of `todo!()` which would panic.
///
/// `opaque_types` is the set of opaque type names (Arc-wrapped). Opaque types do not
/// implement `Default`, so returning `Default::default()` for their Named return types
/// would fail to compile. For those cases a `todo!()` body is emitted instead.
pub fn gen_unimplemented_body(
    return_type: &TypeRef,
    fn_name: &str,
    has_error: bool,
    cfg: &RustBindingConfig,
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
) -> String {
    // Suppress unused_variables by binding all params to `_`
    let suppress = if params.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        if names.len() == 1 {
            format!("let _ = {};\n        ", names[0])
        } else {
            format!("let _ = ({});\n        ", names.join(", "))
        }
    };
    let err_msg = format!("Not implemented: {fn_name}");
    let body = if has_error {
        // Backend-specific error return
        match cfg.async_pattern {
            AsyncPattern::Pyo3FutureIntoPy => {
                format!("Err(pyo3::exceptions::PyNotImplementedError::new_err(\"{err_msg}\"))")
            }
            AsyncPattern::NapiNativeAsync => {
                format!("Err(napi::Error::new(napi::Status::GenericFailure, \"{err_msg}\"))")
            }
            AsyncPattern::WasmNativeAsync => {
                format!("Err(JsValue::from_str(\"{err_msg}\"))")
            }
            _ => format!("Err(\"{err_msg}\".to_string())"),
        }
    } else {
        // Return type-appropriate default
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                alef_core::ir::PrimitiveType::F32 => "0.0f32".to_string(),
                alef_core::ir::PrimitiveType::F64 => "0.0f64".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0".to_string(),
            TypeRef::Named(name) => {
                // Opaque types (Arc-wrapped) do not implement Default — use todo!() to
                // produce a compilable placeholder that panics at runtime if called.
                // Non-opaque Named types (config structs) do derive Default, so use that.
                if opaque_types.contains(name.as_str()) {
                    format!("todo!(\"{err_msg}\")")
                } else {
                    "Default::default()".to_string()
                }
            }
            TypeRef::Json => {
                // Json return without error type: return Default::default()
                "Default::default()".to_string()
            }
        }
    };
    format!("{suppress}{body}")
}
