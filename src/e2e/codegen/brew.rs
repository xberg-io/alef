//! Brew (Homebrew CLI) e2e test generator.
//!
//! Generates a self-contained shell-script test suite that tests a CLI binary
//! installed via Homebrew.  The suite consists of:
//!
//! - `run_tests.sh` — main runner that sources per-category files, tracks
//!   pass/fail counts and exits 1 on any failure.
//! - `test_{category}.sh` — one file per fixture category, each containing
//!   a `test_{fixture_id}()` shell function.
//!
//! Each test function:
//! 1. Constructs a CLI invocation: `{binary} {subcommand} "{url}" {flags...}`
//! 2. Captures stdout into a variable.
//! 3. Uses `jq` to extract fields and runs helper assertion functions.
//!
//! Requirements at runtime: `bash`, `jq`, and `MOCK_SERVER_URL` env var.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::path::PathBuf;

use super::E2eCodegen;

mod category;
mod run_tests;

use category::render_category_file;
use run_tests::render_run_tests;

/// Brew (Homebrew CLI) e2e code generator.
pub struct BrewCodegen;

impl E2eCodegen for BrewCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        // Resolve call config with overrides for the "brew" language key.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        // Default subcommand (used when fixture has no routing tags).
        let default_subcommand = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());

        // Static CLI flags appended to every invocation.
        let static_cli_args: Vec<String> = overrides.map(|o| o.cli_args.clone()).unwrap_or_default();

        // Field-to-flag mapping (fixture input field → CLI flag name).
        let cli_flags: std::collections::HashMap<String, String> =
            overrides.map(|o| o.cli_flags.clone()).unwrap_or_default();

        // Resolve binary name from the "brew" package entry, falling back to call.module.
        let binary_name = e2e_config
            .registry
            .packages
            .get(lang)
            .and_then(|p| p.name.as_ref())
            .cloned()
            .or_else(|| e2e_config.packages.get(lang).and_then(|p| p.name.as_ref()).cloned())
            .unwrap_or_else(|| call.module.clone());

        // Filter active groups (non-skipped fixtures).
        let active_groups: Vec<(&FixtureGroup, Vec<&Fixture>)> = groups
            .iter()
            .filter_map(|group| {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    .collect();
                if active.is_empty() { None } else { Some((group, active)) }
            })
            .collect();

        let mut files = Vec::new();

        // Generate run_tests.sh.
        let category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
        files.push(GeneratedFile {
            path: output_base.join("run_tests.sh"),
            content: render_run_tests(&category_names),
            generated_header: true,
        });

        // Generate per-category test files.
        for (group, active) in &active_groups {
            let safe_category = sanitize_filename(&group.category);
            let filename = format!("test_{safe_category}.sh");
            let content = render_category_file(
                &group.category,
                active,
                &binary_name,
                &default_subcommand,
                &static_cli_args,
                &cli_flags,
                &e2e_config.call.args,
                e2e_config,
            );
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "brew"
    }
}

/// Emit a brew test backend stub.
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    super::TestBackendEmission::unimplemented("brew")
}
