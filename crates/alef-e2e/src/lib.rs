//! Fixture-driven e2e test generation for alef.
//!
//! This crate generates complete, runnable e2e test projects for all supported
//! languages from JSON fixture files. Each project is self-contained with
//! build files, test files, and local package references.

pub mod codegen;
pub mod config;
pub mod escape;
pub mod field_access;
pub mod fixture;
pub mod format;
pub mod scaffold;
pub mod template_env;
pub mod validate;

use alef_core::backend::GeneratedFile;
use alef_core::config::e2e::DependencyMode;
use alef_core::config::{Language, ResolvedCrateConfig};
use anyhow::{Context, Result};
use config::E2eConfig;
use fixture::{group_fixtures, load_fixtures};
use std::path::Path;
use tracing::{info, warn};
use validate::Severity;

/// Map the top-level `[languages]` list (the scaffolded bindings) to the
/// e2e generator names registered in [`codegen::all_generators`].
///
/// `Language::Ffi` maps to the `c` generator (the FFI binding's e2e harness
/// is the C test runner). `Language::Rust` is always appended because rust is
/// the source language and the rust e2e suite exercises the core crate.
///
/// Generators that don't have a corresponding `Language` variant (e.g.
/// `brew`) are intentionally excluded — they require an explicit opt-in via
/// `[e2e].languages` in alef.toml.
pub fn default_e2e_languages(scaffolded: &[Language]) -> Vec<String> {
    let mut names: Vec<String> = scaffolded
        .iter()
        .map(|l| match l {
            Language::Ffi => "c".to_string(),
            other => other.to_string(),
        })
        .collect();
    if !names.iter().any(|n| n == "rust") {
        names.push("rust".to_string());
    }
    names
}

/// Generate e2e test projects from fixtures.
///
/// Returns the list of generated files. The caller is responsible for writing
/// them to disk.
///
/// `type_defs` is the IR type registry for the source crate. Pass
/// `&api.types` from the extracted [`alef_core::ir::ApiSurface`]. It is
/// forwarded to generators that need to introspect struct field types (e.g.
/// the TypeScript/WASM backend uses it to auto-derive `nested_types` for
/// wasm-bindgen class wrapping). Pass an empty slice when the registry is not
/// available; generators will fall back to explicit call-override mappings.
pub fn generate_e2e(
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    languages: Option<&[String]>,
    type_defs: &[alef_core::ir::TypeDef],
) -> Result<Vec<GeneratedFile>> {
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let fixtures = load_fixtures(fixtures_dir)
        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;

    info!("Loaded {} fixture(s) from {}", fixtures.len(), e2e_config.fixtures);

    // Resolution order for which language generators to run:
    //   1. Explicit `--lang` filter from the CLI (highest priority).
    //   2. `[e2e].languages` from alef.toml when set.
    //   3. The top-level `[languages]` list mapped to e2e generator names —
    //      so e2e tests are only generated for actually scaffolded bindings,
    //      never for backends the consumer hasn't opted into.
    //
    // The legacy `all_generators()` fallback is removed; emitting tests for
    // languages without a matching binding produces broken e2e dirs that
    // cannot compile.
    let resolved_languages: Vec<String> = if let Some(langs) = languages {
        langs.to_vec()
    } else if !e2e_config.languages.is_empty() {
        e2e_config.languages.clone()
    } else {
        default_e2e_languages(&config.languages)
    };

    // Run semantic validation against the resolved language set so the
    // empty-category check warns about the same languages we're about to
    // generate for.
    let diagnostics = validate::validate_fixtures_semantic(&fixtures, e2e_config, &resolved_languages);
    for diag in &diagnostics {
        match diag.severity {
            Severity::Error => warn!("{}: {}", diag.file, diag.message),
            Severity::Warning => warn!("{}: {}", diag.file, diag.message),
        }
    }

    let all_groups = group_fixtures(&fixtures);

    // Drop categories that are explicitly excluded from cross-language e2e
    // codegen. These fixtures stay on disk for Rust integration tests but
    // never reach binding generators.
    let all_groups: Vec<_> = if e2e_config.exclude_categories.is_empty() {
        all_groups
    } else {
        all_groups
            .into_iter()
            .filter(|g| !e2e_config.exclude_categories.contains(&g.category))
            .collect()
    };

    // In registry mode with a non-empty category filter, keep only the listed
    // categories so the generated test apps contain a curated subset.
    let groups: Vec<_> =
        if e2e_config.dep_mode == DependencyMode::Registry && !e2e_config.registry.categories.is_empty() {
            let allowed = &e2e_config.registry.categories;
            all_groups
                .into_iter()
                .filter(|g| allowed.iter().any(|c| c == &g.category))
                .collect()
        } else {
            all_groups
        };

    let generators = codegen::generators_for(&resolved_languages);

    let mut all_files = Vec::new();
    for generator in &generators {
        let files = generator.generate(&groups, e2e_config, config, type_defs)?;
        info!("  [{}] generated {} file(s)", generator.language_name(), files.len());
        all_files.extend(files);
    }

    Ok(all_files)
}
