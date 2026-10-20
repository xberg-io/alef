//! The conversion from a core error into its Dart mirror.

use crate::core::ir::{ErrorDef, FieldDef, PrimitiveType, TypeRef};

/// Return the conversion expression that builds a mirror field from the owned core field
/// value `expr`. The inverse of [`field_from_expr`]: integers widen to `i64`/`f64`,
/// `Duration` becomes milliseconds, and an optional field maps through its option so `None`
/// stays `None`.
fn field_to_mirror_expr(field: &FieldDef, expr: &str) -> String {
    if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
        let inner_field = FieldDef {
            optional: false,
            ..field.clone()
        };
        let inner_expr = field_to_mirror_expr(&inner_field, "v");
        return if inner_expr == "v" {
            expr.to_string()
        } else {
            format!("{expr}.map(|v| {inner_expr})")
        };
    }
    if field.sanitized {
        return format!("{expr}.to_string()");
    }
    match &field.ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::I64 | PrimitiveType::F64 | PrimitiveType::Bool => expr.to_string(),
            _ => format!("{expr} as _"),
        },
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        TypeRef::String => {
            if matches!(field.core_wrapper, crate::core::ir::CoreWrapper::Cow) {
                format!("{expr}.into_owned()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Bytes => format!("{expr}.into()"),
        TypeRef::Char => format!("{expr}.to_string()"),
        TypeRef::Path => format!("{expr}.to_string_lossy().into_owned()"),
        TypeRef::Optional(inner) => {
            let inner_field = FieldDef {
                ty: *inner.clone(),
                optional: false,
                ..field.clone()
            };
            let inner_expr = field_to_mirror_expr(&inner_field, "v");
            if inner_expr == "v" {
                expr.to_string()
            } else {
                format!("{expr}.map(|v| {inner_expr})")
            }
        }
        TypeRef::Vec(inner) => {
            let inner_field = FieldDef {
                ty: *inner.clone(),
                optional: false,
                ..field.clone()
            };
            let inner_expr = field_to_mirror_expr(&inner_field, "x");
            if inner_expr == "x" {
                expr.to_string()
            } else {
                format!("{expr}.into_iter().map(|x| {inner_expr}).collect()")
            }
        }
        TypeRef::Named(name) => {
            if field.is_boxed {
                format!("{name}::from(*{expr})")
            } else {
                format!("{name}::from({expr})")
            }
        }
        _ => format!("{expr}.into()"),
    }
}

/// Emit `impl From<CoreError> for MirrorError`, the conversion the bridge applies to a failed
/// call so Dart receives the typed error with its variant and fields.
///
/// The mirror declares the same variants and visible fields as [`emit_mirror_error`] writes,
/// so each arm below mirrors one declaration there. A variant whose fields are all excluded
/// from the binding is declared with a single `field0` string, which carries the core error
/// message. The final arm covers variants a non-exhaustive core error may gain after the
/// binding was generated. ~keep
pub(super) fn emit_from_core_impl(out: &mut String, error: &ErrorDef, core_path: &str) {
    let name = error.name.as_str();
    out.push_str("\n#[allow(unreachable_patterns)]\n");
    out.push_str(&format!("impl From<{core_path}> for {name} {{\n"));
    out.push_str(&format!("    fn from(e: {core_path}) -> Self {{\n        match e {{\n"));
    for variant in &error.variants {
        let vname = &variant.name;
        let visible: Vec<(usize, &FieldDef)> = variant
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.binding_excluded)
            .collect();
        if variant.is_unit {
            out.push_str(&format!("            {core_path}::{vname} => {name}::{vname},\n"));
        } else if variant.is_tuple && variant.fields.iter().all(|f| f.binding_excluded) {
            out.push_str(&format!(
                "            {core_path}::{vname} {{ .. }} => {name}::{vname} {{ field0: e.to_string() }},\n"
            ));
        } else if visible.is_empty() {
            out.push_str(&format!(
                "            {core_path}::{vname} {{ .. }} => {name}::{vname},\n"
            ));
        } else {
            let mut pattern: Vec<String> = Vec::new();
            let mut assign: Vec<String> = Vec::new();
            for (visible_idx, (position, f)) in visible.iter().enumerate() {
                let positional = f.name.is_empty() || f.name.starts_with('_');
                let mirror_name = if positional {
                    format!("field{visible_idx}")
                } else {
                    f.name.clone()
                };
                let core_field = if positional { position.to_string() } else { f.name.clone() };
                pattern.push(format!("{core_field}: f_{mirror_name}"));
                assign.push(format!(
                    "{mirror_name}: {}",
                    field_to_mirror_expr(f, &format!("f_{mirror_name}"))
                ));
            }
            out.push_str(&format!(
                "            {core_path}::{vname} {{ {}, .. }} => {name}::{vname} {{ {} }},\n",
                pattern.join(", "),
                assign.join(", ")
            ));
        }
    }
    out.push_str(&format!(
        "            _ => unreachable!(\"{name} gained a variant after the Dart binding was generated\"),\n"
    ));
    out.push_str("        }\n    }\n}\n");
}
