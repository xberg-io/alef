mod enum_render;
mod error_render;
mod excludes;
mod function_render;
mod streaming;
mod type_render;

use crate::codegen::cfg::collect_cfg_features;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, ErrorDef, FunctionDef, TypeDef};
use crate::docs::naming::{lang_display_name, lang_slug};
use crate::docs::sorting::{is_update_type, type_sort_key};
use crate::docs::template_env;
use std::collections::HashSet;
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
    // Filter the surface to what this binding actually compiles: items (and
    // their fields/variants) gated behind a `#[cfg(feature = "...")]` the
    // binding does not enable are compiled out of the real binding, so they
    // must not appear in its reference docs.
    let effective_features = effective_docs_features(api, config, lang);
    let enabled_features: HashSet<&str> = effective_features.iter().map(String::as_str).collect();
    let filtered_api = api.with_cfg_filtered_deep(&enabled_features);
    let api = &filtered_api;

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

/// Compute the feature set a language's binding actually compiles, for the
/// purpose of cfg-filtering the reference docs.
///
/// This must mirror how each backend resolves the feature set that lands in its
/// generated Cargo.toml, otherwise the docs diverge from the real binding
/// surface:
///
/// - Swift and Dart force-enable *every* feature referenced by any `#[cfg]` in
///   the surface (so all conditionally-compiled items build), minus the
///   features listed in `excluded_default_features`. Their generated
///   `[features] default = [...]` is exactly that set (see the swift/dart
///   `gen_rust_crate` cargo emitters), so docs must use the same set — using
///   only the configured `features` list would wrongly drop items gated behind
///   features that are implicitly enabled.
/// - Every other language uses its configured feature list directly (an
///   umbrella feature such as `full`, `wasm-target`, or an explicit list). For
///   those, `features_for_language` already reflects what the binding compiles.
fn effective_docs_features(api: &ApiSurface, config: &ResolvedCrateConfig, lang: Language) -> Vec<String> {
    let mut features: HashSet<String> = config.features_for_language(lang).iter().cloned().collect();

    let excluded_default: Option<&[String]> = match lang {
        Language::Swift => config.swift.as_ref().map(|c| c.excluded_default_features.as_slice()),
        Language::Dart => config.dart.as_ref().map(|c| c.excluded_default_features.as_slice()),
        _ => None,
    };

    if let Some(excluded) = excluded_default {
        let excluded: HashSet<&str> = excluded.iter().map(String::as_str).collect();
        for feature in collect_cfg_features(api) {
            if !excluded.contains(feature.as_str()) {
                features.insert(feature);
            }
        }
    }

    features.into_iter().collect()
}
