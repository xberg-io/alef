//! Emits the mutable-default-construction path for struct wrapper `new()` methods.
//!
//! When a struct needs serde-based field assignment (has_serde=true, or has
//! `Vec<non-primitive>` fields, or non-serde String-like fields), `emit_type_wrapper`
//! delegates here. The emitted code creates a `Default` instance and assigns each
//! field individually via serde JSON round-trips and native unwrapping.

use crate::backends::swift::gen_rust_crate::feature_gate;
use crate::backends::swift::gen_rust_crate::type_bridge::{needs_json_bridge, swift_bridge_rust_type};
use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

fn is_explicitly_excluded(ty: &TypeDef, field: &FieldDef, exclude_fields: &HashSet<String>) -> bool {
    let field_key = format!("{}.{}", ty.name, field.name.to_snake_case());
    exclude_fields.contains(&field_key)
}

/// Emit the body of a `new()` constructor that routes through `Default` + field assignment.
///
/// Returns the lines that go inside the `fn new(…)` body, *not* including the opening/
/// closing braces of the `impl` block — the caller writes those.
pub(crate) fn emit_default_construction_body(
    ty: &TypeDef,
    source_path: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&crate::backends::swift::template_env::render(
        "default_construction_let_mut.jinja",
        minijinja::context! {
            source_path => source_path,
        },
    ));
    for f in &ty.fields {
        if !feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features) {
            continue;
        }
        let name = f.name.to_snake_case();
        let param = crate::core::keywords::swift_ident(&name);
        if f.binding_excluded {
            continue;
        }
        if is_explicitly_excluded(ty, f, exclude_fields) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_excluded_comment.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
            continue;
        }
        let excluded_inner: Option<&str> = if needs_json_bridge(&f.ty) {
            match &f.ty {
                TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n)
                        if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()) =>
                    {
                        Some(n.as_str())
                    }
                    _ => None,
                },
                TypeRef::Named(n) if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()) => {
                    Some(n.as_str())
                }
                _ => None,
            }
        } else {
            None
        };
        if excluded_inner.is_some() {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_inner_excluded.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
        } else if needs_json_bridge(&f.ty) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_json_bridge_read.jinja",
                minijinja::context! {
                    param => &param,
                    name => &name,
                },
            ));
        } else if let TypeRef::Named(n) = &f.ty {
            let is_enum = enum_names.contains(n.as_str());
            if is_enum {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_enum_assign.jinja",
                    minijinja::context! {
                        name => &name,
                        type_name => n,
                    },
                ));
            } else if f.optional {
                if f.is_boxed {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_boxed_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_arc_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_plain_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                }
            } else if f.is_boxed {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_boxed_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_arc_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_plain_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            }
        } else if let TypeRef::Vec(inner) = &f.ty {
            if let TypeRef::Named(inner_n) = inner.as_ref() {
                let is_enum = enum_names.contains(inner_n.as_str());
                if is_enum {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_named_enum_skip.jinja",
                        minijinja::context! {
                            name => &name,
                            inner_name => inner_n,
                        },
                    ));
                } else {
                    let unwrap_expr = match f.vec_inner_core_wrapper {
                        CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                        _ => "w.0".to_string(),
                    };
                    if f.optional {
                        out.push_str(&crate::backends::swift::template_env::render(
                            "default_field_vec_named_unwrap.jinja",
                            minijinja::context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    } else {
                        out.push_str(&crate::backends::swift::template_env::render(
                            "default_field_vec_named_unwrap_plain.jinja",
                            minijinja::context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    }
                }
            } else if ty.has_serde && !f.sanitized {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_vec_serde_round_trip.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
                if f.sanitized && ty.has_serde {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_serde_round_trip.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_primitive_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                }
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_vec_non_primitive_comment.jinja",
                    minijinja::context! {
                        name => &name,
                    },
                ));
            }
        } else if matches!(f.ty, TypeRef::Char) {
            if !ty.has_serde {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_non_serde_comment.jinja",
                    minijinja::context! { name => &name },
                ));
            } else if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_optional_char_assign.jinja",
                    minijinja::context! {
                        name => &name,
                        param => &param,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_char_assign.jinja",
                    minijinja::context! {
                        name => &name,
                        param => &param,
                    },
                ));
            }
        } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json) {
            if !ty.has_serde {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_non_serde_comment.jinja",
                    minijinja::context! { name => &name },
                ));
            } else if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_optional_serde.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_serde.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            }
        } else if matches!(f.ty, TypeRef::Bytes) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_bytes_assign.jinja",
                minijinja::context! { name => &name },
            ));
        } else if matches!(f.ty, TypeRef::Duration) {
            if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_optional_duration_assign.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_duration_assign.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            }
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_generic_assign.jinja",
                minijinja::context! { name => &name, param => &param },
            ));
        }
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "dc_construct_target.jinja",
        minijinja::context! { ty_name => &ty.name },
    ));
    out
}

/// Build the field initializer list used in the direct struct literal construction path.
///
/// Only called when `needs_default_construction` is false: all fields can be constructed
/// directly from the bridge parameter without going through a Default instance.
pub(crate) fn emit_direct_field_inits(
    ty: &TypeDef,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> Vec<String> {
    ty.fields
        .iter()
        .map(|f| {
            let name = f.name.to_snake_case();
            if !feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features) {
                return format!("            {name}: ::std::default::Default::default()");
            }
            if f.binding_excluded {
                return format!("            {name}: ::std::default::Default::default()");
            }
            if is_explicitly_excluded(ty, f, exclude_fields) {
                return format!("            {name}: ::std::default::Default::default()");
            }
            let is_excluded_inner = needs_json_bridge(&f.ty) && {
                match &f.ty {
                    TypeRef::Optional(inner) | TypeRef::Vec(inner) => matches!(inner.as_ref(),
                        TypeRef::Named(n) if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str())
                    ),
                    TypeRef::Named(n) => !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()),
                    _ => false,
                }
            };
            if is_excluded_inner {
                format!("            {name}: ::std::default::Default::default()")
            } else if needs_json_bridge(&f.ty) {
                let native_ty = swift_bridge_rust_type(&f.ty);
                let opt_ty = if f.optional { format!("Option<{native_ty}>") } else { native_ty };
                format!(
                    "            {name}: serde_json::from_str::<{opt_ty}>(&{name}).expect(\"valid JSON for {name}\")"
                )
            } else if let TypeRef::Named(n) = &f.ty {
                let is_enum = enum_names.contains(n.as_str());
                if is_enum {
                    format!("            {name}: ::std::default::Default::default()")
                } else if f.optional {
                    if matches!(f.core_wrapper, CoreWrapper::Arc) {
                        format!("            {name}: {name}.map(|w| std::sync::Arc::new(w.0))")
                    } else {
                        format!("            {name}: {name}.map(|w| w.0)")
                    }
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    format!("            {name}: std::sync::Arc::new({name}.0)")
                } else {
                    format!("            {name}: {name}.0")
                }
            } else if let TypeRef::Vec(inner) = &f.ty {
                if let TypeRef::Named(inner_n) = inner.as_ref() {
                    let is_enum = enum_names.contains(inner_n.as_str());
                    if is_enum {
                        format!("            {name}: ::std::default::Default::default()")
                    } else {
                        let unwrap_expr = match f.vec_inner_core_wrapper {
                            CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                            _ => "w.0".to_string(),
                        };
                        if f.optional {
                            format!("            {name}: {name}.map(|v| v.into_iter().map(|w| {unwrap_expr}).collect())")
                        } else {
                            format!("            {name}: {name}.into_iter().map(|w| {unwrap_expr}).collect()")
                        }
                    }
                } else if f.sanitized && ty.has_serde && matches!(inner.as_ref(), TypeRef::Primitive(_)) {
                    if f.optional {
                        format!(
                            "            {name}: {name}.and_then(|v| ::serde_json::to_value(v).ok()).and_then(|j| ::serde_json::from_value(j).ok())"
                        )
                    } else {
                        format!(
                            "            {name}: ::serde_json::to_value({name}).ok().and_then(|j| ::serde_json::from_value(j).ok()).unwrap_or_default()"
                        )
                    }
                } else {
                    format!("            {name}")
                }
            } else if matches!(f.ty, TypeRef::Char) {
                if !ty.has_serde {
                    format!("            {name}: ::std::default::Default::default()")
                } else if f.optional {
                    format!("            {name}: {name}.as_ref().and_then(|s| s.chars().next())")
                } else {
                    format!("            {name}: {name}.chars().next().unwrap_or('\\0')")
                }
            } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json) {
                if !ty.has_serde {
                    format!("            {name}: ::std::default::Default::default()")
                } else if f.optional {
                    format!(
                        "            {name}: {name}.and_then(|s| serde_json::from_str(&s).ok().or_else(|| serde_json::from_value(::serde_json::Value::String(s)).ok()))"
                    )
                } else {
                    format!(
                        "            {name}: serde_json::from_str(&{name}).ok().or_else(|| serde_json::from_value::<_>(::serde_json::Value::String({name}.clone())).ok()).unwrap_or_else(|| panic!(\"failed to deserialize {name}\"))"
                    )
                }
            } else if matches!(f.ty, TypeRef::Bytes) {
                format!("            {name}: {name}.into()")
            } else if matches!(f.ty, TypeRef::Duration) {
                if f.optional {
                    format!("            {name}: {name}.map(std::time::Duration::from_millis)")
                } else {
                    format!("            {name}: std::time::Duration::from_millis({name})")
                }
            } else {
                format!("            {name}")
            }
        })
        .collect()
}
