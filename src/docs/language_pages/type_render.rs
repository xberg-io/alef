use crate::codegen::shared::binding_fields;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use crate::docs::descriptions::generate_field_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_field_default};
use crate::docs::naming::{field_name, type_name};
use crate::docs::{clean_doc, template_env};

use super::function_render::push_version_annotation;
use super::streaming::{method_visible_in_lang, render_method};

const TYPE_DOC_FIRST_HEADING_LEVEL: usize = 5;

pub(super) fn render_type(
    ty: &TypeDef,
    lang: Language,
    config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let tname = type_name(&ty.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => tname },
    ));

    push_version_annotation(&mut out, &ty.version);

    let doc = clean_doc(&ty.doc, lang);
    let doc = demote_headings_to_start_at(&doc, TYPE_DOC_FIRST_HEADING_LEVEL);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let fields: Vec<_> = if lang == Language::Rust {
        ty.fields.iter().collect()
    } else {
        binding_fields(&ty.fields).collect()
    };
    if !ty.is_opaque && !fields.is_empty() {
        out.push('\n');
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("|-------|------|---------|-------------|\n");
        for field in fields {
            let fname = field_name(&field.name, lang);
            let fty = doc_type_with_optional(&field.ty, lang, field.optional, ffi_prefix);
            let fdefault = format_field_default(field, lang, api, ffi_prefix);
            let fdoc = {
                let raw = clean_doc_inline(&field.doc, lang);
                if raw.is_empty() {
                    generate_field_description(&field.name, &field.ty)
                } else {
                    raw
                }
            };
            out.push_str(&template_env::render(
                "field_row.jinja",
                minijinja::context! {
                    name => escape_table_cell(&fname),
                    ty => escape_table_cell(&fty),
                    default => escape_table_cell(&fdefault),
                    doc => escape_table_cell(&fdoc),
                },
            ));
        }
        out.push('\n');
    }

    let methods: Vec<_> = ty
        .methods
        .iter()
        .filter(|method| method_visible_in_lang(config, method, &ty.name, lang))
        .collect();
    if !methods.is_empty() {
        let methods_heading = if lang == Language::Elixir {
            "Functions"
        } else {
            "Methods"
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            minijinja::context! { marker => "#####", title => methods_heading },
        ));
        for method in methods {
            out.push_str(&render_method(method, &ty.name, lang, config, ffi_prefix));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeRef};

    #[test]
    fn type_doc_headings_stay_under_type_heading() {
        let ty = TypeDef {
            name: "ReportConfig".to_string(),
            doc: "## Default Behavior\n\nConfiguration notes.".to_string(),
            methods: vec![MethodDef {
                name: "validate".to_string(),
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::Unit,
                ..Default::default()
            }],
            ..Default::default()
        };

        let rendered = render_type(
            &ty,
            Language::Rust,
            &ResolvedCrateConfig::default(),
            &ApiSurface::default(),
            "sample",
        );

        assert!(
            rendered.contains("#### ReportConfig"),
            "type heading should render at h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Default Behavior"),
            "type rustdoc heading should be demoted below h4; got:\n{rendered}"
        );
        assert!(
            rendered.contains("##### Methods"),
            "methods heading should remain at h5; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n###### Default Behavior"),
            "type rustdoc H2 heading must start at h5, not skip to h6; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\n### Default Behavior"),
            "type rustdoc heading must not be promoted above the type heading; got:\n{rendered}"
        );
    }
}
