//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_type_enum_aware_ref, is_enum_named, is_vec_of_enum, needs_json_bridge,
};
use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Returns true when the wrapper getter for `field` cannot be safely bridged
/// to swift-bridge.
///
/// Used by `extern_block::emit_extern_block_for_type` to skip the extern
/// declaration *and* by `wrappers::emit_getters` to skip the impl. Keeping
/// these in lockstep means the swift-bridge surface never contains a callable
/// function with no valid bridge implementation.
///
/// Five cases qualify:
/// 1. Explicitly excluded fields (`[swift].exclude_fields` config).
/// 2. Fields whose `#[cfg(...)]` condition is not satisfied by the configured features.
/// 3. JSON-bridged container with inner Named that is excluded from codegen
///    or marked as non-serde — round-trip cannot reconstruct the type.
/// 4. `Vec<Named>` field on a non-serde struct — IR cannot guarantee the
///    Named wrapper matches the actual Rust field type (different type may
///    appear in Rust source vs IR).
pub(crate) fn is_unbridgeable_getter(
    ty: &TypeDef,
    field: &FieldDef,
    exclude_fields: &HashSet<String>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &HashSet<&str>,
    configured_features: &std::collections::HashSet<&str>,
) -> bool {
    if !super::super::feature_gate::cfg_satisfied(field.cfg.as_deref(), configured_features) {
        return true;
    }

    let name = field.name.to_snake_case();
    let field_key = format!("{}.{}", ty.name, name);
    if field.binding_excluded || exclude_fields.contains(&field_key) {
        return true;
    }
    if needs_json_bridge(&field.ty) {
        let inner_named = match &field.ty {
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        };
        if let Some(n) = inner_named {
            if !type_paths.contains_key(n) || no_serde_names.contains(n) {
                return true;
            }
        }
    }
    if let TypeRef::Vec(inner) = &field.ty {
        if !ty.has_serde && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
            return true;
        }
        if field.sanitized && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
            return true;
        }
    }
    false
}

/// Per-field derived context reused across getter emitters.
struct GetterCtx {
    name: String,
    getter_name: String,
    bridge_ty_owned: String,
}

/// Emit getter methods for all fields of a type wrapper.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_getters(
    ty: &TypeDef,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    unit_enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    first_class_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
    out: &mut String,
) {
    let parent_first_class = first_class_names.contains(ty.name.as_str());
    for field in &ty.fields {
        let bridge_ty = bridge_type_enum_aware_ref(&field.ty, enum_names);
        let bridge_ty_owned = if field.optional && !needs_json_bridge(&field.ty) {
            if is_vec_of_enum(&field.ty, enum_names) {
                "String".to_string()
            } else {
                format!("Option<{bridge_ty}>")
            }
        } else {
            bridge_ty
        };
        let name = field.name.to_snake_case();
        let getter_name = swift_ident(&name);
        if is_unbridgeable_getter(
            ty,
            field,
            exclude_fields,
            type_paths,
            no_serde_names,
            configured_features,
        ) {
            if field.binding_excluded {
                continue;
            }
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_skip_comment.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
            continue;
        }
        let ctx = GetterCtx {
            name,
            getter_name,
            bridge_ty_owned,
        };
        if needs_json_bridge(&field.ty) {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_json_bridge.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    return_type => &ctx.bridge_ty_owned,
                    name => &ctx.name,
                },
            ));
        } else if is_enum_named(&field.ty, enum_names) {
            emit_enum_string_getter(field, &ctx, enum_names, unit_enum_names, out);
        } else if is_vec_of_enum(&field.ty, enum_names) {
            emit_vec_enum_string_getter(field, &ctx, enum_names, unit_enum_names, out);
        } else if let TypeRef::Named(wrapper) = &field.ty {
            emit_named_getter(field, wrapper, &ctx, enum_names, out);
        } else if let TypeRef::Vec(inner) = &field.ty {
            emit_vec_getter(
                ty,
                field,
                inner,
                &ctx,
                enum_names,
                no_serde_names,
                parent_first_class,
                out,
            );
        } else if matches!(
            field.ty,
            TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json
        ) {
            emit_string_like_getter(ty, field, &ctx, out);
        } else if matches!(field.ty, TypeRef::Bytes) {
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_optional_bytes.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_bytes.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else if matches!(field.ty, TypeRef::Duration) {
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_optional_duration.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_duration.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        name => &ctx.name,
                    },
                ));
            }
        } else if ty.has_serde && matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Primitive(_)) {
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_simple_clone.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    return_type => &ctx.bridge_ty_owned,
                    name => &ctx.name,
                },
            ));
        }
    }
}

/// Emit a `String`-returning getter for an enum-typed `Named` field.
///
/// Returns the opaque enum as a `String` (avoids swift-bridge's `Vec<EnumType>
/// Vectorizable` generation). The encoding depends on the enum kind:
/// - Unit enums (all variants fieldless): the bridge wrapper's `to_string()` yields the
///   bare serde raw value (e.g. `stop`), which Swift reconstructs via `Type(rawValue:)`.
/// - Tagged enums (some variant carries data): the discriminant-only wrapper drops the
///   payload, so the source value is serialized with `serde_json::to_string` and Swift
///   decodes it via `JSONDecoder` (matching the bidirectional `*_from_json` representation).
fn emit_enum_string_getter(
    field: &crate::core::ir::FieldDef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    unit_enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let TypeRef::Named(wrapper) = &field.ty else {
        return;
    };
    let is_enum = enum_names.contains(wrapper.as_str());
    debug_assert!(is_enum, "emit_enum_string_getter called with non-enum Named type");
    let is_unit = unit_enum_names.contains(wrapper.as_str());

    let name = &ctx.name;
    let getter_name = &ctx.getter_name;

    if field.optional {
        let map_expr = if is_unit {
            if field.is_boxed {
                format!("self.0.{name}.clone().map(|w| {wrapper}::from(*w).to_string())")
            } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
                format!("self.0.{name}.clone().map(|w| {wrapper}::from((*w).clone()).to_string())")
            } else {
                format!("self.0.{name}.clone().map(|w| {wrapper}::from(w).to_string())")
            }
        } else if field.is_boxed || matches!(field.core_wrapper, CoreWrapper::Arc) {
            format!(
                "self.0.{name}.clone().map(|w| serde_json::to_string(&*w).unwrap_or_else(|_| \"null\".to_string()))"
            )
        } else {
            format!("self.0.{name}.clone().map(|w| serde_json::to_string(&w).unwrap_or_else(|_| \"null\".to_string()))")
        };
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_enum_string_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                map_expr => map_expr,
            },
        ));
    } else {
        let expr = if is_unit {
            if field.is_boxed {
                format!("{wrapper}::from(*self.0.{name}.clone()).to_string()")
            } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
                format!("{wrapper}::from((*self.0.{name}).clone()).to_string()")
            } else {
                format!("{wrapper}::from(self.0.{name}.clone()).to_string()")
            }
        } else if field.is_boxed || matches!(field.core_wrapper, CoreWrapper::Arc) {
            format!("serde_json::to_string(&*self.0.{name}).unwrap_or_else(|_| \"null\".to_string())")
        } else {
            format!("serde_json::to_string(&self.0.{name}).unwrap_or_else(|_| \"null\".to_string())")
        };
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_enum_string.jinja",
            minijinja::context! {
                getter_name => getter_name,
                expr => expr,
            },
        ));
    }
}

/// Emit a `Vec<String>`-returning getter for a `Vec<Named(enum)>` field.
///
/// Maps each enum element to a `String`: the bridge wrapper's `to_string()` raw value for
/// unit enums, or `serde_json::to_string` of the source value for tagged enums (see
/// `emit_enum_string_getter` for the encoding rationale).
fn emit_vec_enum_string_getter(
    field: &crate::core::ir::FieldDef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    unit_enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let TypeRef::Vec(inner) = &field.ty else {
        return;
    };
    let TypeRef::Named(wrapper) = inner.as_ref() else {
        return;
    };
    let is_enum = enum_names.contains(wrapper.as_str());
    debug_assert!(is_enum, "emit_vec_enum_string_getter called with non-enum Vec<Named>");
    let is_unit = unit_enum_names.contains(wrapper.as_str());

    let name = &ctx.name;
    let getter_name = &ctx.getter_name;

    let elem_expr = if is_unit {
        match field.vec_inner_core_wrapper {
            CoreWrapper::Arc => format!("{wrapper}::from((**elem).clone()).to_string()"),
            _ => format!("{wrapper}::from(elem.clone()).to_string()"),
        }
    } else {
        match field.vec_inner_core_wrapper {
            CoreWrapper::Arc => "serde_json::to_string(&**elem).unwrap_or_else(|_| \"null\".to_string())".to_string(),
            _ => "serde_json::to_string(elem).unwrap_or_else(|_| \"null\".to_string())".to_string(),
        }
    };

    if field.optional {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_vec_enum_string_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                name => name,
                elem_expr => elem_expr,
            },
        ));
    } else {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_vec_enum_string.jinja",
            minijinja::context! {
                getter_name => getter_name,
                name => name,
                elem_expr => elem_expr,
            },
        ));
    }
}

/// Emit a `Vec<String>`-returning getter for a `Vec<Named(struct)>` field with serde.
///
/// Serializes each struct element to JSON string to avoid swift-bridge's broken
/// `Vec<OpaqueRustType>` Vectorizable codegen. Mirrors the enum case which already
/// avoids the same issue (see `emit_vec_enum_string_getter` comment).
///
/// The getter returns `Vec<String>` where each element is the serde JSON representation
/// of the struct. Swift can call `.count` and iterate without issues.
fn emit_vec_struct_serde_getter(field: &crate::core::ir::FieldDef, ctx: &GetterCtx, out: &mut String) {
    let name = &ctx.name;
    let getter_name = &ctx.getter_name;

    let elem_expr = "serde_json::to_string(elem).unwrap_or_else(|_| \"null\".to_string())".to_string();

    if field.optional {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_vec_enum_string_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                name => name,
                elem_expr => elem_expr,
            },
        ));
    } else {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_vec_enum_string.jinja",
            minijinja::context! {
                getter_name => getter_name,
                name => name,
                elem_expr => elem_expr,
            },
        ));
    }
}

fn emit_named_getter(
    field: &crate::core::ir::FieldDef,
    wrapper: &str,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let name = &ctx.name;
    let getter_name = &ctx.getter_name;
    let is_enum = enum_names.contains(wrapper);
    if field.optional {
        let getter_expr = if field.is_boxed {
            if is_enum {
                format!("self.0.{name}.clone().map(|w| {wrapper}::from(*w))")
            } else {
                format!("self.0.{name}.clone().map(|w| {wrapper}(*w))")
            }
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            if is_enum {
                format!("self.0.{name}.clone().map(|w| {wrapper}::from((*w).clone()))")
            } else {
                format!("self.0.{name}.clone().map(|w| {wrapper}((*w).clone()))")
            }
        } else if is_enum {
            format!("self.0.{name}.clone().map({wrapper}::from)")
        } else {
            format!("self.0.{name}.clone().map({wrapper})")
        };
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_optional_named.jinja",
            minijinja::context! {
                getter_name => getter_name,
                wrapper => wrapper,
                getter_expr => &getter_expr,
            },
        ));
    } else {
        let expr = if field.is_boxed {
            if is_enum {
                format!("{wrapper}::from(*self.0.{name}.clone())")
            } else {
                format!("{wrapper}(*self.0.{name}.clone())")
            }
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            if is_enum {
                format!("{wrapper}::from((*self.0.{name}).clone())")
            } else {
                format!("{wrapper}((*self.0.{name}).clone())")
            }
        } else if is_enum {
            format!("{wrapper}::from(self.0.{name}.clone())")
        } else {
            format!("{wrapper}(self.0.{name}.clone())")
        };
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_named.jinja",
            minijinja::context! {
                getter_name => getter_name,
                wrapper => wrapper,
                expr => &expr,
            },
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_vec_getter(
    ty: &TypeDef,
    field: &crate::core::ir::FieldDef,
    inner: &TypeRef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    parent_first_class: bool,
    out: &mut String,
) {
    let _name = &ctx.name;
    let _getter_name = &ctx.getter_name;
    let _bridge_ty_owned = &ctx.bridge_ty_owned;
    if let TypeRef::Named(wrapper) = inner {
        let is_enum = enum_names.contains(wrapper.as_str());
        let has_serde = !no_serde_names.contains(wrapper.as_str());
        if !is_enum && has_serde && (field.optional || parent_first_class) {
            emit_vec_struct_serde_getter(field, ctx, out);
        } else {
            let elem_expr = match field.vec_inner_core_wrapper {
                CoreWrapper::Arc if !is_enum => format!("{wrapper}((**elem).clone())"),
                CoreWrapper::Arc => format!("{wrapper}::from((**elem).clone())"),
                _ if is_enum => format!("{wrapper}::from(elem.clone())"),
                _ => format!("{wrapper}(elem.clone())"),
            };
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_named_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        wrapper => wrapper,
                        name => &ctx.name,
                        elem_expr => &elem_expr,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_named.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        wrapper => wrapper,
                        name => &ctx.name,
                        elem_expr => &elem_expr,
                    },
                ));
            }
        }
    } else if !matches!(inner, TypeRef::Primitive(_) | TypeRef::Bytes) {
        if ty.has_serde {
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_complex_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_complex_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_vec_complex_skip.jinja",
                minijinja::context! {
                    name => &ctx.name,
                },
            ));
        }
    } else {
        if ty.has_serde {
            if field.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_primitive_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "getter_vec_primitive_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_vec_primitive_clone.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    return_type => &ctx.bridge_ty_owned,
                    name => &ctx.name,
                },
            ));
        }
    }
}

fn emit_string_like_getter(ty: &TypeDef, field: &crate::core::ir::FieldDef, ctx: &GetterCtx, out: &mut String) {
    let name = &ctx.name;
    let getter_name = &ctx.getter_name;
    let bridge_ty_owned = &ctx.bridge_ty_owned;
    if matches!(field.ty, TypeRef::Char) {
        if field.optional {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_char_optional.jinja",
                minijinja::context! {
                    getter_name => getter_name,
                    return_type => bridge_ty_owned,
                    name => name,
                },
            ));
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_char.jinja",
                minijinja::context! {
                    getter_name => getter_name,
                    return_type => bridge_ty_owned,
                    name => name,
                },
            ));
        }
        return;
    }
    // NOTE: TypeRef::Bytes is NOT included here — it maps to Vec<u8> in the
    if !ty.has_serde {
        if field.optional {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_string_like_debug_optional.jinja",
                minijinja::context! {
                    getter_name => getter_name,
                    return_type => bridge_ty_owned,
                    name => name,
                },
            ));
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "getter_string_like_debug.jinja",
                minijinja::context! {
                    getter_name => getter_name,
                    return_type => bridge_ty_owned,
                    name => name,
                },
            ));
        }
    } else if matches!(field.ty, TypeRef::String)
        && !field.sanitized
        && matches!(field.core_wrapper, crate::core::ir::CoreWrapper::None)
    {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_simple_clone.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if matches!(field.ty, TypeRef::String)
        && matches!(field.core_wrapper, crate::core::ir::CoreWrapper::Cow)
        && !field.optional
    {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_string_cow.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if matches!(field.ty, TypeRef::String)
        && matches!(field.core_wrapper, crate::core::ir::CoreWrapper::Cow)
        && field.optional
    {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_string_cow_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if field.optional {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_string_like_serde_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else {
        out.push_str(&crate::backends::swift::template_env::render(
            "getter_string_like_serde.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    }
}
