use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::ident::{dart_safe_ident, dart_safe_type_name};
use crate::template_env;
use crate::type_map::DartMapper;

use super::render_type::render_type;

#[allow(dead_code)]
pub(super) fn emit_type(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !ty.doc.is_empty() {
        let doc_lines: Vec<String> = ty.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    if ty.fields.is_empty() {
        out.push_str(&template_env::render(
            "class_empty.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        return;
    }
    out.push_str(&template_env::render(
        "class_open.jinja",
        minijinja::context! {
            name => ty.name.as_str(),
        },
    ));
    for field in &ty.fields {
        let ty_str = if field.optional {
            format!("{}?", render_type(&field.ty, imports))
        } else {
            render_type(&field.ty, imports)
        };
        let name = dart_safe_ident(&field.name.to_lower_camel_case());
        if !field.doc.is_empty() {
            let doc_lines: Vec<String> = field.doc.lines().map(ToString::to_string).collect();
            out.push_str(&template_env::render(
                "doc_comment.jinja",
                minijinja::context! {
                    indent => "  ",
                    lines => doc_lines,
                },
            ));
        }
        out.push_str(&template_env::render(
            "final_field_decl.jinja",
            minijinja::context! {
                ty_str => ty_str,
                name => name.as_str(),
            },
        ));
    }
    // Constructor
    if ty.fields.len() == 1 {
        let name = dart_safe_ident(&ty.fields[0].name.to_lower_camel_case());
        let ty_str = if ty.fields[0].optional {
            format!("{}?", render_type(&ty.fields[0].ty, imports))
        } else {
            render_type(&ty.fields[0].ty, imports)
        };
        out.push_str(&template_env::render(
            "single_param_constructor.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
                param_name => name.as_str(),
            },
        ));
        let _ = ty_str; // used above for field emission, constructor uses `this.`
    } else {
        out.push_str(&template_env::render(
            "multi_param_constructor_open.jinja",
            minijinja::context! {
                name => ty.name.as_str(),
            },
        ));
        for field in &ty.fields {
            let name = dart_safe_ident(&field.name.to_lower_camel_case());
            out.push_str(&template_env::render(
                "constructor_required_param.jinja",
                minijinja::context! {
                    name => name.as_str(),
                },
            ));
        }
        out.push_str(&template_env::render("constructor_close.jinja", minijinja::context! {}));
    }
    out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
}

#[allow(dead_code)]
pub(super) fn emit_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        let doc_lines: Vec<String> = en.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&template_env::render(
            "enum_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        let count = en.variants.len();
        for (idx, variant) in en.variants.iter().enumerate() {
            if !variant.doc.is_empty() {
                let doc_lines: Vec<String> = variant.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "  ",
                        lines => doc_lines,
                    },
                ));
            }
            let vname = dart_safe_ident(&variant.name.to_lower_camel_case());
            let suffix = if idx + 1 == count { ";" } else { "," };
            out.push_str(&template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    vname => vname.as_str(),
                    suffix => suffix,
                },
            ));
        }
        out.push_str(&template_env::render("enum_close.jinja", minijinja::context! {}));
    } else {
        out.push_str(&template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => en.name.as_str(),
            },
        ));
        for variant in &en.variants {
            // Use dart_safe_type_name to avoid shadowing Dart core types (e.g. `List`, `Map`).
            let safe_variant_name = dart_safe_type_name(&variant.name, Some(&en.name));
            if !variant.doc.is_empty() {
                let doc_lines: Vec<String> = variant.doc.lines().map(ToString::to_string).collect();
                out.push_str(&template_env::render(
                    "doc_comment.jinja",
                    minijinja::context! {
                        indent => "",
                        lines => doc_lines,
                    },
                ));
            }
            if variant.fields.is_empty() {
                out.push_str(&template_env::render(
                    "final_class_extends.jinja",
                    minijinja::context! {
                        name => safe_variant_name.as_str(),
                        parent => en.name.as_str(),
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "final_class_header.jinja",
                    minijinja::context! {
                        name => safe_variant_name.as_str(),
                        parent => en.name.as_str(),
                    },
                ));
                for f in variant.fields.iter() {
                    let ty_str = DartMapper.map_type(&f.ty);
                    let fname = dart_safe_ident(&f.name.to_lower_camel_case());
                    out.push_str(&template_env::render(
                        "final_field_decl.jinja",
                        minijinja::context! {
                            ty_str => ty_str,
                            name => fname.as_str(),
                        },
                    ));
                }
                if variant.fields.len() == 1 {
                    let fname = dart_safe_ident(&variant.fields[0].name.to_lower_camel_case());
                    out.push_str(&template_env::render(
                        "single_param_constructor.jinja",
                        minijinja::context! {
                            name => safe_variant_name.as_str(),
                            param_name => fname.as_str(),
                        },
                    ));
                } else {
                    out.push_str(&template_env::render(
                        "multi_param_constructor_open.jinja",
                        minijinja::context! {
                            name => safe_variant_name.as_str(),
                        },
                    ));
                    for f in variant.fields.iter() {
                        let fname = dart_safe_ident(&f.name.to_lower_camel_case());
                        out.push_str(&template_env::render(
                            "constructor_required_param.jinja",
                            minijinja::context! {
                                name => fname.as_str(),
                            },
                        ));
                    }
                    out.push_str(&template_env::render("constructor_close.jinja", minijinja::context! {}));
                }
                out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
            }
        }
    }
}
