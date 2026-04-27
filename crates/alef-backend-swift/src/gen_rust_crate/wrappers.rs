//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub kreuzberg::T)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::gen_rust_crate::default_construction::{emit_default_construction_body, emit_direct_field_inits};
use crate::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use alef_codegen::generators::type_paths::resolve_type_path;
use alef_core::ir::{CoreWrapper, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

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
    out.push_str(&format!("pub struct {}(pub {});\n\n", ty.name, source_path));

    if !ty.fields.is_empty() {
        out.push_str(&format!("impl {} {{\n", ty.name));

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
                let name = f.name.to_snake_case();
                format!("{name}: {bridge_ty}")
            })
            .collect();

        // Determine construction strategy (see default_construction.rs for details):
        // when any field requires Default-based assignment, we cannot emit a direct struct literal.
        let has_vec_non_primitive = ty.fields.iter().any(|f| {
            matches!(&f.ty, TypeRef::Vec(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes))
        });
        let has_non_serde_string_field = !ty.has_serde
            && ty.fields.iter().any(|f| {
                matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Char)
            });
        let needs_default_construction = ty.has_serde
            || has_vec_non_primitive
            || has_non_serde_string_field
            || ty.fields.iter().any(|f| {
                needs_json_bridge(&f.ty) || matches!(f.ty, TypeRef::Named(_))
            });

        out.push_str(&format!(
            "    pub fn new({}) -> {} {{\n",
            params.join(", "),
            ty.name
        ));

        if needs_default_construction && !ty.has_default {
            // The struct needs mutable-default construction but doesn't impl Default.
            out.push_str("        ::std::unimplemented!(\"constructor not available: struct requires Default which is not implemented\")\n");
        } else if needs_default_construction && ty.has_default {
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
            let field_inits = emit_direct_field_inits(
                ty,
                type_paths,
                enum_names,
                no_serde_names,
                exclude_fields,
            );
            out.push_str(&format!(
                "        {}({} {{\n",
                ty.name, source_path
            ));
            for init in &field_inits {
                out.push_str(init);
                out.push_str(",\n");
            }
            out.push_str("        })\n");
        }
        out.push_str("    }\n");

        // Getters — return bridge types (String for JSON-bridged, wrappers for Named).
        emit_getters(ty, type_paths, enum_names, no_serde_names, exclude_fields, &mut out);

        out.push_str("}\n");
    }

    out
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
        let bridge_ty = bridge_type(&field.ty);
        let bridge_ty_owned = if field.optional && !needs_json_bridge(&field.ty) {
            format!("Option<{bridge_ty}>")
        } else {
            bridge_ty
        };
        let name = field.name.to_snake_case();
        // Explicitly excluded fields: emit an unimplemented getter that compiles
        // but panics at runtime if called.
        let field_key = format!("{}.{}", ty.name, name);
        if exclude_fields.contains(&field_key) {
            out.push_str(&format!(
                "    // alef: excluded field `{name}` — actual type is not serializable\n"
            ));
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::std::unimplemented!(\"{name}: excluded field\") }}\n"
            ));
            continue;
        }
        // Emit unimplemented getter when inner Named type lacks serde.
        let excluded_inner_named: Option<&str> = if needs_json_bridge(&field.ty) {
            match &field.ty {
                TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n)
                        if !type_paths.contains_key(n.as_str())
                            || no_serde_names.contains(n.as_str()) =>
                    {
                        Some(n.as_str())
                    }
                    _ => None,
                },
                TypeRef::Named(n)
                    if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()) =>
                {
                    Some(n.as_str())
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(excluded) = excluded_inner_named {
            out.push_str(&format!(
                "    // alef: skipped — field `{name}` contains `{excluded}` which is excluded from codegen\n"
            ));
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::std::unimplemented!(\"{name}: {excluded} is not bridgeable\") }}\n"
            ));
        } else if needs_json_bridge(&field.ty) {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ serde_json::to_string(&self.0.{name}).expect(\"serializable {name}\") }}\n"
            ));
        } else if let TypeRef::Named(wrapper) = &field.ty {
            emit_named_getter(field, wrapper, &name, &bridge_ty_owned, enum_names, out);
        } else if let TypeRef::Vec(inner) = &field.ty {
            emit_vec_getter(ty, field, inner, &name, &bridge_ty_owned, enum_names, out);
        } else if matches!(field.ty, TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json) {
            emit_string_like_getter(ty, field, &name, &bridge_ty_owned, out);
        } else if matches!(field.ty, TypeRef::Bytes) {
            // bytes::Bytes bridges as Vec<u8>; convert with .to_vec() for the return.
            if field.optional {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().map(|b| b.to_vec()) }}\n"
                ));
            } else {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.to_vec() }}\n"
                ));
            }
        } else if ty.has_serde && matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Primitive(_)) {
            // Vec<T> or Primitive fields in serde structs: use serde JSON round-trip.
            if field.optional {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().and_then(|v| ::serde_json::to_value(v).ok().and_then(|j| ::serde_json::from_value(j).ok())) }}\n"
                ));
            } else {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::serde_json::to_value(&self.0.{name}).ok().and_then(|j| ::serde_json::from_value(j).ok()).unwrap_or_default() }}\n"
                ));
            }
        } else {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.clone() }}\n"
            ));
        }
    }
}

fn emit_named_getter(
    field: &alef_core::ir::FieldDef,
    wrapper: &str,
    name: &str,
    _bridge_ty_owned: &str,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
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
        out.push_str(&format!(
            "    pub fn {name}(&self) -> Option<{wrapper}> {{ {getter_expr} }}\n"
        ));
    } else {
        let expr = if field.is_boxed {
            // Deref the Box<kreuzberg::T> before wrapping.
            if is_enum {
                format!("{wrapper}::from(*self.0.{name}.clone())")
            } else {
                format!("{wrapper}(*self.0.{name}.clone())")
            }
        } else if matches!(field.core_wrapper, CoreWrapper::Arc) {
            // Deref the Arc<kreuzberg::T> before wrapping.
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
        out.push_str(&format!(
            "    pub fn {name}(&self) -> {wrapper} {{ {expr} }}\n"
        ));
    }
}

fn emit_vec_getter(
    ty: &TypeDef,
    field: &alef_core::ir::FieldDef,
    inner: &TypeRef,
    name: &str,
    bridge_ty_owned: &str,
    enum_names: &HashSet<&str>,
    out: &mut String,
) {
    if let TypeRef::Named(wrapper) = inner {
        let is_enum = enum_names.contains(wrapper.as_str());
        // When the source field is Vec<Arc<T>>, cloning an element
        // yields Arc<kreuzberg::T>; we must deref before wrapping.
        let elem_expr = match field.vec_inner_core_wrapper {
            // elem is &Arc<T>; (*elem) is Arc<T>; (**elem) is T — deref twice.
            CoreWrapper::Arc if !is_enum => format!("{wrapper}((**elem).clone())"),
            CoreWrapper::Arc => format!("{wrapper}::from((**elem).clone())"),
            _ if is_enum => format!("{wrapper}::from(elem.clone())"),
            _ => format!("{wrapper}(elem.clone())"),
        };
        if field.optional {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> Option<Vec<{wrapper}>> {{ self.0.{name}.as_ref().map(|v| v.iter().map(|elem| {elem_expr}).collect()) }}\n"
            ));
        } else {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> Vec<{wrapper}> {{ self.0.{name}.iter().map(|elem| {elem_expr}).collect() }}\n"
            ));
        }
    } else if !matches!(inner, TypeRef::Primitive(_) | TypeRef::Bytes) {
        // Vec<non-Primitive, non-Bytes>: use JSON round-trip for serde structs.
        if ty.has_serde {
            if field.optional {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().and_then(|v| ::serde_json::to_value(v).ok().and_then(|j| ::serde_json::from_value(j).ok())) }}\n"
                ));
            } else {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::serde_json::to_value(&self.0.{name}).ok().and_then(|j| ::serde_json::from_value(j).ok()).unwrap_or_default() }}\n"
                ));
            }
        } else {
            out.push_str(&format!(
                "    // alef: skipped — Vec field `{name}` may have different actual type in non-serde struct\n"
            ));
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::std::unimplemented!(\"{name}: Vec field type mismatch in non-serde struct\") }}\n"
            ));
        }
    } else {
        // Vec<Primitive> or Vec<Bytes>: use serde round-trip in serde structs.
        if ty.has_serde {
            if field.optional {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().and_then(|v| ::serde_json::to_value(v).ok().and_then(|j| ::serde_json::from_value(j).ok())) }}\n"
                ));
            } else {
                out.push_str(&format!(
                    "    pub fn {name}(&self) -> {bridge_ty_owned} {{ ::serde_json::to_value(&self.0.{name}).ok().and_then(|j| ::serde_json::from_value(j).ok()).unwrap_or_default() }}\n"
                ));
            }
        } else {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.clone() }}\n"
            ));
        }
    }
}

fn emit_string_like_getter(
    ty: &TypeDef,
    field: &alef_core::ir::FieldDef,
    name: &str,
    bridge_ty_owned: &str,
    out: &mut String,
) {
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
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().map(|v| format!(\"{{v:?}}\")) }}\n"
            ));
        } else {
            out.push_str(&format!(
                "    pub fn {name}(&self) -> {bridge_ty_owned} {{ format!(\"{{:?}}\", &self.0.{name}) }}\n"
            ));
        }
    } else if field.optional {
        out.push_str(&format!(
            "    pub fn {name}(&self) -> {bridge_ty_owned} {{ self.0.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok()) }}\n"
        ));
    } else {
        out.push_str(&format!(
            "    pub fn {name}(&self) -> {bridge_ty_owned} {{ serde_json::to_string(&self.0.{name}).unwrap_or_default() }}\n"
        ));
    }
}

