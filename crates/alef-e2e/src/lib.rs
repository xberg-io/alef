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
pub mod validate;

use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::config::e2e::DependencyMode;
use anyhow::{Context, Result};
use config::E2eConfig;
use fixture::{group_fixtures, load_fixtures};
use std::path::Path;
use tracing::{info, warn};
use validate::Severity;

/// Generate e2e test projects from fixtures.
///
/// Returns the list of generated files. The caller is responsible for writing
/// them to disk.
pub fn generate_e2e(
    alef_config: &AlefConfig,
    e2e_config: &E2eConfig,
    languages: Option<&[String]>,
) -> Result<Vec<GeneratedFile>> {
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let fixtures = load_fixtures(fixtures_dir)
        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;

    info!("Loaded {} fixture(s) from {}", fixtures.len(), e2e_config.fixtures);

    // Run semantic validation and emit warnings (don't block generation)
    let diagnostics = validate::validate_fixtures_semantic(&fixtures, e2e_config, &e2e_config.languages);
    for diag in &diagnostics {
        match diag.severity {
            Severity::Error => warn!("{}: {}", diag.file, diag.message),
            Severity::Warning => warn!("{}: {}", diag.file, diag.message),
        }
    }

    let all_groups = group_fixtures(&fixtures);

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

    let generators = if let Some(langs) = languages {
        codegen::generators_for(langs)
    } else if !e2e_config.languages.is_empty() {
        codegen::generators_for(&e2e_config.languages)
    } else {
        codegen::all_generators()
    };

    let mut all_files = Vec::new();
    for generator in &generators {
        let files = generator.generate(&groups, e2e_config, alef_config)?;
        info!("  [{}] generated {} file(s)", generator.language_name(), files.len());
        all_files.extend(files);
    }

    Ok(all_files)
}
