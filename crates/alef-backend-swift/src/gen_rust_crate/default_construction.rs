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
    out.push_str(&format!(
        "        let mut __target: {} = ::std::default::Default::default();\n",
        source_path
    ));
    for f in &ty.fields {
        let name = f.name.to_snake_case();
        // Explicitly excluded fields: leave at Default::default() silently.
        let field_key = format!("{}.{}", ty.name, name);
        if exclude_fields.contains(&field_key) {
            out.push_str(&format!(
                "        // alef: excluded field `{name}` — actual type is not serializable, left at default\n"
            ));
            continue;
        }
        // Check if the inner Named type (if any) is excluded or lacks serde.
        let excluded_inner: Option<&str> = if needs_json_bridge(&f.ty) {
            match &f.ty {
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
        if excluded_inner.is_some() {
            // The inner type is excluded (e.g. InternalDocument — no serde derive).
            // Leave the field at its Default value; the serde bridge can't work.
            out.push_str(&format!(
                "        // alef: skipped — field `{name}` contains excluded type, left at default\n"
            ));
        } else if needs_json_bridge(&f.ty) {
            // JSON-decode into a serde_json::Value, then assign as JSON-deserialized
            // typed value via reinterpret.
            out.push_str(&format!(
                "        if let Ok(v) = ::serde_json::from_str::<::serde_json::Value>(&{name}) {{\n"
            ));
            out.push_str(&format!(
                "            if let Ok(t) = ::serde_json::from_value(v) {{ __target.{name} = t; }}\n"
            ));
            out.push_str("        }\n");
        } else if let TypeRef::Named(n) = &f.ty {
            // Enum wrappers only have From<kreuzberg::T> for BridgeT (not the reverse),
            // so we cannot convert a bridge enum back to the source type via .into().
            // For struct newtypes, use .0; for enums, leave at Default.
            // The constructor param is still accepted (so the API is stable) but
            // the value is dropped for enum fields. This is a known limitation.
            let is_enum = enum_names.contains(n.as_str());
            if is_enum {
                // alef: enum fields in constructors are not converted back — leave at default
                out.push_str(&format!(
                    "        // alef: {name} ({n}) is an enum; reverse From not generated — left at default\n"
                ));
            } else if f.optional {
                // Optional Named field; wrap in Some(w.0), Box::new, or Arc::new if needed.
                if f.is_boxed {
                    out.push_str(&format!(
                        "        if let Some(w) = {name} {{ __target.{name} = Some(Box::new(w.0)); }}\n"
                    ));
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    out.push_str(&format!(
                        "        if let Some(w) = {name} {{ __target.{name} = Some(std::sync::Arc::new(w.0)); }}\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "        if let Some(w) = {name} {{ __target.{name} = Some(w.0); }}\n"
                    ));
                }
            } else if f.is_boxed {
                // The source field is Box<T>; wrap in Box::new().
                out.push_str(&format!("        __target.{name} = Box::new({name}.0);\n"));
            } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                // The source field is Arc<T>; wrap in Arc::new().
                out.push_str(&format!("        __target.{name} = std::sync::Arc::new({name}.0);\n"));
            } else {
                out.push_str(&format!("        __target.{name} = {name}.0;\n"));
            }
        } else if let TypeRef::Vec(inner) = &f.ty {
            // Vec<Named> fields: unwrap bridge wrappers element-wise.
            // Enum elements: same limitation as above — leave at default.
            if let TypeRef::Named(inner_n) = inner.as_ref() {
                let is_enum = enum_names.contains(inner_n.as_str());
                if is_enum {
                    out.push_str(&format!(
                        "        // alef: {name} (Vec<{inner_n}>) contains enum elements — left at default\n"
                    ));
                } else {
                    // When the source field is Vec<Arc<T>>, wrap each element in Arc::new().
                    let unwrap_expr = match f.vec_inner_core_wrapper {
                        CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                        _ => "w.0".to_string(),
                    };
                    if f.optional {
                        out.push_str(&format!(
                            "        if let Some(v) = {name} {{ __target.{name} = Some(v.into_iter().map(|w| {unwrap_expr}).collect()); }}\n"
                        ));
                    } else {
                        out.push_str(&format!(
                            "        __target.{name} = {name}.into_iter().map(|w| {unwrap_expr}).collect();\n"
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
                out.push_str(&format!(
                    "        if let Ok(__v) = ::serde_json::to_value({name}) {{\n"
                ));
                out.push_str(&format!(
                    "            if let Ok(t) = ::serde_json::from_value(__v) {{ __target.{name} = t; }}\n"
                ));
                out.push_str("        }\n");
            } else if matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
                // Vec<Primitive> or Vec<Bytes> in non-serde struct: types should match.
                out.push_str(&format!("        __target.{name} = {name};\n"));
            } else {
                // Vec<non-Primitive> in non-serde struct: actual type may differ from IR.
                // Leave at Default to avoid type mismatches.
                out.push_str(&format!(
                    "        // alef: {name} — Vec field type may differ from IR in non-serde struct, left at default\n"
                ));
            }
        } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Char | TypeRef::Json) {
            // String-like fields may map to enum/Named types in the source struct
            // (alef's IR uses String as a fallback when the actual type can't be
            // resolved). When the struct lacks serde derives, the field type is
            // likely a non-serde type — leave at default to avoid compile errors.
            // Bytes (Vec<u8>) is excluded: bridges as Vec<u8> directly, not String.
            if !ty.has_serde {
                out.push_str(&format!(
                    "        // alef: {name} — String fallback in non-serde struct, left at default\n"
                ));
            } else if f.optional {
                out.push_str(&format!(
                    "        if let Some(s) = {name} {{\n"
                ));
                out.push_str(&format!(
                    "            if let Ok(v) = ::serde_json::from_str::<::serde_json::Value>(&s) {{\n"
                ));
                out.push_str(&format!(
                    "                if let Ok(t) = ::serde_json::from_value(v) {{ __target.{name} = Some(t); }}\n"
                ));
                out.push_str("            }\n        }\n");
            } else {
                out.push_str(&format!(
                    "        if let Ok(v) = ::serde_json::from_str::<::serde_json::Value>(&{name}) {{\n"
                ));
                out.push_str(&format!(
                    "            if let Ok(t) = ::serde_json::from_value(v) {{ __target.{name} = t; }}\n"
                ));
                out.push_str("        }\n");
            }
        } else if matches!(f.ty, TypeRef::Bytes) {
            // bytes::Bytes != Vec<u8>; convert with .into() so the assignment compiles.
            out.push_str(&format!("        __target.{name} = {name}.into();\n"));
        } else {
            out.push_str(&format!("        __target.{name} = {name};\n"));
        }
    }
    out.push_str(&format!("        {}(__target)\n", ty.name));
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
            } else {
                format!("            {name}")
            }
        })
        .collect()
}
