use crate::core::config::Language;
use crate::core::ir::ErrorDef;
use crate::docs::descriptions::generate_error_variant_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings};
use crate::docs::formatting::escape_table_cell;
use crate::docs::naming::{enum_variant_name, type_name};
use crate::docs::{clean_doc, template_env};
use heck::ToPascalCase;

pub(super) fn render_error(err: &ErrorDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&err.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => &ename },
    ));

    let doc = clean_doc(&err.doc, lang);
    let doc = demote_headings(&doc, 2);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    if matches!(lang, Language::Node | Language::Wasm) {
        out.push_str("Errors are thrown as plain `Error` objects with descriptive messages.\n\n");
    }

    if lang == Language::Python {
        out.push_str(&template_env::render(
            "base_class.jinja",
            minijinja::context! { name => &ename },
        ));
        out.push('\n');
        out.push_str("| Exception | Description |\n");
        out.push_str("|-----------|-------------|\n");
        for variant in &err.variants {
            let vname = variant.name.to_pascal_case();
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            out.push_str(&template_env::render(
                "exception_row.jinja",
                minijinja::context! {
                    variant => escape_table_cell(&vname),
                    error => escape_table_cell(&ename),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        }
    } else {
        out.push('\n');
        out.push_str("| Variant | Description |\n");
        out.push_str("|---------|-------------|\n");
        for variant in &err.variants {
            let vname = enum_variant_name(&variant.name, lang, ffi_prefix);
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, lang)
            } else if let Some(tmpl) = &variant.message_template {
                clean_doc_inline(tmpl, lang)
            } else {
                generate_error_variant_description(&variant.name)
            };
            out.push_str(&template_env::render(
                "variant_row.jinja",
                minijinja::context! { name => escape_table_cell(&vname), doc => escape_table_cell(&vdoc) },
            ));
        }
    }
    out.push('\n');

    out
}
