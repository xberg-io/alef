//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use alef_core::backend::GeneratedFile;
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToPascalCase;
use std::path::PathBuf;

// Module declarations
mod descriptions;
pub mod doc_cleaning;
mod formatting;
mod naming;
mod signatures;
mod sorting;
mod template_env;
mod type_mapping;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use doc_cleaning::clean_doc;
pub use type_mapping::doc_type;

use descriptions::{
    generate_enum_variant_description, generate_error_variant_description, generate_field_description,
    generate_param_description,
};
use doc_cleaning::{clean_doc_inline, extract_param_docs, wrap_bare_urls};
use formatting::{doc_type_with_optional, escape_table_cell, format_error_phrase, format_field_default};
use naming::{
    enum_variant_name, field_name, func_name, lang_code_fence, lang_display_name, lang_slug, to_camel_case, type_name,
};
use signatures::{render_function_signature, render_method_signature};
use sorting::{is_update_type, type_sort_key};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate API reference documentation for the given languages.
///
/// Produces one `api-{lang}.md` per language, plus shared `configuration.md`,
/// `types.md`, and `errors.md` files written into `output_dir`.
pub fn generate_docs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_dir: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();
    let ffi_prefix = &config.ffi_prefix().to_pascal_case();

    for &lang in languages {
        files.push(generate_lang_doc(api, config, lang, output_dir, ffi_prefix)?);
    }

    files.push(generate_configuration_doc(api, config, output_dir)?);
    files.push(generate_types_doc(api, output_dir)?);
    files.push(generate_errors_doc(api, output_dir)?);

    // Post-process: ensure trailing newline and wrap bare URLs (MD034)
    for file in &mut files {
        // Wrap bare http(s) URLs in angle brackets to satisfy MD034
        file.content = wrap_bare_urls(&file.content);
        // Ensure POSIX trailing newline
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

// ---------------------------------------------------------------------------
// Per-language doc page
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Per-language doc page
// ---------------------------------------------------------------------------

fn generate_lang_doc(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
    output_dir: &str,
    ffi_prefix: &str,
) -> anyhow::Result<GeneratedFile> {
    let lang_display = lang_display_name(lang);
    let version = &api.version;
    let lang_slug = lang_slug(lang);

    let mut out = String::with_capacity(8192);

    out.push_str(&template_env::render(
        "front_matter.jinja",
        minijinja::context! { title => format!("{lang_display} API Reference") },
    ));
    out.push_str(&template_env::render(
        "version_heading.jinja",
        minijinja::context! { marker => "##", title => format!("{lang_display} API Reference"), version => version },
    ));

    // --- Functions section ---
    let public_fns: Vec<&FunctionDef> = api.functions.iter().collect();
    if !public_fns.is_empty() {
        out.push_str("### Functions\n\n");
        for func in &public_fns {
            out.push_str(&render_function(func, lang, config, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Types section ---
    // Order: ConversionOptions, ConversionResult, then rest alphabetical
    // Skip opaque types and *Update types in main section
    let mut types_to_doc: Vec<&TypeDef> = api.types.iter().filter(|t| !is_update_type(&t.name)).collect();

    // Sort: ConversionOptions first, ConversionResult second, rest alphabetical
    types_to_doc.sort_by(|a, b| type_sort_key(&a.name).cmp(&type_sort_key(&b.name)));

    if !types_to_doc.is_empty() {
        out.push_str("### Types\n\n");
        for ty in &types_to_doc {
            out.push_str(&render_type(ty, lang, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Enums section ---
    if !api.enums.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &api.enums {
            out.push_str(&render_enum(en, lang, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Errors section ---
    if !api.errors.is_empty() {
        out.push_str("### Errors\n\n");
        for err in &api.errors {
            out.push_str(&render_error(err, lang, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    let path = PathBuf::from(format!("{output_dir}/api-{lang_slug}.md"));

    Ok(GeneratedFile {
        path,
        content: out,
        generated_header: false,
    })
}

// ---------------------------------------------------------------------------
// Function rendering
// ---------------------------------------------------------------------------

fn render_function(
    func: &FunctionDef,
    lang: Language,
    _config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let fn_name = func_name(&func.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => format!("{fn_name}()") },
    ));

    // Extract parameter descriptions from the RAW doc string BEFORE cleaning
    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        out.push_str(&clean_doc(&func.doc, lang));
        out.push('\n');
        out.push('\n');
    }

    // Signature
    out.push_str("**Signature:**\n\n");
    let lang_code = lang_code_fence(lang);
    let sig = render_function_signature(func, lang, ffi_prefix);
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));

    // Parameters table
    if !func.params.is_empty() {
        out.push_str("**Parameters:**\n\n");
        out.push_str("| Name | Type | Required | Description |\n");
        out.push_str("|------|------|----------|-------------|\n");
        for param in &func.params {
            let pname = field_name(&param.name, lang);
            let pty = doc_type_with_optional(&param.ty, lang, param.optional, ffi_prefix);
            let required = if param.optional { "No" } else { "Yes" };
            let pdoc = param_docs
                .get(param.name.as_str())
                .map(|s| {
                    // Clean Rust syntax from param descriptions
                    let s = s.replace("::", ".");
                    s.replace("ConversionOptions.default()", "default options")
                })
                .unwrap_or_else(|| generate_param_description(&param.name, &param.ty));
            out.push_str(&template_env::render(
                "param_row.jinja",
                minijinja::context! {
                    name => escape_table_cell(&pname),
                    ty => escape_table_cell(&pty),
                    required => required,
                    doc => escape_table_cell(&pdoc),
                },
            ));
        }
        out.push('\n');
    }

    // Return type
    let ret_ty = doc_type(&func.return_type, lang, ffi_prefix);
    out.push_str(&template_env::render(
        "returns.jinja",
        minijinja::context! { ty => ret_ty },
    ));

    // Errors
    if let Some(err) = &func.error_type {
        let error_phrase = format_error_phrase(err, lang);
        out.push_str(&template_env::render(
            "errors_phrase.jinja",
            minijinja::context! { phrase => error_phrase },
        ));
    }

    let _ = api; // api is available for future use in function rendering
    out
}

fn render_method(method: &MethodDef, type_name_str: &str, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let mname = func_name(&method.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "######", title => format!("{mname}()") },
    ));

    let doc = clean_doc(&method.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let lang_code = lang_code_fence(lang);
    let sig = render_method_signature(method, type_name_str, lang, ffi_prefix);
    out.push_str("**Signature:**\n\n");
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));

    out
}

// ---------------------------------------------------------------------------
// Type rendering
// ---------------------------------------------------------------------------

fn render_type(ty: &TypeDef, lang: Language, api: &ApiSurface, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let tname = type_name(&ty.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => tname },
    ));

    let doc = clean_doc(&ty.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    // Fields table (only for non-opaque types or opaque types with documented fields)
    if !ty.is_opaque && !ty.fields.is_empty() {
        out.push_str("| Field | Type | Default | Description |\n");
        out.push_str("|-------|------|---------|-------------|\n");
        for field in &ty.fields {
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

    // Methods (called "Functions" in Elixir)
    if !ty.methods.is_empty() {
        let methods_heading = if lang == Language::Elixir {
            "Functions"
        } else {
            "Methods"
        };
        out.push_str(&template_env::render(
            "heading.jinja",
            minijinja::context! { marker => "#####", title => methods_heading },
        ));
        for method in &ty.methods {
            out.push_str(&render_method(method, &ty.name, lang, ffi_prefix));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Enum rendering
// ---------------------------------------------------------------------------

fn render_enum(en: &EnumDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&en.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => ename },
    ));

    let doc = clean_doc(&en.doc, lang);
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
        // Append field info for data variants
        if !variant.fields.is_empty() {
            let fields_desc: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let fname = field_name(&f.name, lang);
                    let fty = doc_type(&f.ty, lang, ffi_prefix);
                    format!("`{fname}`: `{fty}`")
                })
                .collect();
            vdoc = format!("{vdoc} — Fields: {}", fields_desc.join(", "));
        }
        out.push_str(&template_env::render(
            "variant_row.jinja",
            minijinja::context! { name => escape_table_cell(&vname), doc => escape_table_cell(&vdoc) },
        ));
    }
    out.push('\n');

    out
}

// ---------------------------------------------------------------------------
// Error rendering
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Error rendering
// ---------------------------------------------------------------------------

fn render_error(err: &ErrorDef, lang: Language, ffi_prefix: &str) -> String {
    let mut out = String::new();
    let ename = type_name(&err.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => &ename },
    ));

    let doc = clean_doc(&err.doc, lang);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    // For Node/WASM, note that errors are plain Error objects
    if matches!(lang, Language::Node | Language::Wasm) {
        out.push_str("Errors are thrown as plain `Error` objects with descriptive messages.\n\n");
    }

    // For Python, render as exception class hierarchy
    if lang == Language::Python {
        out.push_str(&template_env::render(
            "base_class.jinja",
            minijinja::context! { name => &ename },
        ));
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

// ---------------------------------------------------------------------------
// Configuration page
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Configuration page
// ---------------------------------------------------------------------------

fn generate_configuration_doc(
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
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

        if !ty.fields.is_empty() {
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
    } else if name.contains("Node") || name.contains("Table") || name.contains("Grid") || name.contains("Document") {
        "Document Structure"
    } else if name.contains("Ocr") || name.contains("Tesseract") || name.contains("Paddle") {
        "OCR Types"
    } else {
        "Other Types"
    }
}

fn generate_types_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
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
        "Document Structure",
        "OCR Types",
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
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push('\n');
                out.push('\n');
            }

            if ty.is_opaque {
                out.push_str("*Opaque type — fields are not directly accessible.*\n\n");
            } else if !ty.fields.is_empty() {
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
fn render_enum_for_shared_doc(en: &EnumDef) -> String {
    let mut out = String::new();

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => &en.name },
    ));

    let doc = clean_doc(&en.doc, Language::Rust);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    let has_wire_rename = en.serde_rename_all.is_some() || en.variants.iter().any(|v| v.serde_rename.is_some());

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

fn generate_errors_doc(api: &ApiSurface, output_dir: &str) -> anyhow::Result<GeneratedFile> {
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
        if !doc.is_empty() {
            out.push_str(&doc);
            out.push('\n');
            out.push('\n');
        }

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
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{make_minimal_api, make_param, make_test_config};

    #[test]
    fn test_generate_docs_empty_api() {
        let api = ApiSurface {
            crate_name: "test".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();

        let files = generate_docs(&api, &config, &[Language::Python], "docs").unwrap();
        // 1 lang + configuration.md + types.md + errors.md
        assert_eq!(files.len(), 4);
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("Python API Reference"));
        assert!(lang_file.content.contains("v0.1.0"));
    }

    #[test]
    fn test_generate_docs_produces_one_file_per_language_plus_three_shared() {
        let api = make_minimal_api("1.2.3");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python, Language::Node], "out").unwrap();
        // 2 language files + configuration.md + types.md + errors.md
        assert_eq!(files.len(), 5);
        let paths: Vec<&str> = files.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert!(paths.iter().any(|p| p.contains("api-python")));
        assert!(paths.iter().any(|p| p.contains("api-typescript")));
        assert!(paths.iter().any(|p| p.contains("configuration")));
        assert!(paths.iter().any(|p| p.contains("types")));
        assert!(paths.iter().any(|p| p.contains("errors")));
    }

    #[test]
    fn test_generate_docs_all_output_files_end_with_newline() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        for file in &files {
            assert!(
                file.content.ends_with('\n'),
                "file {:?} must end with trailing newline",
                file.path
            );
        }
    }

    #[test]
    fn test_generate_docs_output_dir_prefix_in_all_paths() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "custom/output/dir").unwrap();
        for file in &files {
            assert!(
                file.path.to_str().unwrap().starts_with("custom/output/dir"),
                "all paths must be under output_dir: {:?}",
                file.path
            );
        }
    }

    #[test]
    fn test_generate_docs_with_function_renders_signature_and_params() {
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "convert_html".to_string(),
                rust_path: "mylib::convert_html".to_string(),
                original_rust_path: String::new(),
                params: vec![make_param("html", TypeRef::String, false)],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Converts HTML to plain text.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("convert_html()"));
        assert!(lang_file.content.contains("Converts HTML to plain text."));
        assert!(lang_file.content.contains("**Signature:**"));
        assert!(lang_file.content.contains("**Parameters:**"));
    }

    #[test]
    fn test_generate_docs_with_enum_renders_python_screaming_case_variants() {
        use alef_core::ir::EnumVariant;
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![EnumDef {
                name: "OutputFormat".to_string(),
                rust_path: "mylib::OutputFormat".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Markdown".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: "Markdown output.".to_string(),
                        is_default: true,
                        serde_rename: None,
                    },
                    EnumVariant {
                        name: "Plain".to_string(),
                        fields: vec![],
                        is_tuple: false,
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                ],
                doc: "The output format.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("OutputFormat"));
        assert!(
            lang_file.content.contains("MARKDOWN"),
            "Python variant must be SCREAMING_SNAKE"
        );
        assert!(lang_file.content.contains("PLAIN"));
    }

    #[test]
    fn test_generate_types_doc_renders_enum_variants() {
        use alef_core::ir::EnumVariant;
        let api = ApiSurface {
            crate_name: "test".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![EnumDef {
                name: "TableModel".into(),
                rust_path: "test::TableModel".into(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Tatr".into(),
                        fields: vec![],
                        doc: "TATR transformer (default).".into(),
                        is_default: true,
                        serde_rename: None,
                        is_tuple: false,
                    },
                    EnumVariant {
                        name: "SlanetWired".into(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        is_tuple: false,
                    },
                ],
                doc: "Table structure model.".into(),
                cfg: None,
                is_copy: true,
                has_serde: true,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let types_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("types"))
            .unwrap();
        assert!(types_file.content.contains("### Enums"));
        assert!(types_file.content.contains("#### TableModel"));
        assert!(types_file.content.contains("Table structure model."));
        assert!(types_file.content.contains("`Tatr`"));
        assert!(types_file.content.contains("TATR transformer"));
        assert!(types_file.content.contains("`SlanetWired`"));
    }

    #[test]
    fn test_render_enum_for_shared_doc_emits_wire_value_column_when_rename_all_set() {
        use alef_core::ir::EnumVariant;
        let en = EnumDef {
            name: "HtmlTheme".into(),
            rust_path: "test::HtmlTheme".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Default".into(),
                    fields: vec![],
                    doc: "Default theme.".into(),
                    is_default: true,
                    serde_rename: None,
                    is_tuple: false,
                },
                EnumVariant {
                    name: "Github".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    is_tuple: false,
                },
            ],
            doc: "HTML theme.".into(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: Some("lowercase".into()),
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let out = render_enum_for_shared_doc(&en);
        assert!(out.contains("| Variant | Wire value | Description |"));
        assert!(out.contains("| `Default` | `default` |"));
        assert!(out.contains("| `Github` | `github` |"));
    }

    #[test]
    fn test_generate_configuration_doc_renders_referenced_enums_only() {
        use alef_core::ir::{CoreWrapper, EnumVariant, FieldDef};
        let api = ApiSurface {
            crate_name: "mylib".into(),
            version: "0.1.0".into(),
            types: vec![TypeDef {
                name: "ImageConfig".into(),
                rust_path: "mylib::ImageConfig".into(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "format".into(),
                    ty: TypeRef::Named("mylib::ImageFormat".into()),
                    optional: false,
                    default: None,
                    doc: "Output image format.".into(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Image config.".into(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            functions: vec![],
            enums: vec![
                EnumDef {
                    name: "ImageFormat".into(),
                    rust_path: "mylib::ImageFormat".into(),
                    original_rust_path: String::new(),
                    variants: vec![EnumVariant {
                        name: "Png".into(),
                        fields: vec![],
                        doc: "PNG output.".into(),
                        is_default: true,
                        serde_rename: None,
                        is_tuple: false,
                    }],
                    doc: "Image format enum.".into(),
                    cfg: None,
                    is_copy: true,
                    has_serde: true,
                    serde_tag: None,
                    serde_untagged: false,
                    serde_rename_all: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
                EnumDef {
                    name: "Unrelated".into(),
                    rust_path: "mylib::Unrelated".into(),
                    original_rust_path: String::new(),
                    variants: vec![EnumVariant {
                        name: "A".into(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                        is_tuple: false,
                    }],
                    doc: "Not referenced by any config type.".into(),
                    cfg: None,
                    is_copy: true,
                    has_serde: true,
                    serde_tag: None,
                    serde_untagged: false,
                    serde_rename_all: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                },
            ],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let cfg_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("configuration"))
            .unwrap();
        assert!(cfg_file.content.contains("### Enums"));
        assert!(cfg_file.content.contains("#### ImageFormat"));
        assert!(
            !cfg_file.content.contains("#### Unrelated"),
            "configuration.md must filter out enums not referenced by any config-type field"
        );
    }

    #[test]
    fn test_generate_docs_with_type_renders_fields_and_doc() {
        use alef_core::ir::{CoreWrapper, FieldDef};
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "ConversionOptions".to_string(),
                rust_path: "mylib::ConversionOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "max_length".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    optional: true,
                    default: None,
                    doc: "Maximum output length.".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Options for the conversion.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("ConversionOptions"));
        assert!(lang_file.content.contains("max_length"));
        assert!(lang_file.content.contains("Maximum output length."));
    }

    #[test]
    fn test_generate_docs_with_error_appears_in_lang_page_and_errors_md() {
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![ErrorDef {
                name: "ConversionError".to_string(),
                rust_path: "mylib::ConversionError".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    alef_core::ir::ErrorVariant {
                        name: "InvalidInput".to_string(),
                        message_template: Some("Invalid input: {0}".to_string()),
                        fields: vec![],
                        has_source: false,
                        has_from: false,
                        is_unit: false,
                        doc: String::new(),
                    },
                    alef_core::ir::ErrorVariant {
                        name: "IoError".to_string(),
                        message_template: None,
                        fields: vec![],
                        has_source: false,
                        has_from: false,
                        is_unit: true,
                        doc: "An I/O error occurred.".to_string(),
                    },
                ],
                doc: "Errors from the conversion API.".to_string(),
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(lang_file.content.contains("ConversionError"));
        assert!(lang_file.content.contains("InvalidInput"));
        assert!(lang_file.content.contains("IoError"));

        let errors_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("errors"))
            .unwrap();
        assert!(errors_file.content.contains("ConversionError"));
        assert!(errors_file.content.contains("Invalid input: {0}"));
    }

    #[test]
    fn test_generate_docs_multiple_languages_produce_correct_slugs() {
        let api = make_minimal_api("0.1.0");
        let config = make_test_config();
        let langs = [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Ruby,
        ];
        let expected_slugs = ["api-python", "api-typescript", "api-go", "api-java", "api-ruby"];
        let files = generate_docs(&api, &config, &langs, "docs/api").unwrap();
        // 5 lang files + 3 shared
        assert_eq!(files.len(), 8);
        for slug in &expected_slugs {
            assert!(
                files.iter().any(|f| f.path.to_str().unwrap().contains(slug)),
                "expected file with slug {slug}"
            );
        }
    }

    /// Count user-facing cells in a markdown table row.
    ///
    /// GFM strips the leading and trailing `|` if present, then splits on
    /// unescaped `|`. We treat `\|` as a single escaped pipe (a literal `|`
    /// inside a cell), matching how rumdl's MD056 check parses cell counts.
    fn count_md_table_cells(row: &str) -> usize {
        // Strip leading/trailing pipe and any surrounding whitespace.
        let trimmed = row.trim();
        let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
        let inner = inner.strip_suffix('|').unwrap_or(inner);

        let mut cells = 1usize;
        let mut prev_backslash = false;
        for ch in inner.chars() {
            if ch == '|' && !prev_backslash {
                cells += 1;
            }
            prev_backslash = ch == '\\' && !prev_backslash;
        }
        cells
    }

    /// Verify every markdown table in `content` has consistent cell counts
    /// across the header, separator, and every data row.
    ///
    /// This guards against MD056 (table-pipe-style / table-column-count)
    /// violations: rows that emit more or fewer cells than the header.
    fn assert_no_md056_violations(content: &str) {
        let mut header_cells: Option<usize> = None;
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            let is_table_row = trimmed.starts_with('|');
            if !is_table_row {
                header_cells = None;
                continue;
            }
            let cells = count_md_table_cells(line);
            match header_cells {
                None => header_cells = Some(cells),
                Some(expected) => {
                    assert_eq!(
                        cells,
                        expected,
                        "table row {} has {} cells, expected {} (MD056 violation):\n  {}",
                        idx + 1,
                        cells,
                        expected,
                        line,
                    );
                }
            }
        }
    }

    #[test]
    fn test_count_md_table_cells_treats_escaped_pipes_as_literal() {
        // Sanity check the helper used by the MD056 regression test.
        assert_eq!(count_md_table_cells("| a | b | c |"), 3);
        assert_eq!(count_md_table_cells("|---|---|---|"), 3);
        // `\|` is a literal pipe inside one cell, so this row is still 3 cells.
        assert_eq!(count_md_table_cells("| `string \\| null` | `null` | desc |"), 3);
        // An unescaped pipe inside a cell does split — that's the bug we're guarding against.
        assert_eq!(count_md_table_cells("| `string | null` | `null` | desc |"), 4);
    }

    #[test]
    fn test_generate_docs_typescript_optional_field_emits_consistent_table_cells() {
        // Regression test for MD056: union types like `string | null` (TypeScript)
        // or `String.t() | nil` (Elixir) contain a literal `|` which, if not
        // escaped, splits a 4-column table row into 5 cells. The generator must
        // escape pipes in every cell value.
        use alef_core::ir::{CoreWrapper, FieldDef};
        let api = ApiSurface {
            crate_name: "mylib".into(),
            version: "0.1.0".into(),
            types: vec![TypeDef {
                name: "InitOptions".into(),
                rust_path: "mylib::InitOptions".into(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "cache_dir".into(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    doc: "Override default cache directory.".into(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Init options.".into(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(
            &api,
            &config,
            &[Language::Node, Language::Python, Language::Elixir],
            "docs",
        )
        .unwrap();

        for file in &files {
            assert_no_md056_violations(&file.content);
        }

        // Specifically confirm the TS page has an escaped pipe in the type column,
        // not a stray extra cell.
        let ts_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-typescript"))
            .unwrap();
        assert!(
            ts_file.content.contains("`string \\| null`"),
            "expected pipe inside union type to be escaped, got: {}",
            ts_file.content,
        );
    }

    #[test]
    fn test_generate_docs_post_processing_wraps_bare_urls() {
        // A bare URL in a function doc string must be angle-bracket-wrapped in output
        let api = ApiSurface {
            crate_name: "mylib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "fetch".to_string(),
                rust_path: "mylib::fetch".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Fetches from https://example.com directly.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let config = make_test_config();
        let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
        let lang_file = files
            .iter()
            .find(|f| f.path.to_str().unwrap().contains("api-python"))
            .unwrap();
        assert!(
            lang_file.content.contains("<https://example.com>"),
            "bare URL must be wrapped by post-processing: {}",
            lang_file.content
        );
    }
}
