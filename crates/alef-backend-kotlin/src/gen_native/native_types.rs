//! Kotlin/Native type, enum, and error emission for data class declarations.
//!
//! These emitters produce Kotlin code for data types — they are distinct from
//! function body emission because data classes use the Kotlin/Native type set
//! (no `cinterop` types in struct fields).

use alef_core::ir::{EnumDef, ErrorDef, TypeDef};

use super::native_type_str;
use crate::gen_bindings::{kotlin_field_name, to_screaming_snake};

pub(super) fn emit_native_type(ty: &TypeDef, out: &mut String) {
    if !ty.doc.is_empty() {
        for line in ty.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if ty.fields.is_empty() {
        out.push_str(&format!("class {} {{}}\n", ty.name));
        return;
    }
    out.push_str(&format!("data class {}(\n", ty.name));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = native_type_str(&field.ty, field.optional);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&format!("    val {name}: {ty_str}{comma}\n"));
    }
    out.push_str(")\n");
}

pub(super) fn emit_native_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        for line in en.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&format!("enum class {} {{\n", en.name));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&format!("    {name}{comma}\n"));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&format!("sealed class {} {{\n", en.name));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&format!("    object {} : {}()\n", variant.name, en.name));
            } else {
                out.push_str(&format!("    data class {}(\n", variant.name));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = native_type_str(&f.ty, f.optional);
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&format!("        val {name}: {ty_str}{comma}\n"));
                }
                out.push_str(&format!("    ) : {}()\n", en.name));
            }
        }
        out.push_str("}\n");
    }
}

pub(super) fn emit_native_error(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        for line in error.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!(
        "sealed class {}(message: String) : Exception(message) {{\n",
        error.name
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&format!(
                "    object {} : {}(\"{}\")\n",
                variant.name,
                error.name,
                variant.message_template.as_deref().unwrap_or(&variant.name)
            ));
        } else {
            out.push_str(&format!("    data class {}(\n", variant.name));
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = native_type_str(&f.ty, f.optional);
                let name = kotlin_field_name(&f.name, idx);
                let modifier = if name == "message" { "override " } else { "" };
                let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                out.push_str(&format!("        {modifier}val {name}: {ty_str}{comma}\n"));
            }
            let message_template = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&format!("    ) : {}(\"{message_template}\")\n", error.name));
        }
    }
    out.push_str("}\n");
}
