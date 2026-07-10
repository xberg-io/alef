use std::collections::HashSet;

use super::conversions::primitive_name;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};

pub(super) fn compute_types_containing_sanitized(
    api: &ApiSurface,
    direct_sanitized: &HashSet<String>,
    exclude_types: &HashSet<String>,
) -> HashSet<String> {
    let struct_by_name: std::collections::HashMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait && !t.is_opaque)
        .map(|t| (t.name.as_str(), t))
        .collect();
    let enum_by_name: std::collections::HashMap<&str, &EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| (e.name.as_str(), e))
        .collect();

    let mut result: HashSet<String> = direct_sanitized.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for ty in struct_by_name.values() {
            if result.contains(&ty.name) {
                continue;
            }
            let references_sanitized = ty
                .fields
                .iter()
                .any(|f| collect_named_types(&f.ty).iter().any(|n| result.contains(n)));
            if references_sanitized {
                result.insert(ty.name.clone());
                changed = true;
            }
        }
        for en in enum_by_name.values() {
            if result.contains(&en.name) {
                continue;
            }
            let references_sanitized = en.variants.iter().any(|v| {
                v.fields
                    .iter()
                    .any(|f| collect_named_types(&f.ty).iter().any(|n| result.contains(n)))
            });
            if references_sanitized {
                result.insert(en.name.clone());
                changed = true;
            }
        }
    }

    result
}

/// Compute the transitive closure of all struct/enum types reachable from
/// `seed_types` (types with sanitized fields) via non-sanitized field references.
///
/// These are the types that need `From<MirrorT> for SourceT` impls so that
/// `.into()` calls in the generated From impls for sanitized-field types work.
/// Output-only types (e.g. result structs with sanitized fields) are excluded
/// from the seed set — they're never passed as function inputs.
pub(super) fn compute_types_needing_from_impl(
    api: &ApiSurface,
    seed_types: &HashSet<String>,
    exclude_types: &HashSet<String>,
) -> HashSet<String> {
    let struct_by_name: std::collections::HashMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name) && !t.is_trait && !t.is_opaque)
        .map(|t| (t.name.as_str(), t))
        .collect();
    let enum_by_name: std::collections::HashMap<&str, &EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| (e.name.as_str(), e))
        .collect();

    let mut result: HashSet<String> = seed_types.clone();
    let mut worklist: Vec<String> = seed_types.iter().cloned().collect();

    while let Some(type_name) = worklist.pop() {
        if let Some(ty) = struct_by_name.get(type_name.as_str()) {
            for field in binding_fields(&ty.fields) {
                if field.sanitized {
                    continue;
                }
                for named in collect_named_types(&field.ty) {
                    if !result.contains(&named)
                        && (struct_by_name.contains_key(named.as_str()) || enum_by_name.contains_key(named.as_str()))
                    {
                        result.insert(named.clone());
                        worklist.push(named);
                    }
                }
            }
        } else if let Some(en) = enum_by_name.get(type_name.as_str()) {
            for variant in &en.variants {
                for field in &variant.fields {
                    if field.sanitized {
                        continue;
                    }
                    for named in collect_named_types(&field.ty) {
                        if !result.contains(&named)
                            && (struct_by_name.contains_key(named.as_str())
                                || enum_by_name.contains_key(named.as_str()))
                        {
                            result.insert(named.clone());
                            worklist.push(named);
                        }
                    }
                }
            }
        }
    }

    result
}

/// Collect all Named type names referenced (possibly nested) in a TypeRef.
fn collect_named_types(ty: &TypeRef) -> Vec<String> {
    collect_named_types_from_type_ref(ty)
}

/// Collect all Named type names referenced (possibly nested) in a TypeRef.
pub(super) fn collect_named_types_from_type_ref(ty: &TypeRef) -> Vec<String> {
    match ty {
        TypeRef::Named(name) => vec![name.clone()],
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => collect_named_types_from_type_ref(inner),
        TypeRef::Map(k, v) => {
            let mut names = collect_named_types_from_type_ref(k);
            names.extend(collect_named_types_from_type_ref(v));
            names
        }
        _ => vec![],
    }
}

fn emit_rust_struct_field(out: &mut String, cfg: Option<&str>, field_name: &str, expr: &str) {
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_struct_field_assignment.jinja",
        minijinja::context! {
            cfg => cfg,
            field_name => field_name,
            expr => expr,
        },
    ));
}

/// Emit a `From<SourceT> for T` implementation for a mirror struct.
///
/// Each field is converted using the appropriate strategy:
/// - `CoreWrapper::Cow` fields: `.into()` (Cow<'_, str> → String)
/// - `TypeRef::Json` fields: `serde_json::to_string(&v).unwrap_or_default()`
/// - `TypeRef::Named(n)` fields: `n::from(v.field)` (recursive)
/// - Other fields: `.into()` or direct copy
pub(super) fn emit_from_impl_for_struct(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    let name = &ty.name;
    let core_ty_base = if ty.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        ty.rust_path.replace('-', "_")
    };
    let core_ty = if ty.has_lifetime_params {
        format!("{core_ty_base}<'_>")
    } else {
        core_ty_base
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_core_struct_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));

    for field in binding_fields(&ty.fields) {
        if field.sanitized {
            let fallback = sanitized_field_from_expr(field);
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            emit_rust_struct_field(out, None, &field.name, &fallback);
        } else {
            let expr = field_from_expr(field, source_crate_name);
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            emit_rust_struct_field(out, None, &field.name, &expr);
        }
    }

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Build the conversion expression for one struct field (core → mirror direction).
fn field_from_expr(field: &FieldDef, source_crate_name: &str) -> String {
    let name = &field.name;
    let _ = source_crate_name;
    match &field.ty {
        TypeRef::Json => {
            if field.optional {
                format!("v.{name}.map(|j| serde_json::to_string(&j).unwrap_or_default())")
            } else {
                format!("serde_json::to_string(&v.{name}).unwrap_or_default()")
            }
        }
        TypeRef::String => {
            // `#[allow(clippy::useless_conversion)]` absorbs the `String → String`
            if field.optional {
                format!("v.{name}.map(|s| s.into())")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Char => {
            if field.optional {
                format!("v.{name}.map(|c| c.to_string())")
            } else {
                format!("v.{name}.to_string()")
            }
        }
        TypeRef::Path => {
            if field.optional {
                format!("v.{name}.map(|p| p.to_string_lossy().into_owned())")
            } else {
                format!("v.{name}.to_string_lossy().into_owned()")
            }
        }
        TypeRef::Bytes => match field.core_wrapper {
            CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                if field.optional {
                    format!("v.{name}.map(|a| (*a).clone().into())")
                } else {
                    format!("(*v.{name}).clone().into()")
                }
            }
            _ => {
                if field.optional {
                    format!("v.{name}.map(|b| b.into())")
                } else {
                    format!("v.{name}.into()")
                }
            }
        },
        TypeRef::Named(inner_name) => match field.core_wrapper {
            CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                if field.optional {
                    format!("v.{name}.map(|a| {inner_name}::from((*a).clone()))")
                } else {
                    format!("{inner_name}::from((*v.{name}).clone())")
                }
            }
            _ => {
                if field.optional && field.is_boxed {
                    format!("v.{name}.map(|b| {inner_name}::from(*b))")
                } else if field.optional {
                    format!("v.{name}.map({inner_name}::from)")
                } else if field.is_boxed {
                    format!("{inner_name}::from(*v.{name})")
                } else {
                    format!("{inner_name}::from(v.{name})")
                }
            }
        },
        TypeRef::Vec(inner) => vec_inner_from_expr(
            inner,
            &field.vec_inner_core_wrapper,
            field.newtype_wrapper.as_deref(),
            name,
            field.optional,
        ),
        TypeRef::Optional(inner) => {
            let flatten = if field.optional { ".flatten()" } else { "" };
            match inner.as_ref() {
                TypeRef::Named(inner_name) => {
                    format!("v.{name}{flatten}.map({inner_name}::from)")
                }
                TypeRef::String => {
                    // `#[allow(clippy::useless_conversion)]` for plain `String`.
                    format!("v.{name}{flatten}.map(|s| s.into())")
                }
                TypeRef::Char => {
                    format!("v.{name}{flatten}.map(|s| s.into())")
                }
                TypeRef::Path => {
                    format!("v.{name}{flatten}.map(|p| p.to_string_lossy().into_owned())")
                }
                TypeRef::Primitive(_) => {
                    format!("v.{name}{flatten}.map(|x| x as _)")
                }
                _ => format!("v.{name}{flatten}"),
            }
        }
        TypeRef::Map(k, v_ty) => map_from_expr(name, k, v_ty, field.optional, field.core_wrapper.clone()),
        TypeRef::Duration => {
            if field.optional {
                format!("v.{name}.map(|d| d.as_millis() as i64)")
            } else {
                format!("v.{name}.as_millis() as i64")
            }
        }
        TypeRef::Primitive(_) | TypeRef::Unit => {
            if let Some(_nw) = &field.newtype_wrapper {
                if field.optional {
                    format!("v.{name}.map(|x| x.0 as _)")
                } else {
                    format!("v.{name}.0 as _")
                }
            } else if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
    }
}

/// Build the Vec field conversion expression (core → mirror).
fn vec_inner_from_expr(
    inner: &TypeRef,
    vec_inner_core_wrapper: &CoreWrapper,
    field_newtype_wrapper: Option<&str>,
    name: &str,
    optional: bool,
) -> String {
    let item_conv = match (inner, vec_inner_core_wrapper) {
        (TypeRef::Named(inner_name), CoreWrapper::Arc | CoreWrapper::ArcMutex) => {
            format!("|a| {inner_name}::from((*a).clone())")
        }
        (TypeRef::Named(inner_name), _) => {
            format!("{inner_name}::from")
        }
        (TypeRef::String, _) => {
            // friends) compile — the crate-level `#[allow(clippy::useless_conversion)]`
            "|s| s.into()".to_string()
        }
        (TypeRef::Char, _) => "|s| s.into()".to_string(),
        (TypeRef::Json, _) => "|j| serde_json::to_string(&j).unwrap_or_default()".to_string(),
        (TypeRef::Path, _) => "|p: std::path::PathBuf| p.to_string_lossy().into_owned()".to_string(),
        (TypeRef::Bytes, CoreWrapper::Arc | CoreWrapper::ArcMutex) => "|a| (*a).clone().into()".to_string(),
        (TypeRef::Bytes, _) => "|b| b.into()".to_string(),
        (TypeRef::Primitive(_), _) => {
            if field_newtype_wrapper.is_some() {
                "|x| x.0 as _".to_string()
            } else {
                "|x| x as _".to_string()
            }
        }
        (TypeRef::Vec(inner2), _) => match inner2.as_ref() {
            TypeRef::Primitive(_) => {
                return if optional {
                    format!(
                        "v.{name}.map(|vec| vec.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect::<Vec<_>>()).collect::<Vec<_>>())"
                    )
                } else {
                    format!(
                        "v.{name}.into_iter().map(|inner| inner.into_iter().map(|x| x as _).collect::<Vec<_>>()).collect::<Vec<_>>()"
                    )
                };
            }
            _ => {
                return format!("v.{name}");
            }
        },
        _ => {
            return format!("v.{name}");
        }
    };

    if optional {
        format!("v.{name}.map(|vec| vec.into_iter().map({item_conv}).collect::<Vec<_>>())")
    } else {
        format!("v.{name}.into_iter().map({item_conv}).collect::<Vec<_>>()")
    }
}

/// Emit `From<MirrorT> for SourceT` for types with sanitized fields.
///
/// This is the mirror-to-core direction, required by bridge functions that accept a
/// `MirrorT` parameter and need to call the core function with SourceT.
/// Transmute is unsound for these types because sanitized fields (e.g. `Option<String>`
/// substituted for `Option<CancellationToken>`) have different memory sizes than the
/// corresponding core field, making the transmute layout assumption false.
///
/// Non-sanitized fields use field_from_expr_to_core (the inverse of field_from_expr).
/// Sanitized fields use `Default::default()` since they represent types that cannot
/// be meaningfully passed from Dart (e.g. CancellationToken, ConcurrencyConfig).
pub(super) fn emit_from_mirror_to_core_struct(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    let name = &ty.name;
    let core_ty = if ty.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        ty.rust_path.replace('-', "_")
    };

    if ty.has_private_fields {
        let mut assignments = Vec::new();
        for field in &ty.fields {
            if field.binding_excluded {
                continue;
            }
            let safe_sanitized_string = matches!(field.ty, TypeRef::String) && field.core_wrapper == CoreWrapper::Cow;
            if field.sanitized && !safe_sanitized_string {
                continue;
            }
            assignments.push(crate::codegen::conversions::construction::FieldAssign {
                core_field: field.name.clone(),
                expr: field_from_expr_to_core(field, source_crate_name),
            });
        }
        out.push_str(&crate::codegen::conversions::construction::gen_private_field_from_impl(
            &crate::codegen::conversions::construction::PrivateFieldImpl {
                core_path: &core_ty,
                binding_name: name,
                param: "v",
                has_default: ty.has_default,
                assignments: &assignments,
                allow_attrs: &[
                    "clippy::field_reassign_with_default, clippy::let_and_return, clippy::useless_conversion",
                ],
            },
        ));
        return;
    }

    let needs_default_spread = ty.has_default;
    if needs_default_spread {
        out.push_str("#[allow(clippy::needless_update)]\n");
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_mirror_struct_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));

    for field in &ty.fields {
        if field.binding_excluded {
            if !ty.has_default {
                emit_rust_struct_field(out, None, &field.name, "Default::default()");
                continue;
            }
            continue;
        }
        let safe_sanitized_string = matches!(field.ty, TypeRef::String) && field.core_wrapper == CoreWrapper::Cow;
        if field.sanitized && !safe_sanitized_string {
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            emit_rust_struct_field(out, None, &field.name, "Default::default()");
        } else {
            let expr = field_from_expr_to_core(field, source_crate_name);
            // at compile time. Emitting `#[cfg(...)]` here would gate on the dart
            emit_rust_struct_field(out, None, &field.name, &expr);
        }
    }

    if needs_default_spread {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Emit a `From<MirrorEnum> for SourceEnum` implementation.
///
/// Unit-only enums: simple variant match. Data enums: reconstruct each variant.
/// Build the conversion expression for one struct field in the mirror-to-core direction.
/// This is the inverse of `field_from_expr` (which handles core-to-mirror).
fn field_from_expr_to_core(field: &FieldDef, _source_crate_name: &str) -> String {
    let name = &field.name;
    match &field.ty {
        TypeRef::String => {
            // `#[allow(clippy::useless_conversion)]` absorbs the `String → String`
            if field.optional {
                format!("v.{name}.map(Into::into)")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Char => {
            if field.optional {
                format!("v.{name}.as_deref().and_then(|s| s.chars().next())")
            } else {
                format!("v.{name}.chars().next().unwrap_or_default()")
            }
        }
        TypeRef::Path => {
            if field.optional {
                format!("v.{name}.map(std::path::PathBuf::from)")
            } else {
                format!("std::path::PathBuf::from(v.{name})")
            }
        }
        TypeRef::Bytes => {
            if field.optional {
                format!("v.{name}.map(Into::into)")
            } else {
                format!("v.{name}.into()")
            }
        }
        TypeRef::Json => {
            if field.optional {
                format!("v.{name}.as_deref().and_then(|s| serde_json::from_str(s).ok())")
            } else {
                format!("serde_json::from_str(&v.{name}).unwrap_or_default()")
            }
        }
        TypeRef::Named(_) => match field.core_wrapper {
            CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                if field.optional {
                    format!("v.{name}.map(|x| std::sync::Arc::new(x.into()))")
                } else {
                    format!("std::sync::Arc::new(v.{name}.into())")
                }
            }
            _ if field.is_boxed => {
                if field.optional {
                    format!("v.{name}.map(|x| Box::new(x.into()))")
                } else {
                    format!("Box::new(v.{name}.into())")
                }
            }
            _ => {
                if field.optional {
                    format!("v.{name}.map(Into::into)")
                } else {
                    format!("v.{name}.into()")
                }
            }
        },
        TypeRef::Vec(inner) => {
            match inner.as_ref() {
                TypeRef::Named(_) => match field.vec_inner_core_wrapper {
                    CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                        if field.optional {
                            format!(
                                "v.{name}.map(|vec| vec.into_iter().map(|x| std::sync::Arc::new(x.into())).collect())"
                            )
                        } else {
                            format!("v.{name}.into_iter().map(|x| std::sync::Arc::new(x.into())).collect()")
                        }
                    }
                    _ => {
                        if field.optional {
                            format!("v.{name}.map(|vec| vec.into_iter().map(Into::into).collect())")
                        } else {
                            format!("v.{name}.into_iter().map(Into::into).collect()")
                        }
                    }
                },
                TypeRef::Vec(inner_inner) => match inner_inner.as_ref() {
                    TypeRef::Primitive(prim) => {
                        let target = primitive_name(prim);
                        if field.optional {
                            format!(
                                "v.{name}.map(|vv| vv.into_iter().map(|inner| inner.into_iter().map(|x| x as {target}).collect::<Vec<_>>()).collect::<Vec<_>>())"
                            )
                        } else {
                            format!(
                                "v.{name}.into_iter().map(|inner| inner.into_iter().map(|x| x as {target}).collect::<Vec<_>>()).collect::<Vec<_>>()"
                            )
                        }
                    }
                    _ => {
                        if field.optional {
                            format!(
                                "v.{name}.map(|vv| vv.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect())"
                            )
                        } else {
                            format!(
                                "v.{name}.into_iter().map(|inner| inner.into_iter().map(Into::into).collect()).collect()"
                            )
                        }
                    }
                },
                TypeRef::Primitive(prim) => {
                    let target = primitive_name(prim);
                    let elem_conv = if let Some(nw) = &field.newtype_wrapper {
                        format!("|x| {nw}(x as {target})")
                    } else {
                        format!("|x| x as {target}")
                    };
                    if field.optional {
                        format!("v.{name}.map(|vec| vec.into_iter().map({elem_conv}).collect::<Vec<_>>())")
                    } else {
                        format!("v.{name}.into_iter().map({elem_conv}).collect::<Vec<_>>()")
                    }
                }
                _ => {
                    // compiles; the crate-level `#[allow(clippy::useless_conversion)]`
                    if field.optional {
                        format!("v.{name}.map(|vec| vec.into_iter().map(Into::into).collect())")
                    } else {
                        format!("v.{name}.into_iter().map(Into::into).collect()")
                    }
                }
            }
        }
        TypeRef::Optional(inner) => {
            let wrap_some = if field.optional { ".map(Some)" } else { "" };
            match inner.as_ref() {
                TypeRef::Named(_) => format!("v.{name}.map(Into::into){wrap_some}"),
                TypeRef::String | TypeRef::Char => format!("v.{name}.map(Into::into){wrap_some}"),
                TypeRef::Path => format!("v.{name}.map(std::path::PathBuf::from){wrap_some}"),
                TypeRef::Primitive(_) => format!("v.{name}.map(|x| x as _){wrap_some}"),
                _ => format!("v.{name}{wrap_some}"),
            }
        }
        TypeRef::Primitive(_) => {
            if let Some(nw) = &field.newtype_wrapper {
                if field.optional {
                    format!("v.{name}.map(|x| {nw}(x as _))")
                } else {
                    format!("{nw}(v.{name} as _)")
                }
            } else if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
        TypeRef::Duration => {
            if field.optional {
                format!("v.{name}.map(|ms| std::time::Duration::from_millis(ms as u64))")
            } else {
                format!("std::time::Duration::from_millis(v.{name} as u64)")
            }
        }
        TypeRef::Map(_, v_ty) => {
            // the type level. Crate-level `#[allow(clippy::useless_conversion)]` absorbs
            let val_conv = match v_ty.as_ref() {
                TypeRef::Primitive(_) => "v as _",
                TypeRef::Named(_) => "v.into()",
                _ => "v.into()",
            };
            if field.optional {
                format!("v.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), {val_conv})).collect())")
            } else {
                format!("v.{name}.into_iter().map(|(k, v)| (k.into(), {val_conv})).collect()")
            }
        }
        TypeRef::Unit => "()".to_string(),
    }
}

/// Build conversion expression for a Map field (core → mirror).
/// Mirror always uses HashMap<String, String> or HashMap<String, T>.
/// Core may use BTreeMap, AHashMap, HashMap with Value values, etc.
///
/// When `core_wrapper` is `CoreWrapper::Cow` the map itself is a
/// `Cow<'_, BTreeMap<...>>` — call `.into_owned()` before `.into_iter()` to
/// consume the borrow and produce an owned `BTreeMap` that can be iterated.
fn map_from_expr(name: &str, _k: &TypeRef, v_ty: &TypeRef, optional: bool, core_wrapper: CoreWrapper) -> String {
    // `#[allow(clippy::useless_conversion)]`.
    let value_conv = match v_ty {
        TypeRef::Json => "serde_json::to_string(&v).unwrap_or_default()",
        TypeRef::Named(mirror_name) => return map_named_from_expr(name, mirror_name, optional, core_wrapper),
        TypeRef::Primitive(_) => "v as _",
        _ => "v.into()",
    };

    // crate-level `clippy::useless_conversion` allow absorbs the no-op String case.
    let iter_method = if core_wrapper == CoreWrapper::Cow {
        "into_owned().into_iter()"
    } else {
        "into_iter()"
    };
    let iter_expr = format!("{iter_method}.map(|(k, v)| (k.into(), {value_conv})).collect()");

    if optional {
        format!("v.{name}.map(|m| m.{iter_expr})")
    } else {
        format!("v.{name}.{iter_expr}")
    }
}

fn map_named_from_expr(field_name: &str, mirror_name: &str, optional: bool, core_wrapper: CoreWrapper) -> String {
    let iter_method = if core_wrapper == CoreWrapper::Cow {
        "into_owned().into_iter()"
    } else {
        "into_iter()"
    };
    let iter_expr = format!("{iter_method}.map(|(k, v)| (k.into(), {mirror_name}::from(v))).collect()");
    if optional {
        format!("v.{field_name}.map(|m| m.{iter_expr})")
    } else {
        format!("v.{field_name}.{iter_expr}")
    }
}

/// Fallback expression for sanitized fields (unknown core types mapped to String/i64).
///
/// Sanitized fields have an unknown or complex core type that was simplified in the IR.
/// We use Default::default() as a safe fallback — attempting serde_json::to_string
/// would require the type to implement Serialize, which is not guaranteed for all
/// sanitized or excluded types.
fn sanitized_field_from_expr(field: &FieldDef) -> String {
    let name = &field.name;
    match &field.ty {
        TypeRef::Primitive(_) => {
            if field.optional {
                format!("v.{name}.map(|x| x as _)")
            } else {
                format!("v.{name} as _")
            }
        }
        TypeRef::String | TypeRef::Char if field.core_wrapper == CoreWrapper::Cow => {
            if field.optional {
                format!("v.{name}.map(|s| s.into_owned())")
            } else {
                format!("v.{name}.into_owned()")
            }
        }
        _ => {
            let _ = name;
            String::from("Default::default()")
        }
    }
}
