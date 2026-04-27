use alef_core::ir::{EnumDef, TypeDef};

use super::conversions::{frb_rust_type, frb_rust_type_inner};

pub(crate) fn emit_mirror_struct(out: &mut String, ty: &TypeDef) {
    // FRB v2 mirror convention: the mirror struct keeps the same name as the
    // original; the `mirror` attribute argument tells FRB which type this
    // declaration shadows for codegen purposes.
    out.push_str(&format!("#[frb(mirror({name}))]\n", name = ty.name));
    out.push_str(&format!("pub struct {} {{\n", ty.name));
    for field in &ty.fields {
        let rust_ty = frb_rust_type(&field.ty, field.optional);
        out.push_str(&format!("    pub {}: {rust_ty},\n", field.name));
    }
    out.push_str("}\n");
}

pub(crate) fn emit_mirror_enum(out: &mut String, en: &EnumDef) {
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    out.push_str(&format!("#[frb(mirror({name}))]\n", name = en.name));
    if all_unit {
        out.push_str(&format!("pub enum {} {{\n", en.name));
        for variant in &en.variants {
            out.push_str(&format!("    {},\n", variant.name));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&format!("pub enum {} {{\n", en.name));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&format!("    {},\n", variant.name));
            } else {
                out.push_str(&format!("    {} {{\n", variant.name));
                for (idx, f) in variant.fields.iter().enumerate() {
                    // Tuple-variant fields land in the IR as "_0", "_1", ... but
                    // flutter_rust_bridge strips a leading underscore from Dart
                    // field names — leaving an invalid bare digit. Rename any
                    // empty or "_N"-style field to a Dart-safe "fieldN".
                    let fname = if f.name.is_empty() || f.name.starts_with('_') {
                        format!("field{idx}")
                    } else {
                        f.name.clone()
                    };
                    let rust_ty = frb_rust_type_inner(&f.ty);
                    out.push_str(&format!("        {fname}: {rust_ty},\n"));
                }
                out.push_str("    },\n");
            }
        }
        out.push_str("}\n");
    }
}
