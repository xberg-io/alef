use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, PrimitiveType, TypeDef, TypeRef};
use std::path::PathBuf;

use super::descriptions::{
    generate_enum_variant_description, generate_error_variant_description, generate_field_description,
};
use super::doc_cleaning::{clean_doc_inline, demote_headings};
use super::formatting::{doc_type_with_optional, escape_table_cell, format_field_default};
use super::naming::to_camel_case;
use super::sorting::is_update_type;
use super::{clean_doc, template_env, version_labels};

pub(super) fn generate_configuration_doc(
    api: &ApiSurface,
    _config: &ResolvedCrateConfig,
    output_dir: &str,
) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Configuration Reference\"\n---\n\n");
    out.push_str("## Configuration Reference\n\n");
    out.push_str("This page documents all configuration types and their defaults across all languages.\n\n");

    // Collect config-like types (Config, Options, Settings suffixes, or types with Default)
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
            minijinja::context! { marker => "###", title => &ty.name },
        ));
        let doc = clean_doc(&ty.doc, Language::Python);
        // Demote any embedded headings in the type documentation by 1 level
        // to ensure they stay nested under the type heading (###).
        let doc = demote_headings(&doc, 1);
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        if !ty.fields.is_empty() {
            out.push('\n');
            out.push_str("| Field | Type | Default | Description |\n");
            out.push_str("|-------|------|---------|-------------|\n");
            for field in &ty.fields {
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
                    minijinja::context! {
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

    // --- Enums referenced by config-type fields ---
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
            config_types_for_enum_filter.iter().any(|ty| {
                ty.fields
                    .iter()
                    .any(|field| type_ref_contains_named(&field.ty, &en.name))
            })
        })
        .collect();
    referenced_enums.sort_by(|a, b| a.name.cmp(&b.name));

    if !referenced_enums.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &referenced_enums {
            out.push_str(&render_enum_for_shared_doc(en));
            out.push_str("\n---\n\n");
        }
    }

    Ok(GeneratedFile {
        path: PathBuf::from(format!("{output_dir}/configuration.md")),
        content: out,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Types reference page
// ---------------------------------------------------------------------------

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

pub(super) fn generate_types_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Types Reference\"\n---\n\n");
    out.push_str("## Types Reference\n\n");
    out.push_str("All types defined by the library, grouped by category. Types are shown using Rust as the canonical representation.\n\n");

    // Collect non-update types
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

    // Define category order
    let category_order = [
        "Result Types",
        "Configuration Types",
        "Metadata Types",
        "Structured Data Types",
        "Other Types",
    ];

    // Group types by category
    let mut groups: std::collections::HashMap<&str, Vec<&TypeDef>> = std::collections::HashMap::new();
    for ty in &types_to_doc {
        let cat = categorize_type(ty);
        groups.entry(cat).or_default().push(ty);
    }

    // Render each category in order
    for &cat in &category_order {
        let Some(types) = groups.get(cat) else {
            continue;
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            minijinja::context! { marker => "###", title => cat },
        ));

        if cat == "Configuration Types" {
            out.push_str("See [Configuration Reference](configuration.md) for detailed defaults and language-specific representations.\n\n");
        }

        for ty in types {
            out.push_str(&template_env::render(
                "heading.jinja",
                minijinja::context! { marker => "####", title => &ty.name },
            ));

            let doc = clean_doc(&ty.doc, Language::Python);
            // Demote any embedded headings in the type documentation by 2 levels
            // to ensure they stay nested under the type heading (####).
            let doc = demote_headings(&doc, 2);
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push('\n');
                out.push('\n');
            }

            if ty.is_opaque {
                out.push_str("*Opaque type — fields are not directly accessible.*\n\n");
            } else if !ty.fields.is_empty() {
                out.push('\n');
                out.push_str("| Field | Type | Default | Description |\n");
                out.push_str("|-------|------|---------|-------------|\n");
                for field in &ty.fields {
                    // Use Rust-style type representation as canonical
                    let fty = format_type_ref_rust(&field.ty, field.optional);
                    // Use the typed default (consistent with per-language pages)
                    // falling back to the raw string default.
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
                        minijinja::context! {
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

    // --- Enums section ---
    if !api.enums.is_empty() {
        let mut sorted_enums: Vec<&EnumDef> = api.enums.iter().collect();
        sorted_enums.sort_by(|a, b| a.name.cmp(&b.name));

        out.push_str("### Enums\n\n");
        for en in &sorted_enums {
            out.push_str(&render_enum_for_shared_doc(en));
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
pub(super) fn render_enum_for_shared_doc(en: &EnumDef) -> String {
    let mut out = String::new();

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => &en.name },
    ));

    // Version annotation
    if let Some(ref since) = en.version.since {
        let since = version_labels::major_minor(since);
        out.push_str(&template_env::render(
            "since_badge.jinja",
            minijinja::context! { since => since },
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
            minijinja::context! {
                since => since,
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }

    let doc = clean_doc(&en.doc, Language::Rust);
    // Demote any embedded headings in the enum documentation by 2 levels
    // to ensure they stay nested under the enum heading (####).
    let doc = demote_headings(&doc, 2);
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
            clean_doc_inline(&variant.doc, Language::Rust)
        } else {
            generate_enum_variant_description(&variant.name)
        };
        if !variant.fields.is_empty() {
            let fields_desc: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let fty = format_type_ref_rust(&f.ty, false);
                    format!("`{}`: `{}`", f.name, fty)
                })
                .collect();
            vdoc = format!("{vdoc} — Fields: {}", fields_desc.join(", "));
        }
        if has_wire_rename {
            let wire = wire_variant_value(
                &variant.name,
                en.serde_rename_all.as_deref(),
                variant.serde_rename.as_deref(),
            );
            out.push_str(&template_env::render(
                "wire_variant_row.jinja",
                minijinja::context! {
                    name => escape_table_cell(&variant.name),
                    wire => escape_table_cell(&wire),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        } else {
            out.push_str(&template_env::render(
                "variant_row.jinja",
                minijinja::context! {
                    name => escape_table_cell(&variant.name),
                    doc => escape_table_cell(&vdoc),
                },
            ));
        }
    }

    out
}

/// Compute the JSON/TOML wire value for an enum variant, applying
/// `#[serde(rename = "...")]` first and then `#[serde(rename_all = "...")]`
/// to the variant's PascalCase name. Falls back to the variant name verbatim
/// when neither attribute applies.
fn wire_variant_value(name: &str, rename_all: Option<&str>, explicit_rename: Option<&str>) -> String {
    if let Some(r) = explicit_rename {
        return r.to_string();
    }
    use heck::{ToKebabCase, ToShoutyKebabCase, ToShoutySnakeCase, ToSnakeCase};
    match rename_all {
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        Some("snake_case") => name.to_snake_case(),
        Some("SCREAMING_SNAKE_CASE") => name.to_shouty_snake_case(),
        Some("kebab-case") => name.to_kebab_case(),
        Some("SCREAMING-KEBAB-CASE") => name.to_shouty_kebab_case(),
        Some("camelCase") => to_camel_case(name),
        Some("PascalCase") | None => name.to_string(),
        Some(_) => name.to_string(),
    }
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

// ---------------------------------------------------------------------------
// Errors page
// ---------------------------------------------------------------------------

pub(super) fn generate_errors_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
    // ---------------------------------------------------------------------------
    // Errors reference page
    // ---------------------------------------------------------------------------

    let mut out = String::with_capacity(8192);

    out.push_str("---\ntitle: \"Error Reference\"\n---\n\n");
    out.push_str("## Error Reference\n\n");
    out.push_str("All error types thrown by the library across all languages.\n\n");

    for err in &api.errors {
        out.push_str(&template_env::render(
            "heading.jinja",
            minijinja::context! { marker => "###", title => &err.name },
        ));

        let doc = clean_doc(&err.doc, Language::Python);
        // Demote any embedded headings in the error documentation by 1 level
        // to ensure they stay nested under the error heading (###).
        let doc = demote_headings(&doc, 1);
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
                minijinja::context! {
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

// ---------------------------------------------------------------------------
// Type mapping
