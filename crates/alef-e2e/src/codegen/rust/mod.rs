//! Rust e2e test code generator.
//!
//! Generates `e2e/rust/Cargo.toml` and `tests/{category}_test.rs` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

pub mod assertions;
pub mod cargo_toml;
pub mod http;
pub mod mock_server;
pub mod test_file;

mod args;
mod assertion_helpers;
mod assertion_synthetic;

pub use cargo_toml::render_cargo_toml;
pub use mock_server::{render_common_module, render_mock_server_binary, render_mock_server_module};

use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use anyhow::Result;
use std::path::PathBuf;

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::fixture::{Fixture, FixtureGroup};

use super::E2eCodegen;
use test_file::{is_skipped, render_test_file};

/// Rust e2e test code generator.
pub struct RustE2eCodegen;

impl E2eCodegen for RustE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[alef_core::ir::TypeDef],
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("rust");

        // Resolve crate name and path from config.
        let crate_name = resolve_crate_name(e2e_config, config);
        let crate_path = resolve_crate_path(e2e_config, &crate_name);
        let dep_name = crate_name.replace('-', "_");

        // Cargo.toml
        // Check if any call config (default or named) uses json_object/handle args (needs serde_json dep).
        let all_call_configs = std::iter::once(&e2e_config.call).chain(e2e_config.calls.values());
        let needs_serde_json = all_call_configs
            .flat_map(|c| c.args.iter())
            .any(|a| a.arg_type == "json_object" || a.arg_type == "handle");

        // Check if any fixture in any group requires a mock HTTP server.
        // This includes both liter-llm mock_response fixtures and spikard http fixtures.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.needs_mock_server());

        // Check if any fixture uses the http integration test pattern (spikard http fixtures).
        let needs_http_tests = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| !is_skipped(f, "rust") && f.http.is_some());

        // Check if any http fixture uses CORS or static-files middleware (needs tower-http).
        let needs_tower_http = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .filter(|f| !is_skipped(f, "rust"))
            .filter_map(|f| f.http.as_ref())
            .filter_map(|h| h.handler.middleware.as_ref())
            .any(|m| m.cors.is_some() || m.static_files.is_some());

        // Tokio is needed when any test is async (mock server, http tests, or async call config).
        let any_async_call = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .any(|c| c.r#async);
        let needs_tokio = needs_mock_server || needs_http_tests || any_async_call;

        let crate_version = resolve_crate_version(e2e_config).or_else(|| config.resolved_version());
        files.push(GeneratedFile {
            path: output_base.join("Cargo.toml"),
            content: render_cargo_toml(
                &crate_name,
                &dep_name,
                &crate_path,
                needs_serde_json,
                needs_mock_server,
                needs_http_tests,
                needs_tokio,
                needs_tower_http,
                e2e_config.dep_mode,
                crate_version.as_deref(),
                &config.features,
            ),
            generated_header: true,
        });

        // Generate mock_server.rs when at least one fixture uses mock_response.
        if needs_mock_server {
            files.push(GeneratedFile {
                path: output_base.join("tests").join("mock_server.rs"),
                content: render_mock_server_module(),
                generated_header: true,
            });
            // Generate common.rs module for spawning the standalone mock-server binary.
            files.push(GeneratedFile {
                path: output_base.join("tests").join("common.rs"),
                content: render_common_module(),
                generated_header: true,
            });
        }
        // Always generate standalone mock-server binary for cross-language e2e suites
        // when any fixture has http data (serves fixture responses for non-Rust tests).
        if needs_mock_server || needs_http_tests {
            files.push(GeneratedFile {
                path: output_base.join("src").join("main.rs"),
                content: render_mock_server_binary(),
                generated_header: true,
            });
        }

        // Per-category test files.
        for group in groups {
            let fixtures: Vec<&Fixture> = group.fixtures.iter().filter(|f| !is_skipped(f, "rust")).collect();

            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("{}_test.rs", sanitize_filename(&group.category));
            let content = render_test_file(&group.category, &fixtures, e2e_config, &dep_name, needs_mock_server);

            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "rust"
    }
}

// ---------------------------------------------------------------------------
// Config resolution helpers
// ---------------------------------------------------------------------------

fn resolve_crate_name(_e2e_config: &E2eConfig, config: &ResolvedCrateConfig) -> String {
    // Always use the Cargo package name (with hyphens) from alef.toml [crate].
    // The `crate_name` override in [e2e.call.overrides.rust] is for the Rust
    // import identifier, not the Cargo package name.
    config.name.clone()
}

fn resolve_crate_path(e2e_config: &E2eConfig, crate_name: &str) -> String {
    e2e_config
        .resolve_package("rust")
        .and_then(|p| p.path.clone())
        .unwrap_or_else(|| format!("../../crates/{crate_name}"))
}

fn resolve_crate_version(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config.resolve_package("rust").and_then(|p| p.version.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_crate_name_uses_config_name() {
        use alef_core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my_lib"
result_var = "result"
"#,
        )
        .unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let name = resolve_crate_name(&e2e, &resolved);
        assert_eq!(name, "my-lib");
    }
}
