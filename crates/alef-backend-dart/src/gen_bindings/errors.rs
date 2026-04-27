use alef_core::ir::ErrorDef;
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use super::render_type::render_type;

pub(super) fn emit_error(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !error.doc.is_empty() {
        for line in error.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("sealed class {} implements Exception {{\n", error.name));
    out.push_str("  String get message;\n");
    out.push_str("}\n\n");
    for variant in &error.variants {
        if !variant.doc.is_empty() {
            for line in variant.doc.lines() {
                out.push_str("/// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        if variant.is_unit {
            let msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&format!("final class {} implements {} {{\n", variant.name, error.name));
            out.push_str(&format!("  @override\n  String get message => '{msg}';\n"));
            out.push_str(&format!("  const {}();\n", variant.name));
            out.push_str("}\n");
        } else {
            out.push_str(&format!("final class {} implements {} {{\n", variant.name, error.name));
            for f in &variant.fields {
                let ty_str = render_type(&f.ty, imports);
                let fname = f.name.to_lower_camel_case();
                out.push_str(&format!("  final {ty_str} {fname};\n"));
            }
            let msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str("  @override\n");
            out.push_str(&format!("  String get message => '{msg}';\n"));
            if variant.fields.len() == 1 {
                let fname = variant.fields[0].name.to_lower_camel_case();
                out.push_str(&format!("  {}(this.{fname});\n", variant.name));
            } else {
                out.push_str(&format!("  {}({{\n", variant.name));
                for f in &variant.fields {
                    let fname = f.name.to_lower_camel_case();
                    out.push_str(&format!("    required this.{fname},\n"));
                }
                out.push_str("  });\n");
            }
            out.push_str("}\n");
        }
    }
}
