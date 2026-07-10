//! Generation of return-type `TypedDict` classes for the Python options module.

use super::python_field_type;
use crate::backends::pyo3::gen_bindings::enums::{EmitContext, class_name_to_docstring, sanitize_python_doc};
use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::codegen::shared::binding_fields;
use ahash::AHashSet;

/// Generate a `TypedDict` class for a return type.
///
/// TypedDict is emitted with `total=False` because all fields are optional at the
/// call site — the caller may receive only a subset of keys.  Default values are
/// not supported by TypedDict, so we only emit field name + type hint.
///
/// ```python
/// class ParseOutput(TypedDict, total=False):
///     """One-line doc."""
///
///     content: str | None
///     tables: list[ExtractedTable]
/// ```
pub(super) fn gen_typeddict(
    typ: &crate::core::ir::TypeDef,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
    str_coercible_data_enums: &AHashSet<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&crate::backends::pyo3::template_env::render(
        "typeddict_header.jinja",
        minijinja::context! { name => &typ.name },
    ));
    let typeddict_doc = if !typ.doc.is_empty() {
        let raw = doc_first_paragraph_joined(&typ.doc);
        let first = sanitize_python_doc(&raw);
        let content = if first.len() > 89 {
            first[..89].to_string()
        } else {
            first
        };
        if content.ends_with(['.', '?', '!']) {
            content
        } else {
            format!("{}.", content)
        }
    } else {
        class_name_to_docstring(&typ.name)
    };
    out.push_str(&crate::backends::pyo3::template_env::render(
        "class_docstring.jinja",
        minijinja::context! { doc => &typeddict_doc },
    ));
    for field in binding_fields(&typ.fields) {
        let type_hint = python_field_type(
            &field.ty,
            field.optional,
            enum_names,
            data_enum_names,
            str_coercible_data_enums,
            EmitContext::OptionsModule,
        );
        let type_hint_with_none = if field.optional && !type_hint.contains("None") {
            if matches!(&field.ty, crate::core::ir::TypeRef::Named(_)) {
                format!("{} | None", type_hint)
            } else {
                type_hint
            }
        } else {
            type_hint
        };
        let safe_name = crate::core::keywords::python_ident(&field.name);
        if !field.doc.is_empty() {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "typeddict_field.jinja",
                minijinja::context! {
                    name => &safe_name,
                    type_hint => &type_hint_with_none,
                },
            ));
            out.push('\n');
            let doc_line = sanitize_python_doc(&doc_first_paragraph_joined(&field.doc));
            let safe_doc = if doc_line.ends_with('"') {
                format!("{doc_line} ")
            } else {
                doc_line
            };
            out.push_str(&crate::backends::pyo3::template_env::render(
                "typeddict_field_docstring.jinja",
                minijinja::context! { doc => &safe_doc },
            ));
        } else {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "typeddict_field.jinja",
                minijinja::context! {
                    name => &safe_name,
                    type_hint => &type_hint_with_none,
                },
            ));
            out.push('\n');
        }
    }
    out.push('\n');
    out
}
