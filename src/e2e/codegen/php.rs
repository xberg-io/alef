//! PHP e2e test generator using PHPUnit.
//!
//! Generates `e2e/php/composer.json`, `e2e/php/phpunit.xml`, and
//! `tests/{Category}Test.php` files from JSON fixtures, driven entirely by
//! `E2eConfig` and `CallConfig`.

use crate::backends::php::layout::php_psr4_target;
use crate::backends::php::naming::php_autoload_namespace;
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashMap;
use std::path::PathBuf;

use super::E2eCodegen;

/// PHP e2e code generator.
pub struct PhpCodegen;

impl E2eCodegen for PhpCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve top-level call config to derive class/namespace/factory — these are
        // shared across all categories. Per-fixture call routing (function name, args)
        // is resolved inside render_test_method via e2e_config.resolve_call().
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let extension_name = config.php_extension_name();
        let class_name = overrides
            .and_then(|o| o.class.as_ref())
            .cloned()
            .map(|cn| cn.split('\\').next_back().unwrap_or(&cn).to_string())
            .unwrap_or_else(|| extension_name.to_upper_camel_case());
        let namespace = overrides.and_then(|o| o.module.as_ref()).cloned().unwrap_or_else(|| {
            if extension_name.contains('_') {
                extension_name
                    .split('_')
                    .map(|p| p.to_upper_camel_case())
                    .collect::<Vec<_>>()
                    .join("\\")
            } else {
                extension_name.to_upper_camel_case()
            }
        });
        let empty_enum_fields = HashMap::new();
        let enum_fields = overrides.map(|o| &o.enum_fields).unwrap_or(&empty_enum_fields);
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);
        let php_client_factory = overrides.and_then(|o| o.php_client_factory.as_deref());
        let options_via = overrides.and_then(|o| o.options_via.as_deref()).unwrap_or("array");

        // Resolve package config.
        let php_pkg = e2e_config.resolve_package("php");
        let pkg_name = php_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                // Derive `<org>/<package>` for Packagist from the configured repository URL.
                // The Packagist package name is typically based on call.module (not the Rust
                // crate name), which may include `-rs` for FFI crates. For PHP (which uses
                // the pure Packagist name without language suffixes), strip `-rs` if present.
                let org = config
                    .try_github_repo()
                    .ok()
                    .as_deref()
                    .and_then(crate::core::config::derive_repo_org)
                    .unwrap_or_else(|| config.name.clone());
                let mut pkg_module = call.module.replace('_', "-");
                // Strip Rust FFI crate suffix for Packagist package naming convention.
                if pkg_module.ends_with("-rs") {
                    pkg_module = pkg_module[..pkg_module.len() - 3].to_string();
                }
                format!("{org}/{pkg_module}")
            });
        let pkg_path = php_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| default_php_pkg_path(config));
        let pkg_version = php_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // Derive the e2e composer project metadata from the consumer-binding
        // pkg_name (`<vendor>/<crate>`) and the configured PHP autoload
        // namespace — alef is vendor-neutral, so we don't fall back to a
        // fixed "sample_core" string.
        let e2e_vendor = pkg_name.split('/').next().unwrap_or(&pkg_name).to_string();
        let e2e_pkg_name = format!("{e2e_vendor}/e2e-php");
        // PSR-4 autoload keys appear inside a JSON document, so each PHP
        // namespace separator must be JSON-escaped (`\` → `\\`). The trailing
        // pair represents the PHP-mandated trailing `\` (which itself escapes
        // to `\\` in JSON).
        let php_namespace = php_autoload_namespace(config);
        let php_namespace_escaped = php_namespace.replace('\\', "\\\\");
        let e2e_autoload_ns = format!("{php_namespace_escaped}\\\\E2e\\\\");

        // Generate composer.json.
        let autoload_target = php_autoload_target(config, php_pkg.as_ref().and_then(|p| p.path.as_deref()));
        files.push(GeneratedFile {
            path: output_base.join("composer.json"),
            content: project::render_composer_json(
                &e2e_pkg_name,
                &e2e_autoload_ns,
                &extension_name,
                &php_namespace,
                &autoload_target,
                &pkg_version,
                e2e_config.dep_mode,
            ),
            generated_header: false,
        });

        // Generate install.sh (registry mode only) — bootstraps PIE and installs
        // the extension before `composer install` runs in the verify-install flow.
        // The pinned version is baked in at generate time so callers can run
        // `bash install.sh` with no args.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            files.push(GeneratedFile {
                path: output_base.join("install.sh"),
                content: project::render_install_sh(&pkg_name, &extension_name, &pkg_version),
                generated_header: false,
            });
        }

        // `generated_header: true` so `ensure_generated_header` actually runs and stamps
        // provenance. Without it poly cannot recognise the file as generated, lints it as
        // hand-written source, and then alef and poly fight over its bytes on every regen.
        // The extension table was NOT the cause here — `marker_header_syntax` handles `.xml`
        // (marker on line 1, after the `<?xml ?>` declaration); this callsite simply never
        // asked for a header. Safe to flip specifically because e2e/test_apps files are
        // written with `overwrite = true`, so `can_skip` is already false and this controls
        // nothing but the header. Do NOT copy the flip to a `packages/**` file on that
        // reasoning — those are create-once and flipping it there can freeze them. ~keep
        files.push(GeneratedFile {
            path: output_base.join("phpunit.xml"),
            content: project::render_phpunit_xml(),
            generated_header: true,
        });

        // Check if any fixture needs a mock HTTP server (either http-shape or
        // demo-client mock_response-shape) so bootstrap.php spawns it.
        let has_mock_server_fixtures = groups
            .iter()
            .flat_map(|g| g.fixtures.iter())
            .any(|f| f.needs_mock_server());

        // Check if any fixture uses HTTP server-pattern (has http field and harness config).
        let has_http_server_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| f.http.is_some());
        let uses_server_harness = has_http_server_fixtures && !e2e_config.harness.imports.is_empty();

        let file_input_scan = super::file_inputs::FileInputScan::new(type_defs, enums);
        // Check if any fixture reads test-document content, including byte fields nested in a
        // typed request object.
        let has_file_fixtures = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
            let cc = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            file_input_scan.fixture_uses_test_documents(f, cc)
        });

        // app_harness.php is now emitted by a consumer extension.

        // Generate bootstrap.php that loads both autoloaders and optionally starts the mock server.
        files.push(GeneratedFile {
            path: output_base.join("bootstrap.php"),
            content: project::render_bootstrap(project::BootstrapOptions {
                e2e_config,
                pkg_path: &pkg_path,
                has_mock_server_fixtures,
                has_file_fixtures,
                test_documents_path: &e2e_config.test_documents_relative_from(0),
                uses_server_harness,
                harness_host: &e2e_config.harness.host,
                harness_port: e2e_config.harness.port,
            }),
            generated_header: true,
        });

        // Generate run_tests.php that loads the extension and invokes phpunit.
        // The Cargo package name shown in the build hint mirrors the same default the
        // scaffold uses for the crate itself (`{core_crate_dir}-php`) when no explicit
        // `cargo_crate_name` override is configured. ~keep
        let cargo_package_name = config
            .php_cargo_crate_name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-php", config.core_crate_dir()));
        files.push(GeneratedFile {
            path: output_base.join("run_tests.php"),
            content: project::render_run_tests_php(
                &extension_name,
                config.php_cargo_crate_name(),
                &cargo_package_name,
                &pkg_version,
            ),
            generated_header: true,
        });

        // Generate test files per category.
        let tests_base = output_base.join("tests");

        // Compute per-(type, field) getter classification for PHP.
        // ext-php-rs 0.15.x exposes scalar fields as PHP properties via `#[php(prop)]`,
        // but non-scalar fields (Named structs, Vec<Named>, Map, etc.) need a
        // `#[php(getter)]` method because `get_method_props` is unimplemented in
        // ext-php-rs-derive 0.11.7. E2e assertions must call `->getCamelCase()` for those.
        //
        // The classification MUST be keyed by (owner_type, field_name) rather than
        // bare field_name: two unrelated types can declare the same field name with
        // different scalarness (e.g. `CrawlConfig.content: ContentConfig` vs
        // `MarkdownResult.content: String`). A bare-name union would force every
        // `->content` access to `->getContent()` even on types where it is a scalar
        // property. This covers DTOs where `getContent()` is a true accessor
        // without forcing getter syntax for scalar fields where the method does
        // not exist.
        let php_enum_lowering = enum_variant_access::PhpEnumLowering::from_enums(enums);

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let test_class = format!("{}Test", sanitize_filename(&group.category).to_upper_camel_case());
            let filename = format!("{test_class}.php");
            let php_lang_rename_all = config.serde_rename_all_for_language(crate::core::config::Language::Php);
            let content = test_file::render_test_file(
                &group.category,
                &active,
                e2e_config,
                lang,
                &namespace,
                &class_name,
                &test_class,
                type_defs,
                enums,
                functions,
                &php_enum_lowering,
                enum_fields,
                result_is_simple,
                php_client_factory,
                options_via,
                &config.adapters,
                php_lang_rename_all,
                config,
                errors,
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
        "php"
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        snippet::render_snippet_body(fixture, e2e_config, config, type_defs, enums)
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
        snippet::render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, functions)
    }
}

/// Number of directory levels between a generated PHP project (`e2e/php`, `test_apps/php`) and
/// the repository root every alef output path is spelled relative to.
const E2E_PROJECT_TO_REPO_ROOT: &str = "../../";

/// PSR-4 target the generated `composer.json` maps the binding namespace to.
///
/// Unset `[crates.e2e.packages.php] path` — the normal case — resolves the directory alef
/// actually writes the userland classes into, via the one authority every other PHP stage reads
/// ([`php_psr4_target`]), so the e2e project and the repository-root manifest cannot name two
/// different trees. An explicit `path` keeps its historical meaning: a package root whose classes
/// sit in its nested `src/`. ~keep
fn php_autoload_target(config: &ResolvedCrateConfig, configured_pkg_path: Option<&str>) -> String {
    match configured_pkg_path {
        Some(path) => format!("{}/src/", path.trim_end_matches('/')),
        None => format!("{E2E_PROJECT_TO_REPO_ROOT}{}", php_psr4_target(config)),
    }
}

/// Default `path` for the local PHP composer dependency when
/// `[crates.e2e.packages.php].path` is unset.
///
/// This is the package ROOT — the directory `bootstrap.php` looks for a `vendor/autoload.php`
/// under — not the class directory. The PSR-4 target is [`php_autoload_target`]'s job; the two
/// are separate because a co-located layout has its classes in a `src/` child of this root while
/// the manifest that would create `vendor/` sits at the root itself. ~keep
///
/// Derived from [`ResolvedCrateConfig::package_dir`] for [`Language::Php`], which follows
/// `[crates.output] php` when configured, or, unconfigured, resolves through
/// `OutputTemplate::resolve`'s default to `crates/<pkg>-php/src` — the co-located layout
/// that places the generated userland classes beside the PHP binding crate, matching
/// where the scaffolder writes `crates/<pkg>-php/Cargo.toml`. The trailing `/src` is stripped
/// so the result names the crate root rather than its source directory.
fn default_php_pkg_path(config: &ResolvedCrateConfig) -> String {
    let pkg_dir = config.package_dir(Language::Php);
    let trimmed = pkg_dir.trim_end_matches('/');
    if trimmed.is_empty() {
        return "../../packages/php".to_string();
    }
    let crate_root = trimmed.strip_suffix("/src").unwrap_or(trimmed);
    if crate_root.is_empty() {
        return "../../packages/php".to_string();
    }
    format!("../../{crate_root}")
}

mod args;
mod assertions;
mod call_field_resolver;
mod enum_variant_access;
mod http;
mod project;
mod snippet;
mod stubs;
mod test_file;
mod test_method;
mod types;
pub(crate) mod values;
mod visitor;

pub use stubs::{emit_test_backend, emit_test_backend_with_ns};

#[cfg(test)]
mod assertion_wildcard_element_tests;
#[cfg(test)]
mod composer_autoload_tests;
#[cfg(all(test, unix))]
mod install_sh_execution_tests;
#[cfg(test)]
mod tests;
