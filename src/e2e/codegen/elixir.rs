//! Elixir e2e test generator using ExUnit.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use super::E2eCodegen;

/// Elixir e2e code generator.
pub struct ElixirCodegen;

impl E2eCodegen for ElixirCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let raw_module = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        // Convert module path to Elixir PascalCase if it looks like snake_case
        // (e.g., "demo_markup" -> "DemoMarkup").
        // If the override already contains "." (e.g., "Elixir.DemoMarkup"), use as-is.
        let module_path = if raw_module.contains('.') || raw_module.chars().next().is_some_and(|c| c.is_uppercase()) {
            raw_module.clone()
        } else {
            values::elixir_module_name(&raw_module)
        };
        let base_function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        // Elixir facade exports async variants with `_async` suffix when the call is async.
        // Append the suffix only if not already present and the function isn't a streaming
        // entry-point — streaming wrappers (e.g. `defaultclient_chat_stream`) drive the
        // FFI iterator handle and aren't async-callable in the OpenAI sense.
        let function_name =
            if call.r#async && !base_function_name.ends_with("_async") && !base_function_name.ends_with("_stream") {
                format!("{base_function_name}_async")
            } else {
                base_function_name
            };
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let options_default_fn = overrides.and_then(|o| o.options_via.clone());
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let handle_struct_type = overrides.and_then(|o| o.handle_struct_type.clone());
        let empty_atom_fields = std::collections::HashSet::new();
        let handle_atom_list_fields = overrides
            .map(|o| &o.handle_atom_list_fields)
            .unwrap_or(&empty_atom_fields);
        let result_var = &call.result_var;

        // Check if any fixture in any group is an HTTP test.
        let has_http_tests = groups.iter().any(|g| g.fixtures.iter().any(|f| f.is_http_test()));
        let has_nif_tests = groups.iter().any(|g| g.fixtures.iter().any(|f| !f.is_http_test()));
        // Check if any fixture needs the mock server (either via http or mock_response or client_factory).
        let has_mock_server_tests = groups.iter().any(|g| {
            g.fixtures.iter().any(|f| {
                if f.needs_mock_server() {
                    return true;
                }
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                let elixir_override = cc
                    .overrides
                    .get("elixir")
                    .or_else(|| e2e_config.call.overrides.get("elixir"));
                elixir_override.and_then(|o| o.client_factory.as_deref()).is_some()
            })
        });

        // Resolve package reference (path or version) for the NIF dependency.
        let pkg_ref = e2e_config.resolve_package(lang);
        let pkg_dep_ref = if has_nif_tests {
            match e2e_config.dep_mode {
                crate::e2e::config::DependencyMode::Local => pkg_ref
                    .as_ref()
                    .and_then(|p| p.path.as_deref())
                    .unwrap_or("../../packages/elixir")
                    .to_string(),
                crate::e2e::config::DependencyMode::Registry => pkg_ref
                    .as_ref()
                    .and_then(|p| p.version.clone())
                    .or_else(|| config.resolved_version())
                    .unwrap_or_else(|| "0.1.0".to_string()),
            }
        } else {
            String::new()
        };

        // Generate mix.exs. The dep atom must match the binding package's
        // mix `app:` value, not the crate name. Use the configured
        // `[elixir].app_name` (the same source the package's own mix.exs
        // uses); fall back to the crate name only when unset. Without this,
        // mix's path-dep resolution silently misroutes — the path-dep's
        // own deps (notably `:rustler_precompiled`) never load during its
        // compilation and the parent build fails with `RustlerPrecompiled
        // is not loaded`.
        let pkg_atom = config.elixir_app_name();
        // Check if there are HTTP fixtures that need server-pattern harness.
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();
        files.push(GeneratedFile {
            path: output_base.join("mix.exs"),
            content: project::render_mix_exs(
                &pkg_atom,
                &pkg_dep_ref,
                e2e_config.dep_mode,
                has_http_tests,
                has_mock_server_tests,
                has_nif_tests,
                uses_harness,
            ),
            generated_header: false,
        });

        // Generate lib/e2e_elixir.ex — required so the mix project compiles.
        files.push(GeneratedFile {
            path: output_base.join("lib").join("e2e_elixir.ex"),
            content: "defmodule E2eElixir do\n  @moduledoc false\nend\n".to_string(),
            generated_header: false,
        });

        // Generate app_harness.exs if using server-pattern HTTP fixtures.
        if uses_harness {
            files.push(GeneratedFile {
                path: output_base.join("app_harness.exs"),
                content: project::render_app_harness(e2e_config, groups, config),
                generated_header: true,
            });
        }

        // Generate test_helper.exs.
        files.push(GeneratedFile {
            path: output_base.join("test").join("test_helper.exs"),
            content: project::render_test_helper(has_http_tests || has_mock_server_tests, uses_harness, e2e_config),
            generated_header: false,
        });

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.exs", sanitize_filename(&group.category));
            let content = test_file::render_test_file(
                &group.category,
                &active,
                e2e_config,
                &module_path,
                &function_name,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                options_default_fn.as_deref(),
                enum_fields,
                handle_struct_type.as_deref(),
                handle_atom_list_fields,
                &config.adapters,
                enums,
                config,
                type_defs,
            );
            files.push(GeneratedFile {
                path: output_base.join("test").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "elixir"
    }
}

mod args;
mod assertions;
mod http;
mod project;
mod stubs;
mod test_case;
mod test_file;
mod values;
mod visitor;

pub use stubs::emit_test_backend;

#[cfg(test)]
mod tests;
