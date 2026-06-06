//! Gleam e2e test generator using gleeunit/should.
//!
//! Generates `packages/gleam/test/<crate>_test.gleam` files from JSON fixtures.
//! HTTP fixtures hit the mock server at `MOCK_SERVER_URL/fixtures/<id>` using
//! the `gleam_httpc` HTTP client library. Non-HTTP fixtures without a gleam-specific
//! call override emit a skip stub.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToSnakeCase;
use std::path::PathBuf;

use super::E2eCodegen;

mod args;
mod assertions;
mod constructors;
mod http;
mod project;
mod stubs;
mod test_case;
mod test_file;
mod values;

#[cfg(test)]
mod tests;

pub use stubs::emit_test_backend;

/// Gleam e2e code generator.
pub struct GleamE2eCodegen;

impl E2eCodegen for GleamE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;

        // Resolve package config.
        let gleam_pkg = e2e_config.resolve_package("gleam");
        let pkg_path = gleam_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/gleam".to_string());
        let pkg_name = gleam_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_snake_case());

        // Generate gleam.toml.
        files.push(GeneratedFile {
            path: output_base.join("gleam.toml"),
            content: project::render_gleam_toml(&pkg_path, &pkg_name, e2e_config.dep_mode),
            generated_header: false,
        });

        // OTP application atom. Defaults to the snake-cased binding crate
        // name (matches `pkg_name`); kept as a separate binding because
        // `application:ensure_all_started/1` and the Erlang shim function
        // identifier both interpolate this same atom.
        let app_name = pkg_name.clone();

        // Gleam requires a `src/` directory even for test-only projects.
        // Emit a helper module with `read_file_bytes` external for loading test
        // documents as BitArray at runtime.
        let e2e_helpers = project::render_e2e_helpers_source(&app_name);

        // Erlang shim module that starts the configured OTP application and all deps.
        // Compiled alongside the Gleam source when gleam test is run.
        // Must start elixir first (provides Elixir.Application used by Rustler NIF init),
        // then ensure the binding OTP application and its transitive deps are running.
        let erlang_startup = project::render_erlang_startup_source(&app_name);
        files.push(GeneratedFile {
            path: output_base.join("src").join("e2e_gleam.gleam"),
            content: e2e_helpers,
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base.join("src").join("e2e_startup.erl"),
            content: erlang_startup,
            generated_header: false,
        });

        // Track whether any test file was emitted.
        let mut any_tests = false;

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                // Include both HTTP and non-HTTP fixtures. Filter out those marked as skip.
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                // gleam_httpc cannot follow HTTP/1.1 protocol upgrades (101 Switching
                // Protocols), so skip WebSocket-upgrade fixtures whose request advertises
                // Upgrade: websocket. The server returns 101 and gleam_httpc times out.
                .filter(|f| {
                    if let Some(http) = &f.http {
                        let has_upgrade = http
                            .request
                            .headers
                            .iter()
                            .any(|(k, v)| k.eq_ignore_ascii_case("upgrade") && v.eq_ignore_ascii_case("websocket"));
                        !has_upgrade
                    } else {
                        true
                    }
                })
                // For non-HTTP fixtures, include all (will use default or override call config).
                // Gleam always has a call override or can use the default call config.
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.gleam", sanitize_filename(&group.category));
            // Look up gleam-specific config for element_type → record-constructor
            // recipes. Empty slice when the project hasn't configured any.
            let element_constructors: &[crate::core::config::GleamElementConstructor] = config
                .gleam
                .as_ref()
                .map(|g| g.element_constructors.as_slice())
                .unwrap_or(&[]);
            // Optional wrapper template used when a json_object arg has no
            // matching element_type recipe.
            let json_object_wrapper: Option<&str> =
                config.gleam.as_ref().and_then(|g| g.json_object_wrapper.as_deref());
            let content = test_file::render_test_file(
                &group.category,
                &active,
                e2e_config,
                &module_path,
                &function_name,
                result_var,
                &e2e_config.call.args,
                element_constructors,
                json_object_wrapper,
            );
            files.push(GeneratedFile {
                path: output_base.join("test").join(filename),
                content,
                generated_header: true,
            });
            any_tests = true;
        }

        // Always emit the gleeunit entry module — `gleam test` invokes
        // `<package>_test.main()` to discover and run all `_test.gleam` files.
        // When no fixture-driven tests exist, also include a tiny smoke test so
        // the suite is non-empty.
        let entry = if any_tests {
            concat!(
                "// Generated by alef. Do not edit by hand.\n",
                "import gleeunit\n",
                "import e2e_gleam\n",
                "\n",
                "pub fn main() {\n",
                "  let _ = e2e_gleam.start_app()\n",
                "  gleeunit.main()\n",
                "}\n",
            )
            .to_string()
        } else {
            concat!(
                "// Generated by alef. Do not edit by hand.\n",
                "// No fixture-driven tests for Gleam — e2e tests require HTTP fixtures\n",
                "// or non-HTTP fixtures with gleam-specific call overrides.\n",
                "import gleeunit\n",
                "import gleeunit/should\n",
                "\n",
                "pub fn main() {\n",
                "  gleeunit.main()\n",
                "}\n",
                "\n",
                "pub fn compilation_smoke_test() {\n",
                "  True |> should.equal(True)\n",
                "}\n",
            )
            .to_string()
        };
        files.push(GeneratedFile {
            path: output_base.join("test").join("e2e_gleam_test.gleam"),
            content: entry,
            generated_header: false,
        });

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "gleam"
    }
}
