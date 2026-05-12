use alef_codegen::keywords::zig_ident;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef, TypeRef};

use crate::type_map::ZigMapper;

use super::helpers::emit_cleaned_zig_doc;

pub(crate) fn emit_type(ty: &TypeDef, out: &mut String) {
    emit_cleaned_zig_doc(out, &ty.doc, "");
    out.push_str(&crate::template_env::render(
        "type_header.jinja",
        minijinja::context! {
            type_name => &ty.name,
        },
    ));
    for field in &ty.fields {
        let ty_str = zig_field_type(&field.ty, field.optional);
        out.push_str(&crate::template_env::render(
            "type_field.jinja",
            minijinja::context! {
                field_name => zig_ident(&field.name),
                field_type => ty_str,
            },
        ));
    }
    out.push_str("};\n");
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    emit_cleaned_zig_doc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::template_env::render(
            "enum_unit_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            out.push_str(&crate::template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => zig_ident(&to_snake_case(&variant.name)),
                },
            ));
        }
        out.push_str("};\n");
    } else {
        out.push_str(&crate::template_env::render(
            "enum_tagged_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            let tag = zig_ident(&to_snake_case(&variant.name));
            if variant.fields.is_empty() {
                out.push_str(&crate::template_env::render(
                    "enum_variant_void.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
            } else if variant.fields.len() == 1 {
                let ty_str = zig_field_type(&variant.fields[0].ty, variant.fields[0].optional);
                out.push_str(&crate::template_env::render(
                    "enum_variant_single.jinja",
                    minijinja::context! {
                        tag => &tag,
                        type_str => ty_str,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "enum_variant_struct_header.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
                for f in &variant.fields {
                    let name = if f.name.is_empty() {
                        "value".into()
                    } else {
                        zig_ident(&f.name)
                    };
                    let ty_str = zig_field_type(&f.ty, f.optional);
                    out.push_str(&crate::template_env::render(
                        "enum_variant_struct_field.jinja",
                        minijinja::context! {
                            field_name => name,
                            field_type => ty_str,
                        },
                    ));
                }
                out.push_str("    },\n");
            }
        }
        out.push_str("};\n");
    }
}

pub(crate) fn zig_field_type(ty: &TypeRef, optional: bool) -> String {
    let mapper = ZigMapper;
    let inner = mapper.map_type(ty);
    if optional { format!("?{inner}") } else { inner }
}

pub(crate) fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
