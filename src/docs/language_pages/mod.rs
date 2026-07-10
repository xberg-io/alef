mod enum_render;
mod error_render;
mod excludes;
mod function_render;
mod streaming;
mod type_render;

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, TypeDef};
use crate::docs::naming::{lang_display_name, lang_slug};
use crate::docs::sorting::{is_update_type, type_sort_key};
use crate::docs::template_env;
use std::path::PathBuf;

use enum_render::render_enum;
use error_render::render_error;
use excludes::language_excludes;
use function_render::render_function;
use type_render::render_type;

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
    out.push('\n');
    out.push_str(&template_env::render(
        "version_heading.jinja",
        minijinja::context! { marker => "##", title => format!("{lang_display} API Reference"), version => version },
    ));

    let public_fns: Vec<&FunctionDef> = api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(&f.name) && (lang == Language::Rust || !f.binding_excluded))
        .collect();
    if !public_fns.is_empty() {
        out.push_str("### Functions\n\n");
        for func in &public_fns {
            out.push_str(&render_function(func, lang, config, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    let mut types_to_doc: Vec<&TypeDef> = api
        .types
        .iter()
        .filter(|t| {
            !is_update_type(&t.name)
                && !exclude_types.contains(&t.name)
                && (lang == Language::Rust || !t.binding_excluded)
        })
        .collect();

    types_to_doc.sort_by(|a, b| type_sort_key(&a.name).cmp(&type_sort_key(&b.name)));

    if !types_to_doc.is_empty() {
        out.push_str("### Types\n\n");
        for ty in &types_to_doc {
            out.push_str(&render_type(ty, lang, config, api, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    let enums_to_doc: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| !exclude_types.contains(&e.name) && (lang == Language::Rust || !e.binding_excluded))
        .collect();
    if !enums_to_doc.is_empty() {
        out.push_str("### Enums\n\n");
        for en in &enums_to_doc {
            out.push_str(&render_enum(en, lang, ffi_prefix));
            out.push_str("\n---\n\n");
        }
    }

    let errors_to_doc: Vec<&ErrorDef> = api
        .errors
        .iter()
        .filter(|e| lang == Language::Rust || !e.binding_excluded)
        .collect();
    if !errors_to_doc.is_empty() {
        out.push_str("### Errors\n\n");
        for err in &errors_to_doc {
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
