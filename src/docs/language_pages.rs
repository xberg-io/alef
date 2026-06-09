use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, MethodDef, TypeDef, VersionAnnotation};
use heck::ToPascalCase;
use std::collections::HashSet;
use std::path::PathBuf;

use super::descriptions::{
    generate_enum_variant_description, generate_error_variant_description, generate_field_description,
    generate_param_description,
};
use super::doc_cleaning::{clean_doc_inline, demote_headings, extract_param_docs};
use super::formatting::{doc_type_with_optional, escape_table_cell, format_error_phrase, format_field_default};
use super::naming::{
    enum_variant_name, field_name, func_name, lang_code_fence, lang_display_name, lang_slug, type_name,
};
use super::signatures::{render_function_signature, render_method_signature};
use super::sorting::{is_update_type, type_sort_key};
use super::{clean_doc, doc_type, template_env};

fn language_excludes(config: &ResolvedCrateConfig, lang: Language) -> (HashSet<String>, HashSet<String>) {
    let mut functions: HashSet<String> = config.exclude.functions.iter().cloned().collect();
    let mut types: HashSet<String> = config.exclude.types.iter().cloned().collect();

    match lang {
        Language::Python => {
            if let Some(c) = &config.python {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Node => {
            if let Some(c) = &config.node {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Ruby => {
            if let Some(c) = &config.ruby {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Php => {
            if let Some(c) = &config.php {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Elixir => {
            if let Some(c) = &config.elixir {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Wasm => {
            if let Some(c) = &config.wasm {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Ffi | Language::C => {
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Go => {
            if let Some(c) = &config.go {
                types.extend(c.exclude_types.iter().cloned());
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Java => {
            if let Some(c) = &config.java {
                types.extend(c.exclude_types.iter().cloned());
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Kotlin => {
            if let Some(c) = &config.kotlin {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::KotlinAndroid => {
            if let Some(c) = &config.kotlin_android {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Jni => {
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Swift => {
            if let Some(c) = &config.swift {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Dart => {
            if let Some(c) = &config.dart {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Gleam => {
            if let Some(c) = &config.gleam {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Csharp => {
            if let Some(c) = &config.csharp {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Zig => {
            if let Some(c) = &config.zig {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::R | Language::Rust => {}
    }

    (functions, types)
}

fn extend_excludes(
    functions: &mut HashSet<String>,
    types: &mut HashSet<String>,
    exclude_functions: &[String],
    exclude_types: &[String],
) {
    functions.extend(exclude_functions.iter().cloned());
    types.extend(exclude_types.iter().cloned());
}

// ---------------------------------------------------------------------------
// Per-language doc page
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Per-language doc page
// ---------------------------------------------------------------------------

pub(super) fn generate_lang_doc(
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
    let (exclude_functions, exclude_types) = language_excludes(config, lang);

    out.push_str(&template_env::render(
        "front_matter.jinja",
        minijinja::context! { title => format!("{lang_display} API Reference") },
    ));
    // MD071: blank line required between frontmatter and first heading.
    out.push('\n');
    out.push_str(&template_env::render(
        "version_heading.jinja",
        minijinja::context! { marker => "##", title => format!("{lang_display} API Reference"), version => version },
    ));

    // --- Functions section ---
    let public_fns: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name))
        .collect();
    if !public_fns.is_empty() {
        out.push_str("### Functions\n\n");
        for func in &public_fns {
            out.push_str(&render_function(func, lang, config, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Types section ---
    // Order: ParseOptions, ParseOutput, then rest alphabetical
    // Skip opaque types and *Update types in main section
    let mut types_to_doc: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| !is_update_type(&t.name) && !exclude_types.contains(&t.name))
        .collect();

    // Sort: ParseOptions first, ParseOutput second, rest alphabetical
    types_to_doc.sort_by(|a, b| type_sort_key(&a.name).cmp(&type_sort_key(&b.name)));

    if !types_to_doc.is_empty() {
        out.push_str("### Types\n\n");
        for ty in &types_to_doc {
            out.push_str(&render_type(ty, lang, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    // --- Enums section ---
    let enums_to_doc: Vec<&EnumDef> = api.enums.iter().filter(|e| !exclude_types.contains(&e.name)).collect();
    if !enums_to_doc.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &enums_to_doc {
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
// Version annotation rendering
// ---------------------------------------------------------------------------

fn push_version_annotation(out: &mut String, version: &VersionAnnotation) {
    if let Some(ref since) = version.since {
        out.push_str(&template_env::render(
            "since_badge.jinja",
            minijinja::context! { since => since },
        ));
        out.push('\n');
        out.push('\n');
    }
    if let Some(ref dep) = version.deprecated {
        out.push_str(&template_env::render(
            "deprecated_notice.jinja",
            minijinja::context! {
                since => dep.since.as_deref().unwrap_or(""),
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }
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

    push_version_annotation(&mut out, &func.version);

    // Extract parameter descriptions from the RAW doc string BEFORE cleaning
    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        let doc = clean_doc(&func.doc, lang);
        // Demote any embedded headings in the function documentation by 2 levels
        // to ensure they stay nested under the function heading (####).
        let doc = demote_headings(&doc, 2);
        out.push_str(&doc);
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
    // MD031: blank line required after fenced code block.
    out.push('\n');

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
                    s.replace("ParseOptions.default()", "default options")
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
        minijinja::context! { marker => "####", title => format!("{mname}()") },
    ));

    push_version_annotation(&mut out, &method.version);

    let doc = clean_doc(&method.doc, lang);
    // Demote any embedded headings in the method documentation by 2 levels
    // to ensure they stay nested under the method heading (####).
    let doc = demote_headings(&doc, 2);
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
    // MD031: blank line required after fenced code block.
    out.push('\n');

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

    push_version_annotation(&mut out, &ty.version);

    let doc = clean_doc(&ty.doc, lang);
    // Demote any embedded headings in the type documentation by 2 levels
    // to ensure they stay nested under the type heading (####).
    let doc = demote_headings(&doc, 2);
    if !doc.is_empty() {
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    // Fields table (only for non-opaque types or opaque types with documented fields)
    if !ty.is_opaque && !ty.fields.is_empty() {
        out.push('\n');
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
            minijinja::context! { marker => "###", title => methods_heading },
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

    push_version_annotation(&mut out, &en.version);

    let doc = clean_doc(&en.doc, lang);
    // Demote any embedded headings in the enum documentation by 2 levels
    // to ensure they stay nested under the enum heading (####).
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
        // Inline version annotations into the description cell (block-level elements
        // cannot appear inside a Markdown table row).
        if let Some(ref since) = variant.version.since {
            vdoc = format!("{vdoc} — **Since:** `v{since}`");
        }
        if let Some(ref dep) = variant.version.deprecated {
            let dep_note = match (&dep.since, &dep.note) {
                (Some(s), Some(n)) => format!("Deprecated since `v{s}`: {n}"),
                (Some(s), None) => format!("Deprecated since `v{s}`"),
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
    // Demote any embedded headings in the error documentation by 2 levels
    // to ensure they stay nested under the error heading (####).
    let doc = demote_headings(&doc, 2);
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

// ---------------------------------------------------------------------------
// Configuration page
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Configuration page
