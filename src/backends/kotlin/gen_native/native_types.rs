//! Kotlin/Native type, enum, and error emission for data class declarations.
//!
//! These emitters produce Kotlin code for data types — they are distinct from
//! function body emission because data classes use the Kotlin/Native type set
//! (no `cinterop` types in struct fields).

use crate::core::ir::{EnumDef, ErrorDef, TypeDef};

use super::native_type_str;
use crate::backends::kotlin::gen_bindings::{
    escape_kotlin_ident, kotlin_field_name, to_lower_camel, to_screaming_snake,
};

pub(super) fn emit_native_type(ty: &TypeDef, out: &mut String) {
    if !ty.doc.is_empty() {
        let doc_lines: Vec<String> = ty.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            crate::alef_context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    if ty.fields.is_empty() {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "empty_class.jinja",
            crate::alef_context! {
                name => &ty.name,
            },
        ));
        return;
    }
    out.push_str(&crate::backends::kotlin::template_env::render(
        "data_class_header.jinja",
        crate::alef_context! {
            name => &ty.name,
        },
    ));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = native_type_str(&field.ty, field.optional);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&crate::backends::kotlin::template_env::render(
            "class_field.jinja",
            crate::alef_context! {
                name => &name,
                type => &ty_str,
                comma => comma,
            },
        ));
    }

    use crate::codegen::shared::partition_methods;
    let (instance_methods, _) = partition_methods(&ty.methods);

    if !instance_methods.is_empty() {
        out.push_str(") {\n");

        for method in instance_methods {
            if method.sanitized {
                continue;
            }

            let method_name = to_lower_camel(&method.name);
            let return_type_str = native_type_str(&method.return_type, false);

            // Parameters here keep their raw Rust `snake_case` spelling rather than being
            // lower-camelCased like every other Kotlin parameter site; only the keyword
            // escape is applied, so fixing the reserved-word break does not also migrate
            // the parameter names of every existing Kotlin/Native consumer. ~keep
            let params_sig: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let ptype = native_type_str(&p.ty, p.optional);
                    format!("{}: {ptype}", escape_kotlin_ident(&p.name))
                })
                .collect();

            out.push_str("    fun ");
            out.push_str(&method_name);
            out.push('(');
            out.push_str(&params_sig.join(", "));
            out.push_str("): ");
            out.push_str(&return_type_str);
            out.push_str(" = nativeInterop.");
            out.push_str(&ty.name);
            out.push('_');
            out.push_str(&method.name);
            out.push('(');
            out.push_str("this");
            for p in &method.params {
                out.push_str(", ");
                out.push_str(&escape_kotlin_ident(&p.name));
            }
            out.push_str(")\n");
        }

        out.push_str("}\n");
    } else {
        out.push_str(")\n");
    }
}

pub(super) fn emit_native_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        let doc_lines: Vec<String> = en.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            crate::alef_context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "enum_class_header.jinja",
            crate::alef_context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&crate::backends::kotlin::template_env::render(
                "enum_variant.jinja",
                crate::alef_context! {
                    name => name,
                    comma => comma,
                },
            ));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "sealed_class_header.jinja",
            crate::alef_context! {
                name => &en.name,
            },
        ));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "sealed_object_variant.jinja",
                    crate::alef_context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "variant_data_class_header.jinja",
                    crate::alef_context! {
                        name => &variant.name,
                    },
                ));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = native_type_str(&f.ty, f.optional);
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "variant_class_field.jinja",
                        crate::alef_context! {
                            name => &name,
                            type => &ty_str,
                            comma => comma,
                        },
                    ));
                }
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "variant_close.jinja",
                    crate::alef_context! {
                        parent_name => &en.name,
                    },
                ));
            }
        }
        out.push_str("}\n");
    }
}

pub(super) fn emit_native_error(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        let doc_lines: Vec<String> = error.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            crate::alef_context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    out.push_str(&crate::backends::kotlin::template_env::render(
        "error_sealed_class_header.jinja",
        crate::alef_context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_object_variant.jinja",
                crate::alef_context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => variant.message_template.as_deref().unwrap_or(&variant.name),
                },
            ));
        } else {
            out.push_str(&crate::backends::kotlin::template_env::render(
                "variant_data_class_header.jinja",
                crate::alef_context! {
                    name => &variant.name,
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = native_type_str(&f.ty, f.optional);
                let name = kotlin_field_name(&f.name, idx);
                let modifier = if name == "message" { "override " } else { "" };
                let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_field.jinja",
                    crate::alef_context! {
                        modifier => modifier,
                        name => &name,
                        type => &ty_str,
                        comma => comma,
                    },
                ));
            }
            let message_template = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_variant_close.jinja",
                crate::alef_context! {
                    parent_name => &error.name,
                    message => message_template,
                },
            ));
        }
    }
    out.push_str("}\n");
}
