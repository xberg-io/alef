//! Kotlin/Native type, enum, and error emission for data class declarations.
//!
//! These emitters produce Kotlin code for data types — they are distinct from
//! function body emission because data classes use the Kotlin/Native type set
//! (no `cinterop` types in struct fields).

use crate::core::ir::{EnumDef, ErrorDef, TypeDef};

use super::native_type_str;
use crate::backends::kotlin::gen_bindings::{kotlin_field_name, to_screaming_snake};

pub(super) fn emit_native_type(ty: &TypeDef, out: &mut String) {
    if !ty.doc.is_empty() {
        let doc_lines: Vec<String> = ty.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    if ty.fields.is_empty() {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "empty_class.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }
    out.push_str(&crate::backends::kotlin::template_env::render(
        "data_class_header.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = native_type_str(&field.ty, field.optional);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&crate::backends::kotlin::template_env::render(
            "class_field.jinja",
            minijinja::context! {
                name => &name,
                type => &ty_str,
                comma => comma,
            },
        ));
    }

    // Emit inherent instance methods for Kotlin/Native data classes
    use crate::codegen::shared::partition_methods;
    let (instance_methods, _) = partition_methods(&ty.methods);

    if !instance_methods.is_empty() {
        // Complete the data class constructor without closing braces yet
        out.push_str(") {\n");

        // Emit instance methods inside the class body
        for method in instance_methods {
            if method.sanitized {
                continue; // Skip sanitized methods
            }

            let method_name = heck::AsLowerCamelCase(method.name.as_str()).to_string();
            let return_type_str = native_type_str(&method.return_type, false);

            // Build parameter signature
            let params_sig: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let ptype = native_type_str(&p.ty, p.optional);
                    format!("{}: {ptype}", p.name)
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
                out.push_str(&p.name);
            }
            out.push_str(")\n");
        }

        // Close the data class body
        out.push_str("}\n");
    } else {
        // No methods - just close the constructor
        out.push_str(")\n");
    }
}

pub(super) fn emit_native_enum(en: &EnumDef, out: &mut String) {
    if !en.doc.is_empty() {
        let doc_lines: Vec<String> = en.doc.lines().map(ToString::to_string).collect();
        out.push_str(&crate::backends::kotlin::template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "enum_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&crate::backends::kotlin::template_env::render(
                "enum_variant.jinja",
                minijinja::context! {
                    name => name,
                    comma => comma,
                },
            ));
        }
        out.push_str("}\n");
    } else {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "variant_data_class_header.jinja",
                    minijinja::context! {
                        name => &variant.name,
                    },
                ));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = native_type_str(&f.ty, f.optional);
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "variant_class_field.jinja",
                        minijinja::context! {
                            name => &name,
                            type => &ty_str,
                            comma => comma,
                        },
                    ));
                }
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "variant_close.jinja",
                    minijinja::context! {
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
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    out.push_str(&crate::backends::kotlin::template_env::render(
        "error_sealed_class_header.jinja",
        minijinja::context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_object_variant.jinja",
                minijinja::context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => variant.message_template.as_deref().unwrap_or(&variant.name),
                },
            ));
        } else {
            out.push_str(&crate::backends::kotlin::template_env::render(
                "variant_data_class_header.jinja",
                minijinja::context! {
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
                    minijinja::context! {
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
                minijinja::context! {
                    parent_name => &error.name,
                    message => message_template,
                },
            ));
        }
    }
    out.push_str("}\n");
}
