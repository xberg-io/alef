use crate::core::ir::ErrorDef;
use std::collections::BTreeSet;

use super::types::{fits_single_line, kotlin_type_with_string_imports, kotlin_zero_value};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::{kotlin_field_name, to_lower_camel};

fn interpolate_error_message_template(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let after_open = &remaining[open + 1..];
        if let Some(close) = after_open.find('}') {
            let token = &after_open[..close];
            if token.chars().all(|c| c.is_ascii_digit()) && !token.is_empty() {
                out.push_str(&remaining[..open]);
                let after_close = &after_open[close + 1..];
                let next_is_ident_cont = after_close
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                if next_is_ident_cont {
                    out.push_str("${field");
                    out.push_str(token);
                    out.push('}');
                } else {
                    out.push_str("$field");
                    out.push_str(token);
                }
                remaining = &remaining[open + 1 + close + 1..];
                continue;
            }
        }
        out.push_str(&remaining[..open + 1]);
        remaining = &remaining[open + 1..];
    }
    out.push_str(remaining);
    out
}

pub(crate) fn emit_error_type_with_imports(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    emit_cleaned_kdoc(out, &error.doc, "");
    out.push_str(&crate::backends::kotlin::template_env::render(
        "error_sealed_class_header.jinja",
        minijinja::context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            let message = interpolate_error_message_template(raw_msg);
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_object_variant.jinja",
                minijinja::context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => message,
                },
            ));
        } else {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            let message = interpolate_error_message_template(raw_msg);

            let mut err_field_strings: Vec<String> = Vec::with_capacity(variant.fields.len());
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                let modifier = if name == "message" { "override " } else { "" };
                err_field_strings.push(format!("{modifier}val {name}: {ty_str}"));
            }

            let err_prefix = format!("data class {}", variant.name);
            let err_suffix = format!(" : {}(\"{message}\")", error.name);
            let use_single_line = fits_single_line("    ", &err_prefix, &err_field_strings, &err_suffix);

            if use_single_line {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_inline.jinja",
                    minijinja::context! {
                        err_prefix => err_prefix,
                        fields => err_field_strings.join(", "),
                        err_suffix => err_suffix,
                    },
                ));
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_header.jinja",
                    minijinja::context! {
                        err_prefix => err_prefix,
                    },
                ));
                for field_str in &err_field_strings {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "error_variant_field.jinja",
                        minijinja::context! {
                            field => field_str,
                        },
                    ));
                }
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_close_multiline.jinja",
                    minijinja::context! {
                        err_suffix => err_suffix,
                    },
                ));
            }
        }
    }
    for method in error.methods.iter().filter(|m| !m.sanitized) {
        let prop_name = to_lower_camel(&method.name);
        let ty_str = kotlin_type_with_string_imports(&method.return_type, false, imports);
        let default = kotlin_zero_value(&ty_str);
        out.push_str(&crate::backends::kotlin::template_env::render(
            "error_open_property.jinja",
            minijinja::context! {
                prop_name => prop_name,
                ty => ty_str,
                default => default,
            },
        ));
    }
    out.push_str("}\n");
}
