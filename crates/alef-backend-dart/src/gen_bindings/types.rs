use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::type_map::DartMapper;

use super::render_type::render_type;

pub(super) fn emit_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
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
    out.push_str(&format!("class {} {{\n", ty.name));
    for field in &ty.fields {
        let ty_str = if field.optional {
            format!("{}?", render_type(&field.ty, imports))
        } else {
            render_type(&field.ty, imports)
        };
        let name = field.name.to_lower_camel_case();
        if !field.doc.is_empty() {
            for line in field.doc.lines() {
                out.push_str("  /// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&format!("  final {ty_str} {name};\n"));
    }
    // Constructor
    if ty.fields.len() == 1 {
        let name = ty.fields[0].name.to_lower_camel_case();
        let ty_str = if ty.fields[0].optional {
            format!("{}?", render_type(&ty.fields[0].ty, imports))
        } else {
            render_type(&ty.fields[0].ty, imports)
        };
        out.push_str(&format!("  {}(this.{name});\n", ty.name));
        let _ = ty_str; // used above for field emission, constructor uses `this.`
    } else {
        out.push_str(&format!("  {}({{\n", ty.name));
        for field in &ty.fields {
            let name = field.name.to_lower_camel_case();
            out.push_str(&format!("    required this.{name},\n"));
        }
        out.push_str("  });\n");
    }
    out.push_str("}\n");
}

pub(super) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        for line in en.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&format!("enum {} {{\n", en.name));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            if !variant.doc.is_empty() {
                for line in variant.doc.lines() {
                    out.push_str("  /// ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            let vname = variant.name.to_lower_camel_case();
            let semicolon = if idx + 1 == count { ";" } else { "," };
            out.push_str(&format!("  {vname}{semicolon}\n"));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&format!("sealed class {} {{}}\n", en.name));
        for variant in &en.variants {
            if !variant.doc.is_empty() {
                for line in variant.doc.lines() {
                    out.push_str("/// ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            if variant.fields.is_empty() {
                out.push_str(&format!("final class {} extends {} {{}}\n", variant.name, en.name));
            } else {
                out.push_str(&format!("final class {} extends {} {{\n", variant.name, en.name));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = DartMapper.map_type(&f.ty);
                    let fname = if f.name.is_empty() {
                        format!("field{idx}")
                    } else {
                        f.name.to_lower_camel_case()
                    };
                    out.push_str(&format!("  final {ty_str} {fname};\n"));
                }
                if variant.fields.len() == 1 {
                    let fname = if variant.fields[0].name.is_empty() {
                        "field0".to_string()
                    } else {
                        variant.fields[0].name.to_lower_camel_case()
                    };
                    out.push_str(&format!("  {}(this.{fname});\n", variant.name));
                } else {
                    out.push_str(&format!("  {}({{\n", variant.name));
                    for (idx, f) in variant.fields.iter().enumerate() {
                        let fname = if f.name.is_empty() {
                            format!("field{idx}")
                        } else {
                            f.name.to_lower_camel_case()
                        };
                        out.push_str(&format!("    required this.{fname},\n"));
                    }
                    out.push_str("  });\n");
                }
                out.push_str("}\n");
            }
        }
    }
}
