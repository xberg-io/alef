use crate::codegen::shared::binding_fields;
use crate::core::config::Language;
use crate::core::ir::EnumDef;
use crate::docs::descriptions::generate_enum_variant_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings};
use crate::docs::formatting::escape_table_cell;
use crate::docs::naming::{enum_variant_name, field_name, type_name};
use crate::docs::{clean_doc, doc_type, template_env, version_labels};

use super::function_render::push_version_annotation;

pub(super) fn render_enum(en: &EnumDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&en.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => ename },
    ));

    push_version_annotation(&mut out, &en.version);

    let doc = clean_doc(&en.doc, lang);
    let doc = demote_headings(&doc, 2);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    out.push_str("| Value | Description |\n");
    out.push_str("|-------|-------------|\n");
    for variant in &en.variants {
        let vname = enum_variant_name(&variant.name, lang, ffi_prefix);
        let mut vdoc = if !variant.doc.is_empty() {
            clean_doc_inline(&variant.doc, lang)
        } else {
            generate_enum_variant_description(&variant.name)
        };
        let variant_fields: Vec<_> = if lang == Language::Rust {
            variant.fields.iter().collect()
        } else {
            binding_fields(&variant.fields).collect()
        };
        if !variant_fields.is_empty() {
            let fields_desc: Vec<String> = variant_fields
                .into_iter()
                .map(|f| {
                    let fname = field_name(&f.name, lang);
                    let fty = doc_type(&f.ty, lang, ffi_prefix);
                    format!("`{fname}`: `{fty}`")
                })
                .collect();
            vdoc = format!("{vdoc} — Fields: {}", fields_desc.join(", "));
        }
        if let Some(ref since) = variant.version.since {
            let since = version_labels::major_minor(since);
            vdoc = format!("{vdoc} — **Since:** `v{since}`");
        }
        if let Some(ref dep) = variant.version.deprecated {
            let dep_note = match (&dep.since, &dep.note) {
                (Some(s), Some(n)) => format!("Deprecated since `v{}`: {n}", version_labels::major_minor(s)),
                (Some(s), None) => format!("Deprecated since `v{}`", version_labels::major_minor(s)),
                (None, Some(n)) => format!("Deprecated: {n}"),
                (None, None) => "Deprecated".to_string(),
            };
            vdoc = format!("{vdoc} — {dep_note}");
        }
        out.push_str(&template_env::render(
            "variant_row.jinja",
            minijinja::context! { name => escape_table_cell(&vname), doc => escape_table_cell(&vdoc) },
        ));
    }
    out.push('\n');

    out
}
