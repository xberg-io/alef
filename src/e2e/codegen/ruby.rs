//! Ruby e2e test generator using RSpec.
//!
//! Generates `e2e/ruby/Gemfile` and `spec/{category}_spec.rb` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use super::E2eCodegen;

/// Ruby e2e code generator.
pub struct RubyCodegen;

impl E2eCodegen for RubyCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
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
        let class_name = overrides.and_then(|o| o.class.as_ref()).cloned();
        let options_type = overrides.and_then(|o| o.options_type.clone());
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_is_simple = call.result_is_simple || overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let ruby_pkg = e2e_config.resolve_package("ruby");
        let gem_name = ruby_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.replace('-', "_"));
        let gem_path = ruby_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/ruby".to_string());
        let gem_version = ruby_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate Gemfile.
        files.push(GeneratedFile {
            path: output_base.join("Gemfile"),
            content: project::render_gemfile(&gem_name, &gem_path, &gem_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate .rubocop.yaml for linting generated specs.
        files.push(GeneratedFile {
            path: output_base.join(".rubocop.yaml"),
            content: project::render_rubocop_yaml(),
            generated_header: false,
        });

        // Check if there are HTTP fixtures that need server-pattern harness.
        // When uses_harness is true, a consumer extension owns the server-pattern
        // files (app_harness.rb, spec_helper.rb server variant, and all *_spec.rb files).
        // alef only emits those files for the client/mock-server pattern (uses_harness == false).
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();

        // Check if any fixture is an HTTP test (needs mock server bootstrap).
        let has_mock_server_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Check if any fixture uses file_path or bytes args (needs chdir to test_documents).
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        // For client/mock-server pattern only: emit spec_helper.rb.
        // The server-pattern spec_helper.rb (uses_harness) is emitted by the extension.
        if !uses_harness && (has_file_fixtures || has_mock_server_fixtures) {
            files.push(GeneratedFile {
                path: output_base.join("spec").join("spec_helper.rb"),
                content: project::render_spec_helper(
                    has_file_fixtures,
                    has_mock_server_fixtures,
                    false,
                    &e2e_config.test_documents_relative_from(1),
                    &gem_name,
                    &module_path,
                    &e2e_config.harness.host,
                    e2e_config.harness.port,
                    &e2e_config.env,
                ),
                generated_header: true,
            });
        }

        // Generate spec files per category.
        // When uses_harness is true, spec files are owned by a consumer extension.
        if !uses_harness {
            let spec_base = output_base.join("spec");

            for group in groups {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    .collect();

                if active.is_empty() {
                    continue;
                }

                // Skip the entire file if no fixture in this category produces output.
                let has_any_output = active.iter().any(|f| {
                    // HTTP tests always produce output.
                    if f.is_http_test() {
                        return true;
                    }
                    let cc = e2e_config.resolve_call_for_fixture(
                        f.call.as_deref(),
                        &f.id,
                        &f.resolved_category(),
                        &f.tags,
                        &f.input,
                    );
                    let fr = FieldResolver::new(
                        e2e_config.effective_fields(cc),
                        e2e_config.effective_fields_optional(cc),
                        e2e_config.effective_result_fields(cc),
                        e2e_config.effective_fields_array(cc),
                        &std::collections::HashSet::new(),
                    );
                    let expects_error = f.assertions.iter().any(|a| a.assertion_type == "error");
                    let has_not_error = f.assertions.iter().any(|a| a.assertion_type == "not_error");
                    expects_error || has_not_error || spec_file::has_usable_assertion(f, &fr, result_is_simple)
                });
                if !has_any_output {
                    continue;
                }

                let filename = format!("{}_spec.rb", sanitize_filename(&group.category));
                let content = spec_file::render_spec_file(
                    &group.category,
                    &active,
                    &module_path,
                    class_name.as_deref(),
                    &gem_name,
                    options_type.as_deref(),
                    enum_fields,
                    result_is_simple,
                    e2e_config,
                    has_file_fixtures || has_mock_server_fixtures,
                    false,
                    &config.adapters,
                    config,
                    type_defs,
                );
                files.push(GeneratedFile {
                    path: spec_base.join(filename),
                    content,
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "ruby"
    }
}

mod args;
mod assertions;
mod examples;
mod http;
mod project;
mod spec_file;
mod stubs;
mod values;
mod visitor;

pub use stubs::emit_test_backend;

#[cfg(test)]
mod tests;
