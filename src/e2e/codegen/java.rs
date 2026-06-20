//! Java e2e test generator using JUnit 5.
//!
//! Generates `e2e/java/pom.xml` and language-package test classes
//! files from JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::path::PathBuf;

use super::E2eCodegen;
use super::java_mvnw::{MAVEN_WRAPPER_PROPERTIES, MVNW_UNIX, MVNW_WINDOWS};

/// Java e2e code generator.
pub struct JavaCodegen;

impl E2eCodegen for JavaCodegen {
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
        let result_var = &call.result_var;

        // Resolve package config.
        let java_pkg = e2e_config.resolve_package("java");
        let pkg_name = java_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.clone());

        // Resolve Java package info for the dependency.
        let java_group_id = config.java_group_id();
        let binding_pkg = config.java_package();
        let pkg_version = config.resolved_version().unwrap_or_else(|| "0.1.0".to_string());

        // Prepare environment variables for Surefire configuration.
        let mut env_entries: Vec<(String, String)> = e2e_config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>();
        env_entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Generate pom.xml.
        // `harness_extras` deps support the alef-generated e2e harness code under
        // `e2e/{lang}/tests/` (Local dep mode). Registry mode emits the published-package
        // test_apps at `test_apps/{lang}/` whose tests only import the under-test package
        // and never need harness-specific dev deps. Injecting harness_extras here drags
        // unused native deps (e.g. upstream `io.github.tree-sitter:jtreesitter`) into
        // Maven downloads, which can break on newer Java versions that the unrelated
        // native build doesn't support yet.
        files.push(GeneratedFile {
            path: output_base.join("pom.xml"),
            content: project::render_pom_xml(
                &pkg_name,
                &java_group_id,
                &pkg_version,
                e2e_config,
                &config.ffi_lib_name(),
                &env_entries,
            ),
            generated_header: false,
        });

        // Maven wrapper: ./mvnw + mvnw.cmd + .mvn/wrapper/maven-wrapper.properties.
        // The wrapper scripts bootstrap-download maven-wrapper.jar from the URL in
        // maven-wrapper.properties on first invocation, so alef does not need to
        // emit the binary jar. The shebang on mvnw triggers 0755 chmod in the
        // file writer.
        files.push(GeneratedFile {
            path: output_base.join("mvnw"),
            content: MVNW_UNIX.to_string(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base.join("mvnw.cmd"),
            content: MVNW_WINDOWS.to_string(),
            generated_header: false,
        });
        files.push(GeneratedFile {
            path: output_base
                .join(".mvn")
                .join("wrapper")
                .join("maven-wrapper.properties"),
            content: MAVEN_WRAPPER_PROPERTIES.to_string(),
            generated_header: false,
        });

        // Check if there are HTTP fixtures that need server-pattern harness
        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_harness = has_http_fixtures && !e2e_config.harness.imports.is_empty();
        // Detect mock-server need from fixture `mock_response` or `http.expected_response`
        // shapes. Mirrors kotlin_android codegen.
        let needs_mock_server = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Generate test files per category. Path mirrors the configured Java
        // package — `dev.myorg` becomes `dev/myorg`, etc. — so the package
        // declaration in each test file matches its filesystem location.
        let mut test_base = output_base.join("src").join("test").join("java");
        for segment in java_group_id.split('.') {
            test_base = test_base.join(segment);
        }
        let test_base = test_base.join("e2e");

        // When any fixture needs a mock server, emit MockServerListener.java
        // plus its META-INF SPI entry so JUnit Platform discovers and starts
        // the `mock-server` binary once per launcher session. Without these
        // the tests reference `mockServerUrl` but no server runs, and the
        // existing service file (if left over from a prior alef version) points
        // at a class that does not exist on the classpath.
        if needs_mock_server {
            files.push(GeneratedFile {
                path: test_base.join("MockServerListener.java"),
                content: project::render_mock_server_listener(&java_group_id),
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
                content: format!("{java_group_id}.e2e.MockServerListener\n"),
                generated_header: false,
            });
        }

        // Emit fixture JSON files to src/test/resources/fixtures/ (avoids 65KB string literal limit)
        let fixtures_resource_base = output_base.join("src").join("test").join("resources").join("fixtures");
        for group in groups {
            for fixture in &group.fixtures {
                if fixture.http.is_none() {
                    continue;
                }
                let http_data = fixture.http.as_ref().unwrap();
                let fixture_json = serde_json::json!({
                    "http": {
                        "handler": {
                            "route": &http_data.handler.route,
                            "method": &http_data.handler.method,
                            "body_schema": http_data.handler.body_schema.clone(),
                        },
                        "request": {
                            "path": &http_data.request.path,
                        },
                        "expected_response": {
                            "status_code": http_data.expected_response.status_code,
                            "body": &http_data.expected_response.body,
                            "headers": &http_data.expected_response.headers,
                        }
                    }
                });
                let fixture_json_str = serde_json::to_string(&fixture_json).unwrap_or_default();
                files.push(GeneratedFile {
                    path: fixtures_resource_base.join(format!("{}.json", fixture.id)),
                    content: fixture_json_str,
                    generated_header: false,
                });
            }
        }

        // Emit FixtureLoader.java helper for loading fixtures from classpath
        if uses_harness {
            files.push(GeneratedFile {
                path: test_base.join("FixtureLoader.java"),
                content: project::render_fixture_loader(&java_group_id),
                generated_header: true,
            });
        }

        // Emit HarnessMain.java if server-pattern harness is needed
        if uses_harness {
            files.push(GeneratedFile {
                path: test_base.join("HarnessMain.java"),
                content: project::render_harness_main(e2e_config, groups, &java_group_id, &binding_pkg),
                generated_header: true,
            });
        }

        // Collect all distinct sealed-union type names declared in `assert_enum_fields`
        // across all call configs for this language.  For each such type we emit a
        // `{TypeName}Display.java` helper that pattern-matches on variants from the IR;
        // projects that declare no `assert_enum_fields` get no extra helper files.
        let sealed_display_types: std::collections::BTreeSet<String> = std::iter::once(&e2e_config.call)
            .chain(e2e_config.calls.values())
            .filter_map(|c| c.overrides.get(lang))
            .flat_map(|o| o.assert_enum_fields.values().cloned())
            .collect();

        for type_name in &sealed_display_types {
            if let Some(enum_def) = enums.iter().find(|e| &e.name == type_name) {
                files.push(GeneratedFile {
                    path: test_base.join(format!("{type_name}Display.java")),
                    content: project::render_sealed_display(type_name, enum_def, type_defs, &java_group_id),
                    generated_header: true,
                });
            }
        }

        // Resolve options_type: prefer Java override, fall back to other languages' options_type.
        // This ensures that when a call declares options_type in C#/Go/Python/PHP but not Java,
        // Java e2e tests still properly deserialize json_object args via JsonUtil.fromJson().
        let options_type = overrides.and_then(|o| o.options_type.clone()).or_else(|| {
            // Inherit from non-Java language overrides (C# first, then C, Go, PHP, Python).
            for cand in ["csharp", "c", "go", "php", "python"] {
                if let Some(o) = e2e_config.call.overrides.get(cand) {
                    if let Some(t) = &o.options_type {
                        return Some(t.clone());
                    }
                }
            }
            None
        });

        // Resolve enum_fields and nested_types from Java override config.
        static EMPTY_ENUM_FIELDS: std::sync::LazyLock<std::collections::HashMap<String, String>> =
            std::sync::LazyLock::new(std::collections::HashMap::new);
        let _enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&EMPTY_ENUM_FIELDS);

        // Build effective nested_types from configured overrides (empty by default).
        let mut effective_nested_types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(overrides_map) = overrides.map(|o| &o.nested_types) {
            effective_nested_types.extend(overrides_map.clone());
        }

        // Resolve nested_types_optional from override (defaults to true for backward compatibility).
        let nested_types_optional = overrides.map(|o| o.nested_types_optional).unwrap_or(true);

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_file_name = format!("{}Test.java", sanitize_filename(&group.category).to_upper_camel_case());
            let content = test_file::render_test_file(
                &group.category,
                &active,
                &class_name,
                &function_name,
                &java_group_id,
                &binding_pkg,
                result_var,
                &e2e_config.call.args,
                options_type.as_deref(),
                result_is_simple,
                e2e_config,
                &effective_nested_types,
                nested_types_optional,
                &config.adapters,
                config,
                type_defs,
                uses_harness,
            );
            files.push(GeneratedFile {
                path: test_base.join(class_file_name),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "java"
    }
}

mod args;
mod assertions;
mod http;
mod project;
mod stubs;
mod test_file;
mod test_method;
mod values;
mod visitor;

pub use stubs::emit_test_backend;

#[cfg(test)]
mod tests;
