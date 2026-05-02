//! TypeScript e2e test generator using vitest.

mod assertions;
mod config;
mod json;
mod test_file;
mod visitors;

use crate::config::E2eConfig;
use crate::field_access::FieldResolver;
use crate::fixture::FixtureGroup;
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use anyhow::Result;
use std::path::PathBuf;

use super::E2eCodegen;
use config::{render_file_setup, render_global_setup, render_package_json, render_tsconfig, render_vitest_config};
pub use test_file::render_test_file;
use test_file::resolve_node_function_name;

/// TypeScript e2e code generator.
pub struct TypeScriptCodegen;

impl E2eCodegen for TypeScriptCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let output_base = PathBuf::from(e2e_config.effective_output()).join(self.language_name());
        let tests_base = output_base.join("tests");

        let mut files = Vec::new();

        // Resolve call config with overrides — use "node" key (Language::Node).
        let call = &e2e_config.call;
        let overrides = call.overrides.get("node");
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides.and_then(|o| o.function.as_ref()).cloned().unwrap_or_else(|| {
            let default_cc = e2e_config.resolve_call(None);
            resolve_node_function_name(default_cc)
        });
        let client_factory = overrides.and_then(|o| o.client_factory.as_deref());

        // Resolve package config.
        let node_pkg = e2e_config.resolve_package("node");
        let pkg_path = node_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/typescript".to_string());
        let pkg_name = node_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_version = node_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        let has_http_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.is_http_test());

        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call(f.call.as_deref());
            cc.args
                .iter()
                .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
        });

        files.push(GeneratedFile {
            path: output_base.join("package.json"),
            content: render_package_json(
                &pkg_name,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
                has_http_fixtures,
            ),
            generated_header: false,
        });

        files.push(GeneratedFile {
            path: output_base.join("tsconfig.json"),
            content: render_tsconfig(),
            generated_header: false,
        });

        let needs_global_setup = client_factory.is_some() || has_http_fixtures;

        files.push(GeneratedFile {
            path: output_base.join("vitest.config.ts"),
            content: render_vitest_config(needs_global_setup, has_file_fixtures),
            generated_header: true,
        });

        if needs_global_setup {
            files.push(GeneratedFile {
                path: output_base.join("globalSetup.ts"),
                content: render_global_setup(),
                generated_header: true,
            });
        }

        if has_file_fixtures {
            files.push(GeneratedFile {
                path: output_base.join("setup.ts"),
                content: render_file_setup(),
                generated_header: true,
            });
        }

        let options_type = overrides.and_then(|o| o.options_type.clone());
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        for group in groups {
            let active: Vec<_> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip("node")))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}.test.ts", crate::escape::sanitize_filename(&group.category));
            let content = render_test_file(
                "node",
                &group.category,
                &active,
                &module_path,
                &pkg_name,
                &function_name,
                &e2e_config.call.args,
                options_type.as_deref(),
                &field_resolver,
                client_factory,
                e2e_config,
            );
            files.push(GeneratedFile {
                path: tests_base.join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "node"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_name_is_node() {
        let codegen = TypeScriptCodegen;
        assert_eq!(codegen.language_name(), "node");
    }

    #[test]
    fn generate_empty_groups_produces_config_files_only() {
        use alef_core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
        )
        .unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let codegen = TypeScriptCodegen;
        let files = codegen.generate(&[], &e2e, &resolved).unwrap();
        // package.json, tsconfig.json, vitest.config.ts
        assert!(files.len() >= 3, "got {} files", files.len());
    }
}
