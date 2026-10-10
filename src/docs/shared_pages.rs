use crate::codegen::shared::binding_fields;
use crate::core::backend::GeneratedFile;
use crate::core::config::{DocsReferenceLinkStyle, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, PrimitiveType, TypeDef, TypeRef};
use std::path::PathBuf;

use super::descriptions::{
    generate_enum_variant_description, generate_error_variant_description, generate_field_description,
};
use super::doc_cleaning::{clean_doc_inline, demote_headings_to_start_at};
use super::enum_payload_fields::variant_field_descriptions;
use super::formatting::{doc_type_with_optional, escape_table_cell, format_field_default};
use super::sorting::is_update_type;
use super::{clean_doc, template_env, version_labels};

/// The link target for a cross-page reference to `page_stem` (e.g. `"configuration"`),
/// honoring `[docs].reference_link_style`.
///
/// This is the single place the docs layer computes a cross-page link -- every generated
/// reference page that links to another generated reference page must call this rather than
/// hand-writing a `.md`-suffixed or extensionless target inline, or the two pages drift the
/// way `types.md`'s link to `configuration.md` once did (hardcoded to the suffixed form,
/// which a content-collection docs site such as Astro Starlight cannot resolve). ~keep
fn reference_page_link(config: &ResolvedCrateConfig, page_stem: &str) -> String {
    let style = config
        .docs
        .as_ref()
        .map(|docs| docs.reference_link_style)
        .unwrap_or_default();
    match style {
        DocsReferenceLinkStyle::Suffixed => format!("{page_stem}.md"),
        DocsReferenceLinkStyle::Extensionless => format!("./{page_stem}/"),
    }
}

pub(super) fn generate_configuration_doc(
    api: &ApiSurface,
    _config: &ResolvedCrateConfig,
    output_dir: &str,
) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Configuration Reference\"\n---\n\n");
    out.push_str("## Configuration Reference\n\n");
    out.push_str("This page documents all configuration types and their defaults across all languages.\n\n");

    let config_types: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            (t.name.ends_with("Config") || t.name.ends_with("Options") || t.name.ends_with("Settings") || t.has_default)
                && !t.is_opaque
                && !is_update_type(&t.name)
        })
        .collect();

    for ty in config_types {
        out.push_str(&template_env::render(
            "heading.jinja",
            crate::alef_context! { marker => "###", title => &ty.name },
        ));
        let doc = clean_doc(&ty.doc, Language::Python);
        // Pin to the level below the `###` heading emitted just above, rather than shifting by a
        // fixed amount: a fixed shift assumes the doc opens at `#`, and any doc that does not
        // lands its headings above their own parent. ~keep
        let doc = demote_headings_to_start_at(&doc, 4);
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        let fields: Vec<_> = binding_fields(&ty.fields).collect();
        if !fields.is_empty() {
            out.push('\n');
            out.push_str("| Field | Type | Default | Description |\n");
            out.push_str("|-------|------|---------|-------------|\n");
            for field in fields {
                let fty = doc_type_with_optional(&field.ty, Language::Python, field.optional, "");
                let fdefault = format_field_default(field, Language::Python, api, "");
                let fdoc = {
                    let raw = clean_doc_inline(&field.doc, Language::Python);
                    if raw.is_empty() {
                        generate_field_description(&field.name, &field.ty)
                    } else {
                        raw
                    }
                };
                out.push_str(&template_env::render(
                    "field_row.jinja",
                    crate::alef_context! {
                        name => escape_table_cell(&field.name),
                        ty => escape_table_cell(&fty),
                        default => escape_table_cell(&fdefault),
                        doc => escape_table_cell(&fdoc),
                    },
                ));
            }
            out.push('\n');
        }

        out.push_str("---\n\n");
    }

    let config_types_for_enum_filter: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            (t.name.ends_with("Config") || t.name.ends_with("Options") || t.name.ends_with("Settings") || t.has_default)
                && !t.is_opaque
                && !is_update_type(&t.name)
        })
        .collect();

    let mut referenced_enums: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|en| {
            config_types_for_enum_filter
                .iter()
                .any(|ty| binding_fields(&ty.fields).any(|field| type_ref_contains_named(&field.ty, &en.name)))
        })
        .collect();
    referenced_enums.sort_by(|a, b| a.name.cmp(&b.name));

    if !referenced_enums.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &referenced_enums {
            out.push_str(&render_enum_for_shared_doc(en, Language::Python, api));
            out.push_str("\n---\n\n");
        }
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/configuration.md")),
        content: out,
        generated_header: false,
    })
}

/// Categorize a type by name/path patterns into a documentation group.
fn categorize_type(ty: &TypeDef) -> &'static str {
    let name = &ty.name;
    if name.ends_with("Result") || name.contains("Result") {
        "Result Types"
    } else if name.contains("Metadata") || name.ends_with("Meta") {
        "Metadata Types"
    } else if name.ends_with("Config") || name.ends_with("Options") || name.ends_with("Settings") || ty.has_default {
        "Configuration Types"
    } else if name.contains("Node") || name.contains("Table") || name.contains("Grid") {
        "Structured Data Types"
    } else {
        "Other Types"
    }
}

pub(super) fn generate_types_doc(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    output_dir: &str,
) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Types Reference\"\n---\n\n");
    out.push_str("## Types Reference\n\n");
    out.push_str("All types defined by the library, grouped by category. Types are shown using Rust as the canonical representation.\n\n");

    let types_to_doc: Vec<&TypeDef> = api.types.iter().filter(|t| !is_update_type(&t.name)).collect();

    if types_to_doc.is_empty() && api.enums.is_empty() {
        out.push_str("No types defined.\n");
        return Ok(GeneratedFile {
            path: PathBuf::from(format!("{output_dir}/types.md")),
            content: out,
            generated_header: false,
        });
    }

    if types_to_doc.is_empty() {
        out.push_str("No struct types defined.\n\n");
    }

    let category_order = [
        "Result Types",
        "Configuration Types",
        "Metadata Types",
        "Structured Data Types",
        "Other Types",
    ];

    let mut groups: std::collections::HashMap<&str, Vec<&TypeDef>> = std::collections::HashMap::new();
    for ty in &types_to_doc {
        let cat = categorize_type(ty);
        groups.entry(cat).or_default().push(ty);
    }

    for &cat in &category_order {
        let Some(types) = groups.get(cat) else {
            continue;
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            crate::alef_context! { marker => "###", title => cat },
        ));

        if cat == "Configuration Types" {
            let target = reference_page_link(config, "configuration");
            out.push_str(&template_env::render(
                "reference_page_link.jinja",
                crate::alef_context! { title => "Configuration Reference", target => target },
            ));
            out.push('\n');
        }

        for ty in types {
            out.push_str(&template_env::render(
                "heading.jinja",
                crate::alef_context! { marker => "####", title => &ty.name },
            ));

            let doc = clean_doc(&ty.doc, Language::Python);
            let doc = demote_headings_to_start_at(&doc, 5);
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push('\n');
                out.push('\n');
            }

            let fields: Vec<_> = binding_fields(&ty.fields).collect();
            if ty.is_opaque {
                out.push_str("*Opaque type — fields are not directly accessible.*\n\n");
            } else if !fields.is_empty() {
                out.push('\n');
                out.push_str("| Field | Type | Default | Description |\n");
                out.push_str("|-------|------|---------|-------------|\n");
                for field in fields {
                    let fty = format_type_ref_rust(&field.ty, field.optional);
                    let fdefault = format_field_default(field, Language::Rust, api, "");
                    let fdoc = {
                        let raw = clean_doc_inline(&field.doc, Language::Rust);
                        if raw.is_empty() {
                            generate_field_description(&field.name, &field.ty)
                        } else {
                            raw
                        }
                    };
                    out.push_str(&template_env::render(
                        "field_row.jinja",
                        crate::alef_context! {
                            name => escape_table_cell(&field.name),
                            ty => escape_table_cell(&fty),
                            default => escape_table_cell(&fdefault),
                            doc => escape_table_cell(&fdoc),
                        },
                    ));
                }
                out.push('\n');
            }

            out.push_str("---\n\n");
        }
    }

    if !api.enums.is_empty() {
        let mut sorted_enums: Vec<&EnumDef> = api.enums.iter().collect();
        sorted_enums.sort_by(|a, b| a.name.cmp(&b.name));

        out.push_str("### Enums\n\n");
        for en in &sorted_enums {
            out.push_str(&render_enum_for_shared_doc(en, Language::Rust, api));
            out.push_str("\n---\n\n");
        }
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/types.md")),
        content: out,
        generated_header: false,
    })
}

/// Render an enum for shared (language-neutral) documentation pages.
///
/// Uses Rust-canonical variant names and type representations, matching the
/// style used by `generate_types_doc` and `generate_configuration_doc`.
pub(super) fn render_enum_for_shared_doc(en: &EnumDef, lang: Language, api: &ApiSurface) -> String {
    let mut out = String::new();

    out.push_str(&template_env::render(
        "heading.jinja",
        crate::alef_context! { marker => "####", title => &en.name },
    ));

    if let Some(ref since) = en.version.since {
        let since = version_labels::major_minor(since);
        out.push_str(&template_env::render(
            "since_badge.jinja",
            crate::alef_context! { since => since },
        ));
        out.push('\n');
        out.push('\n');
    }
    if let Some(ref dep) = en.version.deprecated {
        let since = dep
            .since
            .as_deref()
            .map(version_labels::major_minor)
            .unwrap_or_default();
        out.push_str(&template_env::render(
            "deprecated_notice.jinja",
            crate::alef_context! {
                since => since,
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }

    let doc = clean_doc(&en.doc, lang);
    let doc = demote_headings_to_start_at(&doc, 5);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let has_wire_rename = en.serde_rename_all.is_some() || en.variants.iter().any(|v| v.serde_rename.is_some());

    out.push('\n');
    if has_wire_rename {
        out.push_str("| Variant | Wire value | Description |\n");
        out.push_str("|---------|------------|-------------|\n");
    } else {
        out.push_str("| Variant | Description |\n");
        out.push_str("|---------|-------------|\n");
    }

    for variant in &en.variants {
        let mut vdoc = if !variant.doc.is_empty() {
            clean_doc_inline(&variant.doc, lang)
        } else {
            generate_enum_variant_description(&variant.name)
        };
        let variant_fields: Vec<_> = binding_fields(&variant.fields).collect();
        if !variant_fields.is_empty() {
            let (fields_desc, fields_label) = variant_field_descriptions(
                en,
                variant,
                &variant_fields,
                api,
                |name| name.to_string(),
                |f| format_type_ref_rust(&f.ty, false),
            );
            vdoc = format!("{vdoc} — {fields_label}: {}", fields_desc.join(", "));
        }
        if has_wire_rename {
            let wire = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            out.push_str(&template_env::render(
                "wire_variant_row.jinja",
                crate::alef_context! {
                    name => escape_table_cell(&variant.name),
                    wire => escape_table_cell(&wire),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        } else {
            out.push_str(&template_env::render(
                "variant_row.jinja",
                crate::alef_context! {
                    name => escape_table_cell(&variant.name),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        }
    }

    out
}

/// True if `ty` (or any wrapper layer of it: Option/Vec/Map) names the given type.
fn type_ref_contains_named(ty: &TypeRef, name: &str) -> bool {
    match ty {
        TypeRef::Named(path) => path.rsplit("::").next().unwrap_or(path) == name,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_contains_named(inner, name),
        TypeRef::Map(k, v) => type_ref_contains_named(k, name) || type_ref_contains_named(v, name),
        _ => false,
    }
}

/// Format a TypeRef as a Rust-like canonical type string (language-neutral).
fn format_type_ref_rust(ty: &TypeRef, optional: bool) -> String {
    let base = match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "Duration".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
        },
        TypeRef::Optional(inner) => {
            return format!("Option<{}>", format_type_ref_rust(inner, false));
        }
        TypeRef::Vec(inner) => {
            return format!("Vec<{}>", format_type_ref_rust(inner, false));
        }
        TypeRef::Map(k, v) => {
            return format!(
                "HashMap<{}, {}>",
                format_type_ref_rust(k, false),
                format_type_ref_rust(v, false)
            );
        }
        TypeRef::Named(name) => name.rsplit("::").next().unwrap_or(name).to_string(),
    };
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        format!("Option<{base}>")
    } else {
        base
    }
}

pub(super) fn generate_errors_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Error Reference\"\n---\n\n");
    out.push_str("## Error Reference\n\n");
    out.push_str("All error types thrown by the library across all languages.\n\n");

    for err in &api.errors {
        out.push_str(&template_env::render(
            "heading.jinja",
            crate::alef_context! { marker => "###", title => &err.name },
        ));

        let doc = clean_doc(&err.doc, Language::Python);
        let doc = demote_headings_to_start_at(&doc, 4);
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        out.push('\n');
        out.push_str("| Variant | Message | Description |\n");
        out.push_str("|---------|---------|-------------|\n");
        for variant in &err.variants {
            let tmpl = variant.message_template.as_deref().unwrap_or("");
            let vdoc = if !variant.doc.is_empty() {
                clean_doc_inline(&variant.doc, Language::Python)
            } else {
                generate_error_variant_description(&variant.name)
            };
            out.push_str(&template_env::render(
                "error_message_row.jinja",
                crate::alef_context! {
                    name => escape_table_cell(&variant.name),
                    message => escape_table_cell(tmpl),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        }
        out.push('\n');
        out.push_str("---\n\n");
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/errors.md")),
        content: out,
        generated_header: false,
    })
}
