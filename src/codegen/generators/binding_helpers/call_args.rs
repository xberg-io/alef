use crate::codegen::conversions::helpers::{core_prim_str, needs_f64_cast, needs_i32_cast};
use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;

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
    gen_call_args_vec(params, opaque_types).join(", ")
}

/// Per-parameter call-argument expressions, before joining. Use this when callers must pair each
/// expression with its source param (e.g. building `field: <expr>` struct literals) so there is no
/// need to re-split a comma-joined string. [`gen_call_args`] is `gen_call_args_vec(..).join(", ")`.
pub fn gen_call_args_vec(params: &[ParamDef], opaque_types: &AHashSet<String>) -> Vec<String> {
    params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
            // Note: promoted params that are not Optional<T> will NOT call .expect() because
            let unwrap_suffix = if promoted && p.optional {
                format!(".expect(\"'{}' is required\")", p.name)
            } else {
                String::new()
            };
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
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            format!("{}.map(Into::into)", p.name)
                        }
                    } else if promoted {
                        format!("{}{}.into()", p.name, unwrap_suffix)
                    } else if p.is_mut {
                        format!("&mut {}", p.name)
                    } else {
                        format!("{}.into()", p.name)
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
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else {
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
                TypeRef::Json => {
                    if p.optional {
                        format!("{}.as_ref().and_then(|s| serde_json::from_str(s).ok())", p.name)
                    } else if promoted {
                        format!("serde_json::from_str(&{}{}).unwrap_or_default()", p.name, unwrap_suffix)
                    } else {
                        format!("serde_json::from_str(&{}).unwrap_or_default()", p.name)
                    }
                }
                TypeRef::Vec(inner) => {
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
                    } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.is_ref
                        && p.vec_inner_is_ref
                    {
                        if p.optional {
                            format!(
                                "{}.as_ref().map(|v| v.iter().map(String::as_str).collect::<Vec<_>>()).as_deref()",
                                p.name
                            )
                        } else {
                            format!("&{}.iter().map(String::as_str).collect::<Vec<_>>()", p.name)
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
                TypeRef::Map(_, _) => {
                    if promoted {
                        format!("{}{}", p.name, unwrap_suffix)
                    } else if p.is_mut && p.optional {
                        format!("{}.as_mut()", p.name)
                    } else if p.is_mut {
                        format!("&mut {}", p.name)
                    } else if p.is_ref && p.optional {
                        format!("{}.as_ref()", p.name)
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
                        format!("{}.as_deref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
            }
        })
        .collect()
}

/// Build call argument expressions with primitive type casting for backends that remap
/// numeric types (e.g. extendr maps `f32`/`usize`/`u64` to `f64` and `u32` to `i32`).
///
/// For `TypeRef::Primitive` params whose binding type differs from the core type, emits
/// `name as core_ty` (or `.map(|v| v as core_ty)` for optional params). All other params
/// fall back to the same logic as `gen_call_args`.
pub fn gen_call_args_cfg(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
            let unwrap_suffix = if promoted && p.optional {
                format!(".expect(\"'{}' is required\")", p.name)
            } else {
                String::new()
            };
            if p.newtype_wrapper.is_some() {
                return gen_call_args(std::slice::from_ref(p), opaque_types);
            }
            if let TypeRef::Primitive(prim) = &p.ty {
                let core_ty = core_prim_str(prim);
                let needs_cast =
                    (cast_uints_to_i32 && needs_i32_cast(prim)) || (cast_large_ints_to_f64 && needs_f64_cast(prim));
                if needs_cast {
                    return if p.optional {
                        format!("{}.map(|v| v as {core_ty})", p.name)
                    } else if promoted {
                        format!("({}{}) as {core_ty}", p.name, unwrap_suffix)
                    } else {
                        format!("{} as {core_ty}", p.name)
                    };
                }
            }
            gen_call_args(std::slice::from_ref(p), opaque_types)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build call argument expressions using pre-bound let bindings for non-opaque Named params.
/// Non-opaque Named params use `&{name}_core` references instead of `.into()`.
///
/// Json params are passed through unchanged — appropriate for backends whose binding Json type
/// is already `serde_json::Value`/`JsValue` (NAPI, WASM). Backends whose binding Json type is a
/// `String` (PyO3, extendr, Magnus) must use [`gen_call_args_with_let_bindings_json_str`] so the
/// String is parsed into `serde_json::Value` at the call site.
pub fn gen_call_args_with_let_bindings(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    gen_call_args_with_let_bindings_inner(params, opaque_types, false, false, false).join(", ")
}

/// Like [`gen_call_args_with_let_bindings`] but converts `String`-typed Json params into
/// `serde_json::Value` via `serde_json::from_str(...)` at the call site. Use this for backends
/// that map Json to `String` in binding signatures (PyO3, extendr, Magnus).
pub fn gen_call_args_with_let_bindings_json_str(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    gen_call_args_with_let_bindings_json_str_vec(params, opaque_types).join(", ")
}

/// Per-parameter form of [`gen_call_args_with_let_bindings_json_str`]. Use this when each
/// expression must be paired with its source param (e.g. struct-literal field inits) so there is no
/// need to re-split a comma-joined string.
pub fn gen_call_args_with_let_bindings_json_str_vec(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
) -> Vec<String> {
    gen_call_args_with_let_bindings_inner(params, opaque_types, true, false, false)
}

/// Like [`gen_call_args_with_let_bindings_json_str_vec`] but additionally casts primitive params
/// whose binding type was remapped back to the core type (`cast_uints_to_i32`: u8/u16/u32/i8/i16 →
/// i32; `cast_large_ints_to_f64`: u64/usize/isize/f32 → f64). Use this for backends that remap
/// numerics (extendr) when each expression must be paired with its source param — e.g. building a
/// `field: <expr>` core struct-literal so there is no need to re-split a comma-joined string.
pub fn gen_call_args_with_let_bindings_json_str_cast_vec(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> Vec<String> {
    gen_call_args_with_let_bindings_inner(params, opaque_types, true, cast_uints_to_i32, cast_large_ints_to_f64)
}

fn gen_call_args_with_let_bindings_inner(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    json_from_str: bool,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> Vec<String> {
    params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
            // Only emit `.expect()` when the core param type is itself `Option<T>`
            // calling `.expect()` on it would be a type error.
            let unwrap_suffix = if promoted && p.optional {
                format!(".expect(\"'{}' is required\")", p.name)
            } else {
                String::new()
            };
            if let TypeRef::Primitive(prim) = &p.ty {
                let needs_cast =
                    (cast_uints_to_i32 && needs_i32_cast(prim)) || (cast_large_ints_to_f64 && needs_f64_cast(prim));
                if needs_cast {
                    let core_ty = core_prim_str(prim);
                    return if p.optional {
                        format!("{}.map(|v| v as {core_ty})", p.name)
                    } else if promoted {
                        format!("({}{}) as {core_ty}", p.name, unwrap_suffix)
                    } else {
                        format!("{} as {core_ty}", p.name)
                    };
                }
            }
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
                        format!("{}_core", p.name)
                    } else if p.is_mut {
                        format!("&mut {}_core", p.name)
                    } else if p.is_ref {
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
                        if p.is_ref {
                            format!("&{}{}", p.name, unwrap_suffix)
                        } else {
                            format!("{}{}", p.name, unwrap_suffix)
                        }
                    } else {
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
                TypeRef::Json if json_from_str => {
                    if p.optional {
                        format!("{}.as_ref().and_then(|s| serde_json::from_str(s).ok())", p.name)
                    } else if promoted {
                        format!("serde_json::from_str(&{}{}).unwrap_or_default()", p.name, unwrap_suffix)
                    } else {
                        format!("serde_json::from_str(&{}).unwrap_or_default()", p.name)
                    }
                }
                TypeRef::Vec(inner) => {
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
                        if p.optional && p.is_ref {
                            format!("{}_core.as_deref()", p.name)
                        } else if p.optional {
                            format!("{}_core", p.name)
                        } else if p.is_ref {
                            format!("&{}_core", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.is_ref
                        && p.vec_inner_is_ref
                    {
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
                TypeRef::Map(_, _) => {
                    let to_btree = |expr: String| {
                        if p.map_is_btree {
                            format!("{expr}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
                        } else {
                            expr
                        }
                    };
                    if promoted {
                        let owned = to_btree(format!("{}.unwrap_or_default()", p.name));
                        if p.is_ref { format!("&{owned}") } else { owned }
                    } else if p.is_ref && p.optional {
                        format!("{}.as_ref()", p.name)
                    } else if p.is_ref {
                        format!("&{}", to_btree(p.name.clone()))
                    } else {
                        to_btree(p.name.clone())
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
        .collect()
}

/// Like `gen_call_args_with_let_bindings` but additionally handles opaque Named params that are
/// mutex-wrapped and passed as `&mut` (i.e. `is_ref=true && is_mut=true`).
///
/// For such params the call argument must be `&mut *{name}.inner.lock().unwrap()` rather than
/// the plain `&{name}.inner` emitted by the base function.
pub fn gen_call_args_with_let_bindings_mutex(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    gen_call_args_with_let_bindings_mutex_inner(params, opaque_types, mutex_types, false, false, false)
}

/// Like [`gen_call_args_with_let_bindings_mutex`] but parses `String`-typed Json params into
/// `serde_json::Value` at the call site (for PyO3/extendr/Magnus free functions). The
/// `cast_uints_to_i32`/`cast_large_ints_to_f64` flags mirror the backend's numeric remapping so
/// primitive params are cast back to the core type even on this (let-binding) call-arg path.
pub fn gen_call_args_with_let_bindings_mutex_json_str(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    gen_call_args_with_let_bindings_mutex_inner(
        params,
        opaque_types,
        mutex_types,
        true,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
    )
}

fn gen_call_args_with_let_bindings_mutex_inner(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    json_from_str: bool,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> String {
    let base = gen_call_args_with_let_bindings_inner(
        params,
        opaque_types,
        json_from_str,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
    )
    .join(", ");

    let mut patched = base;
    for p in params {
        if let TypeRef::Named(type_name) = &p.ty {
            if opaque_types.contains(type_name.as_str())
                && mutex_types.contains(type_name.as_str())
                && p.is_ref
                && p.is_mut
                && !p.optional
            {
                let old_expr = format!("&{}.inner", p.name);
                let new_expr = format!("&mut *{}.inner.lock().unwrap()", p.name);
                patched = patched.replace(&old_expr, &new_expr);
            }
        }
    }
    patched
}
