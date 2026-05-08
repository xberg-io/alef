use alef_codegen::error_gen::strip_thiserror_placeholders;
use alef_core::ir::ErrorDef;
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::ident::dart_safe_ident;
use crate::template_env;

use super::render_type::render_type;

/// Escape a string for use inside a Dart single-quoted string literal.
///
/// Dart single-quoted strings interpret `\`, `'`, and `$` specially:
/// - `\` introduces an escape sequence → must be doubled.
/// - `'` terminates the literal → must be escaped as `\'`.
/// - `$` introduces string interpolation → must be escaped as `\$`.
#[allow(dead_code)]
fn escape_dart_string_literal(s: &str) -> String {
    s.replace('\\', r"\\").replace('\'', r"\'").replace('$', r"\$")
}

/// Build the runtime `message` string for a Dart exception variant.
///
/// Strips `thiserror`-style `{name}` placeholders so the host runtime never
/// surfaces literal substitution markers (`Parsing error: {message}` becomes
/// `Parsing error`). When the template is empty (or stripping leaves nothing)
/// falls back to the variant name to preserve some context.
#[allow(dead_code)]
fn build_message(variant_name: &str, template: Option<&str>) -> String {
    let raw = template.unwrap_or(variant_name);
    let stripped = strip_thiserror_placeholders(raw);
    if stripped.is_empty() {
        variant_name.to_string()
    } else {
        stripped
    }
}

#[allow(dead_code)]
pub(super) fn emit_error(error: &ErrorDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !error.doc.is_empty() {
        for line in error.doc.lines() {
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&template_env::render(
        "error_sealed_class.jinja",
        minijinja::context! {
            name => error.name.as_str(),
        },
    ));
    out.push('\n');
    for variant in &error.variants {
        if !variant.doc.is_empty() {
            for line in variant.doc.lines() {
                out.push_str("/// ");
                out.push_str(line);
                out.push('\n');
            }
        }
        if variant.is_unit {
            let raw_msg = build_message(&variant.name, variant.message_template.as_deref());
            let msg = escape_dart_string_literal(&raw_msg);
            out.push_str(&template_env::render(
                "error_class_header.jinja",
                minijinja::context! {
                    name => variant.name.as_str(),
                    error_name => error.name.as_str(),
                },
            ));
            out.push_str(&template_env::render(
                "override_message_getter.jinja",
                minijinja::context! {
                    msg => msg,
                },
            ));
            out.push_str(&template_env::render(
                "const_constructor.jinja",
                minijinja::context! {
                    name => variant.name.as_str(),
                },
            ));
            out.push_str(&template_env::render("class_close.jinja", minijinja::context! {}));
        } else {
            out.push_str(&template_env::render(
                "error_class_header.jinja",
                minijinja::context! {
                    name => variant.name.as_str(),
                    error_name => error.name.as_str(),
                },
            ));
            for f in &variant.fields {
                let ty_str = render_type(&f.ty, imports);
                let fname = dart_safe_ident(&f.name.to_lower_camel_case());
                out.push_str(&template_env::render(
                    "final_field_decl.jinja",
                    minijinja::context! {
                        ty_str => ty_str,
                        name => fname.as_str(),
                    },
                ));
            }
            let raw_msg = build_message(&variant.name, variant.message_template.as_deref());
            let msg = escape_dart_string_literal(&raw_msg);
            out.push_str("  @override\n");
            out.push_str(&template_env::render(
                "override_message_getter.jinja",
                minijinja::context! {
                    msg => msg,
                },
            ));
            if variant.fields.len() == 1 {
                let fname = dart_safe_ident(&variant.fields[0].name.to_lower_camel_case());
                out.push_str(&template_env::render(
                    "single_param_constructor.jinja",
                    minijinja::context! {
                        name => variant.name.as_str(),
                        param_name => fname.as_str(),
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "multi_param_constructor_open.jinja",
                    minijinja::context! {
                        name => variant.name.as_str(),
                    },
                ));
                for f in &variant.fields {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_message_strips_placeholders() {
        assert_eq!(
            build_message("Parsing", Some("Parsing error: {message}")),
            "Parsing error"
        );
        assert_eq!(build_message("Ocr", Some("OCR error: {message}")), "OCR error");
        assert_eq!(
            build_message("Cancelled", Some("extraction cancelled")),
            "extraction cancelled"
        );
    }

    #[test]
    fn build_message_falls_back_when_stripped_empty() {
        assert_eq!(build_message("Other", Some("{message}")), "Other");
    }

    #[test]
    fn build_message_no_template_uses_variant_name() {
        assert_eq!(build_message("NotFound", None), "NotFound");
    }
}
