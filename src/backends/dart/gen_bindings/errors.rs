use crate::codegen::error_gen::strip_thiserror_placeholders;
use crate::core::ir::ErrorDef;
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;

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
        let doc_lines: Vec<String> = error.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "",
                lines => doc_lines,
            },
        ));
    }
    // Pre-render method descriptors: (dart_type, dart_getter_name) pairs.
    // The sealed class declares each as an abstract getter; concrete variant
    // subclasses may override them when they naturally carry the relevant field.
    let method_entries: Vec<(String, String)> = error
        .methods
        .iter()
        .filter(|m| !m.sanitized)
        .map(|m| {
            let ty = render_type(&m.return_type, imports);
            let getter_name = dart_safe_ident(&m.name.to_lower_camel_case());
            (ty, getter_name)
        })
        .collect();
    let methods_ctx: Vec<minijinja::Value> = method_entries
        .iter()
        .map(|(ty, name)| minijinja::Value::from_iter([("return_type", ty.as_str()), ("name", name.as_str())]))
        .collect();
    out.push_str(&template_env::render(
        "error_sealed_class.jinja",
        minijinja::context! {
            name => error.name.as_str(),
            methods => methods_ctx,
        },
    ));
    out.push('\n');
    for variant in &error.variants {
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

    #[test]
    fn emit_error_with_methods_adds_abstract_getters() {
        use crate::core::ir::{MethodDef, ReceiverKind};

        let error = crate::core::ir::ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "demo::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant {
                name: "NotFound".to_string(),
                message_template: Some("not found".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![
                MethodDef {
                    name: "status_code".to_string(),
                    params: vec![],
                    return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "is_transient".to_string(),
                    params: vec![],
                    return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                MethodDef {
                    name: "error_type".to_string(),
                    params: vec![],
                    return_type: crate::core::ir::TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::Ref),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        emit_error(&error, &mut out, &mut imports);
        // Sealed class must declare abstract getters for each method.
        assert!(out.contains("int get statusCode;"), "missing statusCode: {out}");
        assert!(out.contains("bool get isTransient;"), "missing isTransient: {out}");
        assert!(out.contains("String get errorType;"), "missing errorType: {out}");
    }
}
