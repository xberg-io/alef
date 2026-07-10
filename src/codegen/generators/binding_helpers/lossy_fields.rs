use crate::codegen::conversions::helpers::{core_prim_str, needs_f64_cast, needs_i32_cast};
use crate::core::ir::{CoreWrapper, TypeDef, TypeRef};
use ahash::AHashSet;

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
    skip_types: &[String],
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        false,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
        skip_types,
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
    skip_types: &[String],
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        true,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
        skip_types,
    )
}

#[allow(clippy::too_many_arguments)]
fn gen_lossy_binding_to_core_fields_inner(
    typ: &TypeDef,
    core_import: &str,
    needs_mut: bool,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
    skip_types: &[String],
) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let mut_kw = if needs_mut { "mut " } else { "" };

    if typ.has_lifetime_params {
        return format!("let {mut_kw}core_self = {core_path}::from(self.clone());\n        ");
    }

    // clippy::needless_update because the trailer is intentionally emitted even
    let allow = if typ.has_stripped_cfg_fields || typ.has_default {
        "#[allow(clippy::needless_update)]\n        "
    } else {
        ""
    };
    let mut out = format!("{allow}let {mut_kw}core_self = {core_path} {{\n");
    let core_has_default = typ.has_default;
    for field in &typ.fields {
        if field.binding_excluded {
            if !core_has_default {
                out.push_str(&crate::codegen::template_env::render(
                    "binding_helpers/struct_field_default.jinja",
                    minijinja::context! {
                        name => &field.name,
                    },
                ));
                out.push('\n');
                continue;
            }
            continue;
        }
        if field.cfg.is_some() {
            continue;
        }
        let name = &field.name;
        if field.sanitized && field.core_wrapper != CoreWrapper::Cow {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/struct_field_default.jinja",
                minijinja::context! {
                    name => &field.name,
                },
            ));
            out.push('\n');
            continue;
        }
        let is_opaque_named = match &field.ty {
            TypeRef::Named(n) => opaque_types.contains(n.as_str()),
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if opaque_types.contains(n.as_str()))
            }
            _ => false,
        };
        if is_opaque_named {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/struct_field_default.jinja",
                minijinja::context! {
                    name => &field.name,
                },
            ));
            out.push('\n');
            continue;
        }
        let is_skip_named = match &field.ty {
            TypeRef::Named(n) => skip_types.contains(n),
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if skip_types.contains(n))
            }
            _ => false,
        };
        if is_skip_named {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/default_field.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
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
                    format!("self.{name}.map(std::time::Duration::from_millis).unwrap_or_default()")
                } else {
                    format!("std::time::Duration::from_millis(self.{name})")
                }
            }
            TypeRef::String => {
                if matches!(field.core_wrapper, CoreWrapper::Cow | CoreWrapper::Box) {
                    if field.optional {
                        format!("self.{name}.clone().map(Into::into)")
                    } else {
                        format!("self.{name}.clone().into()")
                    }
                } else {
                    format!("self.{name}.clone()")
                }
            }
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
                        format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(Into::into).collect()")
                    }
                }
                TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                    }
                }
                TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                    }
                }
                _ => format!("self.{name}.clone()"),
            },
            TypeRef::Optional(inner) => {
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
                    TypeRef::Vec(vi) => match vi.as_ref() {
                        TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                        }
                        TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                        }
                        _ => format!("self.{name}.clone()"),
                    },
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
                TypeRef::Named(_) => {
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
                    }
                }
                TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect()")
                    }
                }
                TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect()")
                    }
                }
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
                if field.optional {
                    format!("self.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())")
                } else {
                    format!("serde_json::from_str(&self.{name}).unwrap_or_default()")
                }
            }
        };
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
        out.push_str(&crate::codegen::template_env::render(
            "binding_helpers/struct_field_line.jinja",
            minijinja::context! {
                name => &field.name,
                expr => &expr,
            },
        ));
        out.push('\n');
    }
    // cfg-stripped trailer stays unconditional because the `#[cfg(...)]` gates
    if typ.has_stripped_cfg_fields || typ.has_default {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str("        };\n        ");
    out
}
