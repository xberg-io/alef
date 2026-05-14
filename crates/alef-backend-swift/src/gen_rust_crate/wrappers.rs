//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::gen_rust_crate::default_construction::{emit_default_construction_body, emit_direct_field_inits};
use crate::gen_rust_crate::type_bridge::{
    bridge_type, bridge_type_enum_aware_ref, is_enum_named, is_vec_of_enum, needs_json_bridge, swift_bridge_rust_type,
};
use alef_codegen::generators::type_paths::resolve_type_path;
use alef_core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};
use alef_core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Returns true when the wrapper getter for `field` cannot be safely bridged
/// to swift-bridge — i.e. the only viable impl would be `unimplemented!()`.
///
/// Used by `extern_block::emit_extern_block_for_type` to skip the extern
/// declaration *and* by `wrappers::emit_getters` to skip the impl. Keeping
/// these in lockstep means the swift-bridge surface never contains a callable
/// function whose body would panic at runtime.
///
/// Three cases qualify:
/// 1. Explicitly excluded fields (`[swift].exclude_fields` config).
/// 2. JSON-bridged container with inner Named that is excluded from codegen
///    or marked as non-serde — round-trip cannot reconstruct the type.
/// 3. `Vec<Named>` field on a non-serde struct — IR cannot guarantee the
///    Named wrapper matches the actual Rust field type (different type may
///    appear in Rust source vs IR).
pub(crate) fn is_unbridgeable_getter(
    ty: &TypeDef,
    field: &FieldDef,
    exclude_fields: &HashSet<String>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &HashSet<&str>,
) -> bool {
    let name = field.name.to_snake_case();
    let field_key = format!("{}.{}", ty.name, name);
    if exclude_fields.contains(&field_key) {
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
        // Vec<non-Primitive, non-Bytes> on a non-serde struct cannot survive the
        // bridge: there's no JSON round-trip available, and the IR may have
        // sanitized the inner type away from its real Rust source counterpart.
        // This covers Vec<String>, Vec<Path>, Vec<Named>, Vec<Vec<…>>, etc.
        if !ty.has_serde && !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
            return true;
        }
    }
    false
}

pub(crate) fn emit_type_wrapper(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&ty.name, source_crate, type_paths);
    out.push_str(&crate::template_env::render(
        "struct_newtype.jinja",
        minijinja::context! {
            name => &ty.name,
            source_path => &source_path,
        },
    ));

    if !ty.fields.is_empty() {
        out.push_str(&crate::template_env::render(
            "impl_header.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));

        // Constructor — params use bridge types (String for JSON-bridged fields)
        // and Option<bridge_ty> when the field is optional.
        // Excluded fields (via exclude_fields config) are omitted from params
        // and left at Default::default() in the field initializers.
        let params: Vec<String> = ty
            .fields
            .iter()
            .filter(|f| {
                let field_key = format!("{}.{}", ty.name, f.name.to_snake_case());
                !exclude_fields.contains(&field_key)
            })
            .map(|f| {
                let bridge_ty = bridge_type(&f.ty);
                let bridge_ty = if f.optional && !needs_json_bridge(&f.ty) {
                    // Optional fields are JSON-bridged so this branch is rarely hit;
                    // when it is (a primitive Option), wrap in Option<>.
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                // Escape Swift keywords so the param name in `pub fn new()` matches
                // the extern declaration (which also escapes via swift_ident).
                let name = swift_ident(&f.name.to_snake_case());
                format!("{name}: {bridge_ty}")
            })
            .collect();

        // Determine construction strategy (see default_construction.rs for details):
        // when any field requires Default-based assignment, we cannot emit a direct struct literal.
        let has_vec_non_primitive = ty.fields.iter().any(|f| {
            matches!(&f.ty, TypeRef::Vec(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes))
        });
        let has_non_serde_string_field = !ty.has_serde
            && ty
                .fields
                .iter()
                .any(|f| matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Char));
        let needs_default_construction = ty.has_serde
            || has_vec_non_primitive
            || has_non_serde_string_field
            || ty
                .fields
                .iter()
                .any(|f| needs_json_bridge(&f.ty) || matches!(f.ty, TypeRef::Named(_)));

        if needs_default_construction && !ty.has_default {
            // The struct needs mutable-default construction but doesn't impl Default.
            // Omit the constructor entirely — swift-bridge will not expose `init()` for
            // this type, which is correct: the host language can't construct it anyway.
        } else {
            out.push_str(&crate::template_env::render(
                "fn_new_signature.jinja",
                minijinja::context! {
                    params => params.join(", "),
                    name => &ty.name,
                },
            ));

            if needs_default_construction && ty.has_default {
                let body = emit_default_construction_body(
                    ty,
                    &source_path,
                    type_paths,
                    enum_names,
                    no_serde_names,
                    exclude_fields,
                );
                out.push_str(&body);
            } else {
                let field_inits = emit_direct_field_inits(ty, type_paths, enum_names, no_serde_names, exclude_fields);
                out.push_str(&crate::template_env::render(
                    "struct_literal_open.jinja",
                    minijinja::context! {
                        name => &ty.name,
                        source_path => &source_path,
                    },
                ));
                for init in &field_inits {
                    out.push_str(init);
                    out.push_str(",\n");
                }
                out.push_str("        })\n");
            }
            out.push_str("    }\n");
        } // end else (constructor emitted)

        // Getters — return bridge types (String for JSON-bridged, wrappers for Named).
        emit_getters(ty, type_paths, enum_names, no_serde_names, exclude_fields, &mut out);

        out.push_str("}\n");
    }

    out
}

/// Per-field derived context reused across getter emitters.
struct GetterCtx {
    name: String,
    getter_name: String,
    bridge_ty_owned: String,
}

/// Emit getter methods for all fields of a type wrapper.
fn emit_getters(
    ty: &TypeDef,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    out: &mut String,
) {
    for field in &ty.fields {
        // Use enum-aware bridge type so enum-typed Named fields resolve to String.
        // This keeps extern block declarations consistent with the getter impl bodies.
        // For optional Vec<Named(enum)> fields, fall back to String (JSON-serialized)
        // because Option<Vec<String>> is not supported by swift-bridge as a getter return.
        let bridge_ty = bridge_type_enum_aware_ref(&field.ty, enum_names);
        let bridge_ty_owned = if field.optional && !needs_json_bridge(&field.ty) {
            // Option<Vec<String>> is not natively supported by swift-bridge; collapse
            // to plain String (JSON) only when the Vec inner type is an enum.  For
            // Option<Vec<Named(struct)>> keep the opaque-wrapper vector form.
            if is_vec_of_enum(&field.ty, enum_names) {
                "String".to_string()
            } else {
                format!("Option<{bridge_ty}>")
            }
        } else {
            bridge_ty
        };
        let name = field.name.to_snake_case();
        // Swift-bridge emits the Rust fn name verbatim into Swift; escape Swift
        // reserved keywords (extension, subscript, etc.) so the generated Swift
        // accessor is valid. Body still uses `name` for source-struct field access.
        let getter_name = swift_ident(&name);
        // Skip impl entirely for fields whose getter is unbridgeable. The matching
        // `extern_block::emit_extern_block_for_type` skips the extern declaration
        // for the same fields, so the swift-bridge surface stays consistent.
        if is_unbridgeable_getter(ty, field, exclude_fields, type_paths, no_serde_names) {
            out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
                "getter_json_bridge.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    return_type => &ctx.bridge_ty_owned,
                    name => &ctx.name,
                },
            ));
        } else if is_enum_named(&field.ty, enum_names) {
            // Enum-typed Named field: return String via to_string() on the wrapper enum.
            // The opaque enum type is NOT declared in the extern block (see extern_block.rs),
            // so we must not return the wrapper type here.
            emit_enum_string_getter(field, &ctx, enum_names, out);
        } else if is_vec_of_enum(&field.ty, enum_names) {
            // Vec<Named(enum)>: map each element to String via to_string().
            emit_vec_enum_string_getter(field, &ctx, enum_names, out);
        } else if let TypeRef::Named(wrapper) = &field.ty {
            emit_named_getter(field, wrapper, &ctx, enum_names, out);
        } else if let TypeRef::Vec(inner) = &field.ty {
            emit_vec_getter(ty, field, inner, &ctx, enum_names, out);
        } else if matches!(
            field.ty,
            TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json
        ) {
            emit_string_like_getter(ty, field, &ctx, out);
        } else if matches!(field.ty, TypeRef::Bytes) {
            // bytes::Bytes bridges as Vec<u8>; convert with .to_vec() for the return.
            if field.optional {
                out.push_str(&crate::template_env::render(
                    "getter_optional_bytes.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "getter_bytes.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else if matches!(field.ty, TypeRef::Duration) {
            // Duration field: bridge type is u64 (millis), core type is std::time::Duration.
            if field.optional {
                out.push_str(&crate::template_env::render(
                    "getter_optional_duration.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "getter_duration.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        name => &ctx.name,
                    },
                ));
            }
        } else if ty.has_serde && matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Primitive(_)) {
            // Vec<T> or Primitive fields in serde structs: use serde JSON round-trip.
            if field.optional {
                out.push_str(&crate::template_env::render(
                    "getter_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "getter_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            out.push_str(&crate::template_env::render(
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
/// Instead of returning the opaque enum wrapper (which would trigger swift-bridge's
/// `Vec<EnumType> Vectorizable` generation), this converts the enum to `String` by
/// constructing the bridge wrapper enum and calling `.to_string()`.
fn emit_enum_string_getter(
    field: &alef_core::ir::FieldDef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let TypeRef::Named(wrapper) = &field.ty else {
        return;
    };
    let is_enum = enum_names.contains(wrapper.as_str());
    debug_assert!(is_enum, "emit_enum_string_getter called with non-enum Named type");

    let name = &ctx.name;
    let getter_name = &ctx.getter_name;

    if field.optional {
        // Option<EnumType> → Option<String>
        let map_expr = if field.is_boxed {
            format!("self.0.{name}.clone().map(|w| {wrapper}::from(*w).to_string())")
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            format!("self.0.{name}.clone().map(|w| {wrapper}::from((*w).clone()).to_string())")
        } else {
            format!("self.0.{name}.clone().map(|w| {wrapper}::from(w).to_string())")
        };
        out.push_str(&format!(
            "    pub fn {getter_name}(&self) -> Option<String> {{\n        {map_expr}\n    }}\n"
        ));
    } else {
        // EnumType → String
        let expr = if field.is_boxed {
            format!("{wrapper}::from(*self.0.{name}.clone()).to_string()")
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            format!("{wrapper}::from((*self.0.{name}).clone()).to_string()")
        } else {
            format!("{wrapper}::from(self.0.{name}.clone()).to_string()")
        };
        out.push_str(&format!(
            "    pub fn {getter_name}(&self) -> String {{\n        {expr}\n    }}\n"
        ));
    }
}

/// Emit a `Vec<String>`-returning getter for a `Vec<Named(enum)>` field.
///
/// Maps each enum element through the bridge wrapper's `to_string()`.
fn emit_vec_enum_string_getter(
    field: &alef_core::ir::FieldDef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
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

    let name = &ctx.name;
    let getter_name = &ctx.getter_name;

    // Build the per-element mapping expression based on wrapping strategy.
    let elem_expr = match field.vec_inner_core_wrapper {
        CoreWrapper::Arc => format!("{wrapper}::from((**elem).clone()).to_string()"),
        _ => format!("{wrapper}::from(elem.clone()).to_string()"),
    };

    if field.optional {
        out.push_str(&format!(
            "    pub fn {getter_name}(&self) -> String {{\n        \
             serde_json::to_string(&self.0.{name}.as_ref().map(|v| \
             v.iter().map(|elem| {elem_expr}).collect::<Vec<_>>())).expect(\"serializable enum vec\")\n    }}\n"
        ));
    } else {
        out.push_str(&format!(
            "    pub fn {getter_name}(&self) -> Vec<String> {{\n        \
             self.0.{name}.iter().map(|elem| {elem_expr}).collect()\n    }}\n"
        ));
    }
}

fn emit_named_getter(
    field: &alef_core::ir::FieldDef,
    wrapper: &str,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let name = &ctx.name;
    let getter_name = &ctx.getter_name;
    let is_enum = enum_names.contains(wrapper);
    if field.optional {
        // Optional Named: self.0.field.clone().map(T) or T::from
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
        out.push_str(&crate::template_env::render(
            "getter_optional_named.jinja",
            minijinja::context! {
                getter_name => getter_name,
                wrapper => wrapper,
                getter_expr => &getter_expr,
            },
        ));
    } else {
        let expr = if field.is_boxed {
            // Deref the Box<SourceT> before wrapping.
            if is_enum {
                format!("{wrapper}::from(*self.0.{name}.clone())")
            } else {
                format!("{wrapper}(*self.0.{name}.clone())")
            }
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            // Deref the Arc<SourceT> before wrapping.
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
        out.push_str(&crate::template_env::render(
            "getter_named.jinja",
            minijinja::context! {
                getter_name => getter_name,
                wrapper => wrapper,
                expr => &expr,
            },
        ));
    }
}

fn emit_vec_getter(
    ty: &TypeDef,
    field: &alef_core::ir::FieldDef,
    inner: &TypeRef,
    ctx: &GetterCtx,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
    let _name = &ctx.name;
    let _getter_name = &ctx.getter_name;
    let _bridge_ty_owned = &ctx.bridge_ty_owned;
    if let TypeRef::Named(wrapper) = inner {
        let is_enum = enum_names.contains(wrapper.as_str());
        // When the source field is Vec<Arc<T>>, cloning an element
        // yields Arc<SourceT>; we must deref before wrapping.
        let elem_expr = match field.vec_inner_core_wrapper {
            // elem is &Arc<T>; (*elem) is Arc<T>; (**elem) is T — deref twice.
            CoreWrapper::Arc if !is_enum => format!("{wrapper}((**elem).clone())"),
            CoreWrapper::Arc => format!("{wrapper}::from((**elem).clone())"),
            _ if is_enum => format!("{wrapper}::from(elem.clone())"),
            _ => format!("{wrapper}(elem.clone())"),
        };
        if field.optional {
            out.push_str(&crate::template_env::render(
                "getter_vec_named_optional.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    wrapper => wrapper,
                    name => &ctx.name,
                    elem_expr => &elem_expr,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "getter_vec_named.jinja",
                minijinja::context! {
                    getter_name => &ctx.getter_name,
                    wrapper => wrapper,
                    name => &ctx.name,
                    elem_expr => &elem_expr,
                },
            ));
        }
    } else if !matches!(inner, TypeRef::Primitive(_) | TypeRef::Bytes) {
        // Vec<non-Primitive, non-Bytes>: use JSON round-trip for serde structs.
        if ty.has_serde {
            if field.optional {
                out.push_str(&crate::template_env::render(
                    "getter_vec_complex_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "getter_vec_complex_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            // Unreachable: `is_unbridgeable_getter` filters this case out before
            // `emit_vec_getter` is called, so non-serde Vec<Named> never lands here.
            // Emit a comment for visibility if the filter ever drifts out of sync.
            out.push_str(&crate::template_env::render(
                "getter_vec_complex_skip.jinja",
                minijinja::context! {
                    name => &ctx.name,
                },
            ));
        }
    } else {
        // Vec<Primitive> or Vec<Bytes>: use serde round-trip in serde structs.
        if ty.has_serde {
            if field.optional {
                out.push_str(&crate::template_env::render(
                    "getter_vec_primitive_serde_optional.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "getter_vec_primitive_serde.jinja",
                    minijinja::context! {
                        getter_name => &ctx.getter_name,
                        return_type => &ctx.bridge_ty_owned,
                        name => &ctx.name,
                    },
                ));
            }
        } else {
            out.push_str(&crate::template_env::render(
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

fn emit_string_like_getter(ty: &TypeDef, field: &alef_core::ir::FieldDef, ctx: &GetterCtx, out: &mut String) {
    let name = &ctx.name;
    let getter_name = &ctx.getter_name;
    let bridge_ty_owned = &ctx.bridge_ty_owned;
    // String-like fields might be JSON-bridged enums in the source struct;
    // serialize via serde_json so the result works for both `String` and
    // typed source fields.
    // Exception: when the struct itself lacks serde, the field might be a
    // non-serde type that was mapped to String by the IR. Use Debug format
    // as a safe fallback that always compiles.
    // NOTE: TypeRef::Bytes is NOT included here — it maps to Vec<u8> in the
    // bridge, not String, so it must fall through to the plain clone() branch.
    if !ty.has_serde {
        if field.optional {
            out.push_str(&crate::template_env::render(
                "getter_string_like_debug_optional.jinja",
                minijinja::context! {
                    getter_name => getter_name,
                    return_type => bridge_ty_owned,
                    name => name,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
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
        && matches!(field.core_wrapper, alef_core::ir::CoreWrapper::None)
    {
        // Plain String field (not sanitized from another type) — just clone/clone_opt.
        // serde_json::to_string on a &String adds JSON quotes ("text" → "\"text\"").
        out.push_str(&crate::template_env::render(
            "getter_simple_clone.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if matches!(field.ty, TypeRef::String)
        && matches!(field.core_wrapper, alef_core::ir::CoreWrapper::Cow)
        && !field.optional
    {
        // Cow<'static, str> field — use .to_string() to avoid JSON quoting.
        out.push_str(&crate::template_env::render(
            "getter_string_cow.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if matches!(field.ty, TypeRef::String)
        && matches!(field.core_wrapper, alef_core::ir::CoreWrapper::Cow)
        && field.optional
    {
        // Option<Cow<'static, str>> field — map to String via .to_string().
        out.push_str(&crate::template_env::render(
            "getter_string_cow_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else if field.optional {
        out.push_str(&crate::template_env::render(
            "getter_string_like_serde_optional.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "getter_string_like_serde.jinja",
            minijinja::context! {
                getter_name => getter_name,
                return_type => bridge_ty_owned,
                name => name,
            },
        ));
    }
}

/// Emit a `pub fn create_<type_name>(api_key: String, base_url: Option<String>) -> Result<TypeName, String>`
/// constructor shim for an opaque type that exposes methods.
///
/// The source crate must provide `<TypeName>::new(api_key, base_url)` or a compatible constructor.
/// This mirrors the `liter_llm::DefaultClient::new` pattern.
///
/// When the source crate's constructor signature differs
/// `DefaultClient::new(ClientConfig, Option<&str>)`), the caller can supply a
/// custom body via `[crates.<crate>.swift] client_constructor_body."TypeName" = "..."`
/// in alef.toml. The custom body is interpolated verbatim, with `{type_name}` and
/// `{source_path}` placeholders available.
pub(crate) fn emit_type_constructor_shim(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    custom_body: Option<&str>,
) -> String {
    let type_snake = ty.name.to_snake_case();
    let fn_name = format!("create_{type_snake}");
    let type_name = &ty.name;
    let source_path = resolve_type_path(type_name, source_crate, type_paths);

    if let Some(body) = custom_body {
        let interpolated = body
            .replace("{type_name}", type_name)
            .replace("{source_path}", &source_path);
        return format!(
            "pub fn {fn_name}(api_key: String, base_url: Option<String>) -> Result<{type_name}, String> {{\n{interpolated}\n}}\n"
        );
    }

    format!(
        "pub fn {fn_name}(api_key: String, base_url: Option<String>) -> Result<{type_name}, String> {{\n    \
         {source_path}::new(api_key, base_url)\n        \
         .map_err(|e| e.to_string())\n        \
         .map({type_name})\n}}\n"
    )
}

/// Emit free function shims for each method on `ty`.
///
/// Each method `fn method_name(&self, param: T) -> Result<R, E>` becomes
/// `pub fn type_name_method_name(client: &TypeName, param: BridgeT) -> Result<BridgeR, String>`.
/// Async methods are blocked on a Tokio current-thread runtime (same pattern as function shims).
pub(crate) fn emit_type_method_shims(
    ty: &TypeDef,
    _source_crate: &str,
    _type_paths: &HashMap<String, String>,
) -> String {
    let type_snake = ty.name.to_snake_case();
    let type_name = &ty.name;

    let mut out = String::new();

    // Bring trait providers into scope so trait methods on `client.0` resolve.
    // Methods from inherent impls have `trait_source: None`; methods from trait
    // impls record the fully qualified trait path (e.g. `liter_llm::client::LlmClient`).
    // Without these `use` statements rustc emits `no method named X found` for every
    // trait-provided method.
    let mut trait_uses: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for method in &ty.methods {
        if method.sanitized {
            continue;
        }
        if let Some(path) = method.trait_source.as_deref() {
            trait_uses.insert(path.to_string());
        }
    }
    for path in &trait_uses {
        out.push_str(&format!("#[allow(unused_imports)]\nuse {path};\n"));
    }
    if !trait_uses.is_empty() {
        out.push('\n');
    }

    for method in &ty.methods {
        if method.sanitized {
            continue;
        }
        // Skip static/associated functions: the shim is `pub fn type_method(client: &T)`
        // and the body uses `client.0.method()`. Static methods like `T::default()`
        // need to be called as associated functions (`T::default()`), not via the
        // receiver — calling `client.0.default()` trips E0599. We skip them rather
        // than emitting a separate constructor surface; static constructors are
        // exposed via `create_<T>` shims when an explicit client_constructor_body is
        // configured, not via method shims.
        if method.is_static {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let fn_name = format!("{type_snake}_{method_snake}");

        // Build param list: first param is `client: &TypeName`, then method params.
        let mut params_vec: Vec<String> = vec![format!("client: &{type_name}")];
        for p in &method.params {
            let bridge_ty = bridge_type(&p.ty);
            let bridge_ty = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            params_vec.push(format!("{name}: {bridge_ty}"));
        }
        let params_str = params_vec.join(", ");

        let return_ty = if method.error_type.is_some() {
            let ok_ty = bridge_type(&method.return_type);
            if matches!(method.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type(&method.return_type)
        };

        // Build call args for each method param (excluding the receiver).
        //
        // - Named newtype  → `arg.0` (unwrap to inner source-crate type)
        // - Optional<Named> → `arg.map(|v| v.0)` (preserve None, unwrap Some)
        // - String           → `&arg` (the underlying trait method usually takes `&str`)
        // - JSON-bridged     → deserialize from the bridge String
        // - Other primitives → pass through verbatim
        let call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                if needs_json_bridge(&p.ty) {
                    let native_ty = swift_bridge_rust_type(&p.ty);
                    return format!("serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
                }
                if p.optional {
                    if let TypeRef::Named(_) = &p.ty {
                        return format!("{name}.map(|v| v.0)");
                    }
                }
                match &p.ty {
                    TypeRef::Named(_) => format!("{name}.0"),
                    TypeRef::String | TypeRef::Path => format!("&{name}"),
                    _ => name,
                }
            })
            .collect();
        let call_args_str = call_args.join(", ");

        // Resolve the method call on the inner type.
        let inner_access = "client.0";
        let method_call = format!("{inner_access}.{method_snake}({call_args_str})");

        // Determine return wrapping: Named return types get wrapped in their newtype.
        let json_wrap_ok = needs_json_bridge(&method.return_type);
        let wrap_return = |source: String| -> String {
            if json_wrap_ok {
                return format!("serde_json::to_string(&({source})).expect(\"serializable return\")");
            }
            match &method.return_type {
                TypeRef::Named(t) => format!("{t}({source})"),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(t) = inner.as_ref() {
                        format!("({source}).map({t})")
                    } else {
                        source
                    }
                }
                TypeRef::String => format!("{source}.to_string()"),
                TypeRef::Path => format!("{source}.to_string_lossy().into_owned()"),
                // Trait methods that return `&[&str]` (Vec<String> + returns_ref)
                // can't be bridged to swift's `Vec<String>` without copying each
                // element to owned `String`.
                TypeRef::Vec(inner) if method.returns_ref && matches!(inner.as_ref(), TypeRef::String) => {
                    format!("{source}.iter().map(|s| s.to_string()).collect()")
                }
                _ => source,
            }
        };

        let body = if method.is_async {
            let chain = if method.error_type.is_some() {
                let ok_wrap = if json_wrap_ok {
                    ".map(|v| serde_json::to_string(&v).expect(\"serializable return\"))".to_string()
                } else {
                    match &method.return_type {
                        TypeRef::Named(t) => format!(".map({t})"),
                        TypeRef::String | TypeRef::Path => ".map(|s| s.to_string())".to_string(),
                        // `bytes::Bytes` is bridged as `Vec<u8>` in the swift-bridge surface.
                        // The trait method returns `Bytes`; convert via `.to_vec()`.
                        TypeRef::Bytes => ".map(|b| b.to_vec())".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{method_call}.await.map_err(|e| e.to_string()){ok_wrap}")
            } else {
                wrap_return(format!("{method_call}.await"))
            };
            format!(
                "    ::tokio::runtime::Builder::new_current_thread()\n        \
                 .enable_all()\n        \
                 .build()\n        \
                 .expect(\"build tokio runtime\")\n        \
                 .block_on(async {{ {chain} }})"
            )
        } else if method.error_type.is_some() {
            let ok_wrap = if json_wrap_ok {
                ".map(|v| serde_json::to_string(&v).expect(\"serializable return\"))".to_string()
            } else {
                match &method.return_type {
                    TypeRef::Named(t) => format!(".map({t})"),
                    TypeRef::String | TypeRef::Path => ".map(|s| s.to_string())".to_string(),
                    TypeRef::Bytes => ".map(|b| b.to_vec())".to_string(),
                    _ => String::new(),
                }
            };
            format!("    {method_call}.map_err(|e| e.to_string()){ok_wrap}")
        } else {
            format!("    {}", wrap_return(method_call))
        };

        out.push_str(&format!(
            "pub fn {fn_name}({params_str}) -> {return_ty} {{\n{body}\n}}\n"
        ));
    }
    out
}

/// Emit Rust free-function shims and opaque `StreamHandle` types for streaming
/// adapters that have an `owner_type`.
///
/// For each streaming adapter, emits three free functions + one handle struct:
///
/// - `pub struct {Owner}{Adapter}StreamHandle` — owns a tokio runtime + boxed
///   stream, exposes `next_json(&mut self) -> Result<String, String>` to advance.
/// - `pub fn {owner_snake}_{name}_start(client: &OwnerType, ...params...) -> Result<*mut Handle, String>`
///   — kicks the request (HTTP errors propagate before any chunks arrive).
/// - `pub fn {owner_snake}_{name}_next(handle: &mut Handle) -> Result<String, String>`
///   — blocks on the next chunk; returns the JSON-encoded chunk, or an empty
///   string `""` to signal clean end-of-stream. Errors propagate as `Err(String)`.
/// - `pub fn {owner_snake}_{name}_free(handle: *mut Handle)` — drops the handle.
///
/// ### Why JSON-string at the bridge boundary
///
/// swift-bridge 0.1.x's support for `Result<Option<OpaqueRustType>, String>` is
/// not exercised in the upstream codegen tests, and `Option<RustString>` works
/// reliably across versions. We pick the most stable encoding — a JSON string —
/// matching the FFI/Java backends' item-to-JSON protocol and reusing the
/// item type's existing `Serialize` impl (every adapter `item_type` is a
/// serde-bridged DTO in current consumers).
///
/// An empty string `""` is never a valid JSON value, so it is a safe EOF sentinel.
///
/// ### Runtime ownership (SAFETY)
///
/// Each handle owns its own `tokio::runtime::Runtime`. This is heavier than a
/// shared OnceLock runtime but lets `next_json()` block irrespective of whether
/// the calling Swift thread is on an executor. Streams are typically long-lived
/// (one runtime per chat session), so per-stream runtime overhead is acceptable.
pub(crate) fn emit_streaming_adapter_shims(
    adapters: &[alef_core::config::AdapterConfig],
    source_crate: &str,
) -> String {
    use alef_core::config::AdapterPattern;
    use heck::{ToPascalCase, ToSnakeCase};

    let mut out = String::new();

    for adapter in adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| a.owner_type.is_some())
    {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let item_type = adapter
            .item_type
            .as_deref()
            .expect("streaming adapter must declare item_type for Swift backend");
        let owner_snake = owner_type.to_snake_case();
        let adapter_pascal = adapter.name.to_pascal_case();
        let owner_pascal = owner_type.to_pascal_case();
        let handle_name = format!("{owner_pascal}{adapter_pascal}StreamHandle");
        let fn_start = format!("{owner_snake}_{}_start", adapter.name);

        // The fully-qualified item type lives in the umbrella source crate.
        // Consumers' item types are serde-bridged DTOs that already derive
        // `Serialize`; we use `serde_json::to_string` on each chunk so the
        // bridge boundary only sees `Result<String, String>`.
        let core_item = format!("{source_crate}::{item_type}");

        // Build start-function param list: first param is the opaque client receiver,
        // then adapter params (passed by reference because their swift-bridge wrapper
        // newtypes are non-Copy).
        let mut start_params_vec: Vec<String> = vec![format!("client: &{owner_type}")];
        for p in &adapter.params {
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            let param_name = swift_ident(&p.name.to_snake_case());
            start_params_vec.push(format!("{param_name}: &{simple_ty}"));
        }
        let start_params_str = start_params_vec.join(", ");

        // Build call args (excluding the receiver). Named types bridged as
        // newtypes must be unwrapped to `.0` and cloned because the core API
        // takes ownership of the request.
        let call_args: Vec<String> = adapter
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                format!("{name}.0.clone()")
            })
            .collect();
        let call_args_str = call_args.join(", ");

        // Resolve the core Rust call. A bare `core_path` (no `::`) names a method
        // on the owner type — invoke it as `client.0.<method>(args)`. A fully
        // qualified `core_path` is invoked as a free function.
        let core_call = if adapter.core_path.contains("::") {
            format!("{}(&client.0, {call_args_str})", adapter.core_path)
        } else {
            format!("client.0.{}({call_args_str})", adapter.core_path)
        };

        // Emit the handle struct. It owns a Runtime and a Mutex<Option<BoxStream>>
        // so `next()` can drive polling and so we can drop the stream on Drop.
        //
        // Error type erased to `Box<dyn Error + Send + Sync>` so the struct type
        // is stable across core error-type changes.
        //
        // SAFETY: the handle is single-owner — swift-bridge generates a Swift
        // `class` shadow with `deinit { *_free(ptr) }` that runs Drop on this
        // struct exactly once when the Swift handle goes out of scope. The Mutex
        // guards `next()` so calls serialise even when Swift accidentally fans
        // out across tasks.
        out.push_str(&format!(
            "/// Opaque handle owning a tokio runtime and a boxed `{item_type}` stream.\n\
             ///\n\
             /// Created by `{fn_start}`, advanced via `next()`. Drop runs when the\n\
             /// Swift handle goes out of scope (swift-bridge generates the matching\n\
             /// `deinit`), so explicit cleanup from Swift is unnecessary.\n\
             ///\n\
             /// Items are JSON-encoded at the bridge boundary because swift-bridge's\n\
             /// `Option<OpaqueRust>` support varies across versions, while `Result<String,\n\
             /// String>` is well-tested. An empty string `\"\"` is the EOF sentinel —\n\
             /// no valid JSON value is the empty string.\n\
             pub struct {handle_name} {{\n\
             \x20   rt: ::tokio::runtime::Runtime,\n\
             \x20   stream: ::std::sync::Mutex<\n\
             \x20       Option<\n\
             \x20           ::futures_util::stream::BoxStream<\n\
             \x20               'static,\n\
             \x20               Result<{core_item}, Box<dyn ::std::error::Error + Send + Sync + 'static>>,\n\
             \x20           >,\n\
             \x20       >,\n\
             \x20   >,\n\
             }}\n\n"
        ));

        // _start: open the stream. HTTP-level errors (e.g. 401) surface as
        // Err(String) before any chunks arrive, so Swift `try` catches them.
        out.push_str(&format!(
            "/// Start a streaming `{owner_type}::{adapter_name}` request.\n\
             ///\n\
             /// Returns a fresh `{handle_name}` whose ownership transfers to the\n\
             /// Swift caller (swift-bridge boxes the handle internally).\n\
             pub fn {fn_start}({start_params_str}) -> Result<{handle_name}, String> {{\n\
             \x20   use ::futures_util::StreamExt;\n\
             \x20   let rt = ::tokio::runtime::Builder::new_multi_thread()\n\
             \x20       .worker_threads(1)\n\
             \x20       .enable_all()\n\
             \x20       .build()\n\
             \x20       .map_err(|e| format!(\"build tokio runtime: {{e}}\"))?;\n\
             \x20   let raw = rt.block_on(async {{\n\
             \x20       {core_call}\n\
             \x20           .await\n\
             \x20           .map_err(|e| e.to_string())\n\
             \x20   }})?;\n\
             \x20   let erased: ::futures_util::stream::BoxStream<\n\
             \x20       'static,\n\
             \x20       Result<{core_item}, Box<dyn ::std::error::Error + Send + Sync + 'static>>,\n\
             \x20   > = Box::pin(\n\
             \x20       raw.map(|r| r.map_err(|e| Box::new(e) as Box<dyn ::std::error::Error + Send + Sync + 'static>)),\n\
             \x20   );\n\
             \x20   Ok({handle_name} {{\n\
             \x20       rt,\n\
             \x20       stream: ::std::sync::Mutex::new(Some(erased)),\n\
             \x20   }})\n\
             }}\n\n",
            adapter_name = adapter.name,
        ));

        // The `next` method on the handle drives the stream forward.
        out.push_str(&format!(
            "impl {handle_name} {{\n\
             \x20   /// Advance the stream and return the next chunk JSON, or `\"\"` on clean\n\
             \x20   /// end-of-stream. Returns `Err(message)` on a stream-level error.\n\
             \x20   pub fn next(&mut self) -> Result<String, String> {{\n\
             \x20       let mut guard = self\n\
             \x20           .stream\n\
             \x20           .lock()\n\
             \x20           .map_err(|_| \"{handle_name}::next: stream mutex poisoned\".to_string())?;\n\
             \x20       let stream = match guard.as_mut() {{\n\
             \x20           Some(s) => s,\n\
             \x20           None => return Ok(String::new()),\n\
             \x20       }};\n\
             \x20       use ::futures_util::StreamExt;\n\
             \x20       match self.rt.block_on(stream.next()) {{\n\
             \x20           Some(Ok(item)) => ::serde_json::to_string(&item).map_err(|e| e.to_string()),\n\
             \x20           Some(Err(e)) => {{\n\
             \x20               *guard = None;\n\
             \x20               Err(e.to_string())\n\
             \x20           }}\n\
             \x20           None => {{\n\
             \x20               *guard = None;\n\
             \x20               Ok(String::new())\n\
             \x20           }}\n\
             \x20       }}\n\
             \x20   }}\n\
             }}\n\n"
        ));
    }

    out
}
