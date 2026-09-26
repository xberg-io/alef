//! Kotlin e2e test generator using kotlin.test and JUnit 5.
//!
//! Generates `packages/kotlin/src/test/kotlin/<package>/<Name>Test.kt` files
//! from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashSet;
use std::path::PathBuf;

use super::E2eCodegen;

mod args;
mod assertion_field_gates;
mod assertion_scalar_context;
mod assertion_scalar_dispatch;
mod assertions;
mod call_field_resolver;
mod discriminated;
mod http;
mod imports;
mod not_error;
mod project;
pub(crate) mod snippet;
mod stubs;
mod test_file;
mod test_method;
mod values;
mod visitor;

#[cfg(test)]
mod assertion_wildcard_element_tests;
#[cfg(test)]
mod collection_field_classification_tests;
#[cfg(test)]
mod enum_field_classification_tests;
#[cfg(test)]
mod optional_segment_len_tests;
#[cfg(test)]
mod qualified_class_import_tests;
#[cfg(test)]
mod tests;

pub use stubs::emit_test_backend;

pub(crate) use project::render_mock_server_listener_kt;
pub(crate) use test_file::render_test_file_android;

/// Kotlin e2e code generator.
pub struct KotlinE2eCodegen;

impl E2eCodegen for KotlinE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let _module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let class_name = overrides
            .and_then(|o| o.class.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_upper_camel_case());
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
        let result_var = call.effective_result_var();

        // Resolve package config.
        let kotlin_pkg = e2e_config.resolve_package("kotlin");
        let pkg_name = kotlin_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.clone());

        // Resolve Kotlin package for generated tests.
        let _kotlin_pkg_path = kotlin_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/kotlin".to_string());
        let kotlin_version = kotlin_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());
        let kotlin_pkg_id = config.kotlin_package();

        // Detect whether any fixture has HTTP requests (server-pattern SUT).
        let has_http_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Generate build.gradle.kts.
        files.push(GeneratedFile {
            path: output_base.join("build.gradle.kts"),
            content: project::render_build_gradle(
                &pkg_name,
                &kotlin_pkg_id,
                &kotlin_version,
                e2e_config.dep_mode,
                has_http_fixtures,
                &e2e_config.test_documents_relative_from(0),
            ),
            generated_header: false,
        });

        // Generate test files per category. Path mirrors the configured Kotlin
        // package so the package declaration in each test file matches its
        // filesystem location.
        let mut test_base = output_base.join("src").join("test").join("kotlin");
        for segment in kotlin_pkg_id.split('.') {
            test_base = test_base.join(segment);
        }
        let test_base = test_base.join("e2e");

        // Generate test setup for server-pattern tests.
        // SUT_URL is honored verbatim when preset by an external orchestrator
        // (e.g. `alef test-apps run`). Otherwise, mirroring the Java backend's
        // `MockServerListener`, spawn the `mock-server` binary once per JUnit
        // launcher session so `./gradlew test` works standalone without any
        // external harness — without this, tests reference a server that never
        // started and every request fails with `ConnectException`.
        if has_http_fixtures {
            files.push(GeneratedFile {
                path: test_base.join("SutServerSetup.kt"),
                content: project::render_sut_server_setup_kt(&kotlin_pkg_id),
                generated_header: true,
            });
            files.push(GeneratedFile {
                path: test_base.join("MockServerListener.kt"),
                content: project::render_mock_server_listener_kt(&kotlin_pkg_id, &e2e_config.alt_host),
                generated_header: true,
            });
            files.push(GeneratedFile {
                path: output_base
                    .join("src")
                    .join("test")
                    .join("resources")
                    .join("META-INF")
                    .join("services")
                    .join("org.junit.platform.launcher.LauncherSessionListener"),
                content: format!("{kotlin_pkg_id}.e2e.MockServerListener\n"),
                generated_header: false,
            });
        }

        // Resolve options_type from override.
        let options_type = overrides.and_then(|o| o.options_type.clone());

        // Build a map from TypeDef name → set of field names whose Rust type
        // is a `Named(T)` reference where `T` is NOT itself a known struct.
        // Those fields are enum-typed and should route through `.getValue()` in
        // generated assertions automatically, even without an explicit per-call
        // `enum_fields` override in the alef.toml.
        let struct_names: HashSet<&str> = type_defs.iter().map(|td| td.name.as_str()).collect();
        let type_enum_fields: std::collections::HashMap<String, HashSet<String>> = type_defs
            .iter()
            .filter_map(|td| {
                let enum_field_names: HashSet<String> = td
                    .fields
                    .iter()
                    .filter(|field| test_file::is_enum_typed(&field.ty, &struct_names))
                    .map(|field| field.name.clone())
                    .collect();
                if enum_field_names.is_empty() {
                    None
                } else {
                    Some((td.name.clone(), enum_field_names))
                }
            })
            .collect();

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_file_name = format!("{}Test.kt", sanitize_filename(&group.category).to_upper_camel_case());
            let content = test_file::render_test_file(
                &group.category,
                &active,
                &class_name,
                &function_name,
                &kotlin_pkg_id,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                result_is_simple,
                e2e_config,
                &type_enum_fields,
                config,
                type_defs,
                enums,
                functions,
            )?;
            files.push(GeneratedFile {
                path: test_base.join(class_file_name),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        snippet::render_snippet_body(fixture, e2e_config, config, type_defs, enums, false)
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        snippet::render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, false, functions)
    }

    fn language_name(&self) -> &'static str {
        "kotlin"
    }
}
