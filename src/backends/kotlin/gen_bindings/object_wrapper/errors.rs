use crate::core::ir::ErrorDef;
use std::collections::BTreeSet;

use super::types::{fits_single_line, kotlin_type_with_string_imports, kotlin_zero_value};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::{kotlin_field_name, to_lower_camel};

fn interpolate_error_message_template(template: &str, redacted: &[usize]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let after_open = &remaining[open + 1..];
        if let Some(close) = after_open.find('}') {
            let token = &after_open[..close];
            if token.chars().all(|c| c.is_ascii_digit()) && !token.is_empty() {
                out.push_str(&remaining[..open]);
                if token.parse::<usize>().is_ok_and(|index| redacted.contains(&index)) {
                    out.push_str("<redacted>");
                    remaining = &after_open[close + 1..];
                    continue;
                }
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
    let open_properties: Vec<(String, String)> = error
        .methods
        .iter()
        .filter(|m| !m.sanitized)
        .map(|m| {
            (
                to_lower_camel(&m.name),
                kotlin_type_with_string_imports(&m.return_type, false, imports),
            )
        })
        .collect();
    emit_cleaned_kdoc(out, &error.doc, "");
    out.push_str(&crate::backends::kotlin::template_env::render(
        "error_sealed_class_header.jinja",
        crate::alef_context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            let message = interpolate_error_message_template(raw_msg, &[]);
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_object_variant.jinja",
                crate::alef_context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => message,
                },
            ));
        } else {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            let redacted: Vec<usize> = variant
                .fields
                .iter()
                .enumerate()
                .filter_map(|(idx, f)| f.sensitive.then_some(idx))
                .collect();
            let message = interpolate_error_message_template(raw_msg, &redacted);

            let mut err_field_strings: Vec<String> = Vec::with_capacity(variant.fields.len());
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                // ~keep A variant field that shares a name and type with an `open val` error
                // accessor declared on the sealed base must override it, or kotlinc rejects
                // the data class ("hides member of supertype").
                let overrides_accessor = open_properties.iter().any(|(n, t)| *n == name && *t == ty_str);
                let modifier = if name == "message" || overrides_accessor {
                    "override "
                } else {
                    ""
                };
                err_field_strings.push(format!("{modifier}val {name}: {ty_str}"));
            }

            let err_prefix = format!("data class {}", variant.name);
            let err_suffix = format!(" : {}(\"{message}\")", error.name);
            let use_single_line = fits_single_line("    ", &err_prefix, &err_field_strings, &err_suffix);

            if use_single_line {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_inline.jinja",
                    crate::alef_context! {
                        err_prefix => err_prefix,
                        fields => err_field_strings.join(", "),
                        err_suffix => err_suffix,
                    },
                ));
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_header.jinja",
                    crate::alef_context! {
                        err_prefix => err_prefix,
                    },
                ));
                for field_str in &err_field_strings {
                    out.push_str(&crate::backends::kotlin::template_env::render(
                        "error_variant_field.jinja",
                        crate::alef_context! {
                            field => field_str,
                        },
                    ));
                }
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "error_variant_close_multiline.jinja",
                    crate::alef_context! {
                        err_suffix => err_suffix,
                    },
                ));
            }
        }
    }
    for (prop_name, ty_str) in open_properties {
        let default = kotlin_zero_value(&ty_str);
        out.push_str(&crate::backends::kotlin::template_env::render(
            "error_open_property.jinja",
            crate::alef_context! {
                prop_name => prop_name,
                ty => ty_str,
                default => default,
            },
        ));
    }
    out.push_str("}\n");
}

#[cfg(test)]
#[path = "errors_tests.rs"]
mod errors_tests;
