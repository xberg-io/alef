//! Emits the mutable-default-construction path for struct wrapper `new()` methods.
//!
//! When a struct needs serde-based field assignment (has_serde=true, or has
//! Vec<non-primitive> fields, or non-serde String-like fields), `emit_type_wrapper`
//! delegates here. The emitted code creates a `Default` instance and assigns each
//! field individually via serde JSON round-trips and native unwrapping.

use crate::gen_rust_crate::type_bridge::{needs_json_bridge, swift_bridge_rust_type};
use alef_core::ir::{CoreWrapper, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

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
) -> String {
    let mut out = String::new();
    out.push_str(&crate::template_env::render(
        "default_construction_let_mut.jinja",
        minijinja::context! {
            source_path => source_path,
        },
    ));
    for f in &ty.fields {
        let name = f.name.to_snake_case();
        // Param name in the constructor signature is keyword-escaped (matches
        // wrappers.rs / extern_block.rs). Field access on `__target` uses the
        // unescaped Rust field name.
        let param = alef_core::keywords::swift_ident(&name);
        // Explicitly excluded fields: leave at Default::default() silently.
        let field_key = format!("{}.{}", ty.name, name);
        if exclude_fields.contains(&field_key) {
            out.push_str(&crate::template_env::render(
                "default_field_excluded_comment.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
            continue;
        }
        // Check if the inner Named type (if any) is excluded or lacks serde.
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
            // The inner type is excluded (e.g. InternalDocument — no serde derive).
            // Leave the field at its Default value; the serde bridge can't work.
            out.push_str(&crate::template_env::render(
                "default_field_inner_excluded.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
        } else if needs_json_bridge(&f.ty) {
            // JSON-decode into a serde_json::Value, then assign as JSON-deserialized
            // typed value via reinterpret.
            out.push_str(&crate::template_env::render(
                "default_field_json_bridge_read.jinja",
                minijinja::context! {
                    param => &param,
                    name => &name,
                },
            ));
        } else if let TypeRef::Named(n) = &f.ty {
            // Enum wrappers only have From<kreuzberg::T> for BridgeT (not the reverse),
            // so we cannot convert a bridge enum back to the source type via .into().
            // For struct newtypes, use .0; for enums, leave at Default.
            // The constructor param is still accepted (so the API is stable) but
            // the value is dropped for enum fields. This is a known limitation.
            let is_enum = enum_names.contains(n.as_str());
            if is_enum {
                // alef: enum fields in constructors are not converted back — leave at default
                out.push_str(&crate::template_env::render(
                    "default_field_enum_assign.jinja",
                    minijinja::context! {
                        name => &name,
                        type_name => n,
                    },
                ));
            } else if f.optional {
                // Optional Named field; wrap in Some(w.0), Box::new, or Arc::new if needed.
                if f.is_boxed {
                    out.push_str(&crate::template_env::render(
                        "default_field_optional_boxed_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    out.push_str(&crate::template_env::render(
                        "default_field_optional_arc_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "default_field_optional_plain_assign.jinja",
                        minijinja::context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                }
            } else if f.is_boxed {
                // The source field is Box<T>; wrap in Box::new().
                out.push_str(&crate::template_env::render(
                    "default_field_boxed_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                // The source field is Arc<T>; wrap in Arc::new().
                out.push_str(&crate::template_env::render(
                    "default_field_arc_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "default_field_plain_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            }
        } else if let TypeRef::Vec(inner) = &f.ty {
            // Vec<Named> fields: unwrap bridge wrappers element-wise.
            // Enum elements: same limitation as above — leave at default.
            if let TypeRef::Named(inner_n) = inner.as_ref() {
                let is_enum = enum_names.contains(inner_n.as_str());
                if is_enum {
                    out.push_str(&crate::template_env::render(
                        "default_field_vec_named_enum_skip.jinja",
                        minijinja::context! {
                            name => &name,
                            inner_name => inner_n,
                        },
                    ));
                } else {
                    // When the source field is Vec<Arc<T>>, wrap each element in Arc::new().
                    let unwrap_expr = match f.vec_inner_core_wrapper {
                        CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                        _ => "w.0".to_string(),
                    };
                    if f.optional {
                        out.push_str(&crate::template_env::render(
                            "default_field_vec_named_unwrap.jinja",
                            minijinja::context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    } else {
                        out.push_str(&crate::template_env::render(
                            "default_field_vec_named_unwrap_plain.jinja",
                            minijinja::context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    }
                }
            } else if ty.has_serde {
                // Vec<non-Named> field in a serde struct. The IR may have mapped
                // Vec<Paragraph> to Vec<String>, Vec<T> to Option<Vec<T>>, etc.
                // Use serde JSON round-trip WITHOUT a type annotation so that the
                // target field type is inferred from __target.{name}. This handles
                // Vec→Option<Vec>, Vec<String>→Vec<OtherType>, etc. gracefully:
                // the deserialized JSON is coerced to whatever type kreuzberg uses.
                out.push_str(&crate::template_env::render(
                    "default_field_vec_serde_round_trip.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
                // Vec<Primitive> or Vec<Bytes> in non-serde struct: types should match.
                out.push_str(&crate::template_env::render(
                    "default_field_vec_primitive_assign.jinja",
                    minijinja::context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else {
                // Vec<non-Primitive> in non-serde struct: actual type may differ from IR.
                // Leave at Default to avoid type mismatches.
                out.push_str(&crate::template_env::render(
                    "default_field_vec_non_primitive_comment.jinja",
                    minijinja::context! {
                        name => &name,
                    },
                ));
            }
        } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json) {
            // String-like fields may map to enum/Named types in the source struct
            // (alef's IR uses String as a fallback when the actual type can't be
            // resolved). When the struct lacks serde derives, the field type is
            // likely a non-serde type — leave at default to avoid compile errors.
            // Bytes (Vec<u8>) is excluded: bridges as Vec<u8> directly, not String.
            if !ty.has_serde {
                out.push_str(&crate::template_env::render(
                    "default_field_string_like_non_serde_comment.jinja",
                    minijinja::context! { name => &name },
                ));
            } else if f.optional {
                out.push_str(&crate::template_env::render(
                    "default_field_string_like_optional_serde.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "default_field_string_like_serde.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            }
        } else if matches!(f.ty, TypeRef::Bytes) {
            // bytes::Bytes != Vec<u8>; convert with .into() so the assignment compiles.
            out.push_str(&crate::template_env::render(
                "default_field_bytes_assign.jinja",
                minijinja::context! { name => &name },
            ));
        } else if matches!(f.ty, TypeRef::Duration) {
            // Duration bridges as u64 (millis) but the field type is std::time::Duration.
            if f.optional {
                out.push_str(&crate::template_env::render(
                    "default_field_optional_duration_assign.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "default_field_duration_assign.jinja",
                    minijinja::context! { param => &param, name => &name },
                ));
            }
        } else {
            out.push_str(&crate::template_env::render(
                "default_field_generic_assign.jinja",
                minijinja::context! { name => &name, param => &param },
            ));
        }
    }
    out.push_str(&crate::template_env::render(
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
) -> Vec<String> {
    ty.fields
        .iter()
        .map(|f| {
            let name = f.name.to_snake_case();
            // Explicitly excluded fields: leave at Default::default().
            let field_key = format!("{}.{}", ty.name, name);
            if exclude_fields.contains(&field_key) {
                return format!("            {name}: ::std::default::Default::default()");
            }
            // If the JSON-bridged field contains an excluded/no-serde Named type, skip it.
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
                // Field type contains an excluded Named type that doesn't impl serde.
                // Use Default::default() for the field rather than failing to compile.
                format!("            {name}: ::std::default::Default::default()")
            } else if needs_json_bridge(&f.ty) {
                let native_ty = swift_bridge_rust_type(&f.ty);
                let opt_ty = if f.optional { format!("Option<{native_ty}>") } else { native_ty };
                format!(
                    "            {name}: serde_json::from_str::<{opt_ty}>(&{name}).expect(\"valid JSON for {name}\")"
                )
            } else if let TypeRef::Named(n) = &f.ty {
                // Enum wrappers only have From<kreuzberg::T> for BridgeT (not the reverse).
                // For struct newtypes use .0; for enum types leave at Default.
                let is_enum = enum_names.contains(n.as_str());
                if is_enum {
                    // Enum fields can't be reverse-converted — use Default
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
                // Vec<Named> — unwrap bridge wrappers element-wise
                if let TypeRef::Named(inner_n) = inner.as_ref() {
                    let is_enum = enum_names.contains(inner_n.as_str());
                    if is_enum {
                        // Vec<EnumT> fields: enum reverse-conversion not generated — use Default
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
                } else {
                    format!("            {name}")
                }
            } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json) {
                // String-like fields are serde-deserialized from the bridge String.
                // Bytes (Vec<u8>) is excluded: it bridges as Vec<u8> directly, not as String.
                // When the struct doesn't have serde derives, the source field
                // might be a non-String type that was mapped to String by the IR
                // (e.g. HeaderFooterType). Avoid serde-based deserialization for
                // non-serde structs — leave the field at Default.
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
                // bytes::Bytes != Vec<u8>; convert with .into().
                format!("            {name}: {name}.into()")
            } else if matches!(f.ty, TypeRef::Duration) {
                // Duration bridges as u64 (millis); convert back to std::time::Duration.
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
