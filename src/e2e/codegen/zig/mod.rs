//! Zig e2e test generator using std.testing.
//!
//! Generates `packages/zig/src/<crate>_test.zig` files from JSON fixtures,
//! driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::AdapterPattern;
use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions::toolchain;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_zig, sanitize_filename};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use anyhow::{Result, bail};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;
use super::streaming_assertions::{StreamingFieldResolver, is_streaming_virtual_field};

/// Zig e2e code generator.
mod args;
mod assertions;
mod build;
mod hash;
mod http;
mod stubs;
mod test_file;
mod visitor;

pub use stubs::emit_test_backend;

use build::{ZigBuildFlags, render_build_zig, render_build_zig_zon};
use hash::{detect_stale_zig_hash, resolve_zig_hash, supported_zig_platforms, uses_platform_registry_deps};
use test_file::render_test_file;

pub struct ZigE2eCodegen;

impl E2eCodegen for ZigE2eCodegen {
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
        let _module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;

        // Resolve package config.
        let zig_pkg = e2e_config.resolve_package("zig");
        let pkg_path = zig_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/zig".to_string());
        let pkg_name = zig_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_snake_case());
        let pkg_version = zig_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());
        // Explicit hash override from alef.toml takes precedence over auto-fetch.
        // However, if the hash is a placeholder (contains STALE_HASH_REGENERATE), treat it as missing
        // and fetch the real hash from the network instead.
        let explicit_hash = zig_pkg.as_ref().and_then(|p| p.hash.clone());
        let platform_hash_overrides = zig_pkg.as_ref().map(|p| p.platform_hashes.clone()).unwrap_or_default();

        // Use the crate name for constructing the release URL (hyphenated form).
        let crate_name = &config.name;

        // Strip placeholder hashes so we can fetch the real ones.
        let explicit_hash_clean = explicit_hash.and_then(|h| {
            if h.contains("STALE_HASH_REGENERATE") {
                None
            } else {
                Some(h)
            }
        });

        // Detect if the explicit hash is stale: if it contains an embedded version
        // string (format: `<pkg_name>-X.Y.Z-<hash>`) and that version doesn't match
        // the current pkg_version, warn and recommend regeneration.
        let hash_is_stale = if let Some(ref h) = explicit_hash_clean {
            detect_stale_zig_hash(h, &pkg_version, &pkg_name)
        } else {
            false
        };
        // Resolve content multihashes for registry mode. A single `hash` applies only to the
        // generic package tarball. Platform-specific release assets must provide
        // `platform_hashes`, because Zig hashes are content-specific.
        let platform_hashes = if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            if hash_is_stale {
                bail!(
                    "zig registry package hash is stale for crate `{}` version `{}`; update `[crates.e2e.registry.packages.zig].hash`",
                    config.name,
                    pkg_version
                );
            }
            let Some(github_repo_owned) = e2e_config.registry.github_repo.as_deref() else {
                bail!(
                    "zig registry mode requires explicit `[crates.e2e.registry] github_repo` for crate `{}`",
                    config.name
                );
            };
            let github_repo = github_repo_owned.trim_end_matches('/');
            let mut hashes = BTreeMap::new();
            if platform_hash_overrides.is_empty() {
                // Try to use explicit hash (already cleaned of placeholders); if missing, fetch from network.
                let url =
                    format!("{github_repo}/releases/download/v{pkg_version}/{crate_name}-zig-v{pkg_version}.tar.gz");
                hashes.insert(
                    "generic".to_string(),
                    (url.clone(), resolve_zig_hash(explicit_hash_clean.as_deref(), &url)),
                );
            } else {
                for platform in supported_zig_platforms() {
                    let Some(platform_hash) = platform_hash_overrides.get(*platform) else {
                        bail!(
                            "zig registry mode requires `[crates.e2e.registry.packages.zig.platform_hashes.{platform}]` for crate `{}`",
                            config.name
                        );
                    };
                    // Strip placeholder hashes (parity with explicit_hash_clean above) so
                    // resolve_zig_hash falls through to cache lookup / network fetch instead
                    // of emitting the literal placeholder string as the dependency hash.
                    let platform_hash_clean = if platform_hash.contains("STALE_HASH_REGENERATE") {
                        None
                    } else {
                        Some(platform_hash.as_str())
                    };
                    let url = format!(
                        "{github_repo}/releases/download/v{pkg_version}/{crate_name}-zig-v{pkg_version}-{platform}.tar.gz"
                    );
                    hashes.insert(
                        platform.to_string(),
                        (url.clone(), resolve_zig_hash(platform_hash_clean, &url)),
                    );
                }
            }
            hashes
        } else {
            BTreeMap::new()
        };
        let use_platform_registry_deps = uses_platform_registry_deps(&platform_hashes);

        // Host-capsule passthrough deps (e.g. zig-tree-sitter). In Local mode the e2e
        // rebuilds the binding module from source, so any capsule dependency the binding
        // `@import`s must be re-declared in the e2e manifest. Each tuple is
        // (module_name, url, hash): the module name is the bare Zig import identifier the
        // binding source `@import`s (`tree_sitter`), which is the trailing identifier of the
        // host_type's namespace — i.e. the last `[A-Za-z0-9_]` run before the type's `.`.
        // `?*const tree_sitter.Language` and `*tree_sitter.Language` both yield `tree_sitter`,
        // matching the import name the package scaffold wires in.
        let mut zig_capsule_deps: Vec<(String, String, String)> = config
            .zig
            .as_ref()
            .map(|z| {
                let mut deps: Vec<(String, String, String)> = z
                    .capsule_types
                    .values()
                    .filter(|cap| !cap.package.is_empty())
                    .filter_map(|cap| {
                        let import_name = capsule_import_name(&cap.host_type)?;
                        Some((import_name, cap.package.clone(), cap.package_version.clone()))
                    })
                    .collect();
                deps.sort();
                deps.dedup();
                deps
            })
            .unwrap_or_default();

        // Merge harness_extras into capsule deps when in Local mode. The harness_extras
        // allow hand-written passthrough tests to @import upstream packages like zig-tree-sitter.
        // Each extra is converted to a (module_name, url, hash) tuple and appended to capsule_deps.
        // Last-write-wins: if a module name collides (same key in both sources), the harness_extras
        // version takes precedence.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Local {
            if let Some(extras) = e2e_config.harness_extras.get(self.language_name()) {
                if !extras.is_empty() {
                    // Merge dependencies (runtime + dev_dependencies combined in Local mode).
                    let mut harness_extras_deps = Vec::new();
                    for (module_name, spec) in &extras.dependencies {
                        if let crate::core::config::manifest_extras::ExtraDepSpec::Detailed(table) = spec {
                            if let (Some(url_val), Some(hash_val)) = (
                                table.get("url").and_then(|v| v.as_str()),
                                table.get("hash").and_then(|v| v.as_str()),
                            ) {
                                harness_extras_deps.push((
                                    module_name.clone(),
                                    url_val.to_string(),
                                    hash_val.to_string(),
                                ));
                            }
                        }
                    }
                    for (module_name, spec) in &extras.dev_dependencies {
                        if let crate::core::config::manifest_extras::ExtraDepSpec::Detailed(table) = spec {
                            if let (Some(url_val), Some(hash_val)) = (
                                table.get("url").and_then(|v| v.as_str()),
                                table.get("hash").and_then(|v| v.as_str()),
                            ) {
                                harness_extras_deps.push((
                                    module_name.clone(),
                                    url_val.to_string(),
                                    hash_val.to_string(),
                                ));
                            }
                        }
                    }
                    // Merge by removing duplicates (keep harness_extras, remove earlier capsule_deps with same module_name).
                    let mut seen_modules = std::collections::HashSet::new();
                    for (module_name, _, _) in &harness_extras_deps {
                        seen_modules.insert(module_name.clone());
                    }
                    zig_capsule_deps.retain(|(module_name, _, _)| !seen_modules.contains(module_name));
                    zig_capsule_deps.extend(harness_extras_deps);
                    zig_capsule_deps.sort();
                    zig_capsule_deps.dedup();
                }
            }
        }

        // Generate build.zig.zon (Zig package manifest).
        files.push(GeneratedFile {
            path: output_base.join("build.zig.zon"),
            content: render_build_zig_zon(
                &pkg_name,
                &pkg_path,
                e2e_config.dep_mode,
                &pkg_version,
                &platform_hashes,
                hash_is_stale,
                &zig_capsule_deps,
            ),
            generated_header: false,
        });

        // Get the module name for imports.
        let module_name = config.zig_module_name();
        let ffi_prefix = config.ffi_prefix();

        // Generate build.zig - collect test file names first.

        // Whether any active fixture uses file-based args (`file_path` or
        // `bytes`). Only when true do the generated tests need the working
        // directory to be `test_documents/` at run time. Consumers whose
        // fixtures are mock-server-only have no
        // `test_documents/` directory, so emitting `setCwd` for them causes
        // `FileNotFound` at spawn time because zig tries to `chdir` into a
        // directory that does not exist before execing the test binary.
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

        // Whether any fixture hits the mock server: a direct HTTP fixture, a
        // fixture with a mock_response, or a function-call fixture that derives
        // its URL from a `mock_url` / `mock_url_list` arg or a `client_factory`
        // override. Zig has no test-suite init hook, so when true the generated
        // `build.zig` spawns the mock-server binary at configure time and exports
        // `MOCK_SERVER_URL` into every test run step's environment. Without it the
        // tests fall back to `http://localhost:8080` and fail with connection
        // refused (the server binds an ephemeral 127.0.0.1 port).
        let needs_mock_server = groups.iter().flat_map(|g| g.fixtures.iter()).any(|f| {
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
            if cc
                .args
                .iter()
                .any(|a| a.arg_type == "mock_url" || a.arg_type == "mock_url_list")
            {
                return true;
            }
            cc.overrides
                .get("zig")
                .or_else(|| e2e_config.call.overrides.get("zig"))
                .and_then(|o| o.client_factory.as_deref())
                .is_some()
        });

        // Zig language filtering: when `[crates.zig].languages` is set, omit
        // fixtures whose target language falls outside that static-compiled list.
        // The Zig binding does not dynamically load sample_language parsers; only the
        // grammars compiled into the static set at build time are available at
        // runtime. Without this filter, fixtures like `smoke_bibtex` would emit
        // tests that fail to load their parser. Mirrors the WASM pattern.
        let zig_languages = config.zig.as_ref().and_then(|z| {
            if z.languages.is_empty() {
                None
            } else {
                Some(z.languages.clone())
            }
        });

        // Generate test files per category and collect their names.
        //
        // The Zig backend does not yet support streaming free functions (the
        // generated binding exposes only the unary entry points). Skip any
        // fixture whose resolved call is marked `streaming = true` so we don't
        // emit streaming calls that fail to compile
        // against a binding that lacks them. Streaming support tracked
        // separately — see streaming-audit notes ("Zig: last-chunk-only").
        let mut test_filenames: Vec<String> = Vec::new();
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .filter(|f| {
                    // When `[crates.zig].languages` is set, drop fixtures whose
                    // target grammar isn't in the static-compiled set. Inspect
                    // both shapes alef fixtures use: top-level `input.language`
                    // (function-call shape) and nested `input.config.language`
                    // (config-object shape used by smoke fixtures).
                    if let Some(ref zig_langs) = zig_languages {
                        let fix_lang = f.input.get("language").and_then(|v| v.as_str()).or_else(|| {
                            f.input
                                .get("config")
                                .and_then(|c| c.get("language"))
                                .and_then(|v| v.as_str())
                        });
                        if let Some(fix_lang) = fix_lang
                            && !zig_langs.iter().any(|l| l == fix_lang)
                        {
                            return false;
                        }
                    }
                    true
                })
                .filter(|f| {
                    let cc = e2e_config.resolve_call_for_fixture(
                        f.call.as_deref(),
                        &f.id,
                        &f.resolved_category(),
                        &f.tags,
                        &f.input,
                    );
                    cc.streaming_enabled() != Some(true)
                })
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.zig", sanitize_filename(&group.category));
            test_filenames.push(filename.clone());
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                &function_name,
                result_var,
                &e2e_config.call.args,
                &module_name,
                &ffi_prefix,
                config,
                type_defs,
            );
            files.push(GeneratedFile {
                path: output_base.join("src").join(filename),
                content,
                generated_header: true,
            });
        }

        // Generate build.zig with collected test files.
        files.insert(
            files
                .iter()
                .position(|f| f.path.file_name().is_some_and(|n| n == "build.zig.zon"))
                .unwrap_or(1),
            GeneratedFile {
                path: output_base.join("build.zig"),
                content: render_build_zig(
                    &test_filenames,
                    &pkg_name,
                    &module_name,
                    &config.ffi_lib_name(),
                    &config.ffi_crate_path(),
                    ZigBuildFlags {
                        has_file_fixtures,
                        needs_mock_server,
                    },
                    &e2e_config.test_documents_relative_from(0),
                    e2e_config.dep_mode,
                    use_platform_registry_deps,
                    &e2e_config.env,
                    &zig_capsule_deps,
                    e2e_config.extra_system_libs_for("zig"),
                ),
                generated_header: false,
            },
        );

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "zig"
    }
}

/// Derive the bare Zig `@import` identifier for a host-capsule type from its
/// configured `host_type`. The Zig binding source imports the capsule package by
/// its module name (e.g. `@import("my_mod")`), which is the trailing
/// `[A-Za-z0-9_]` run of the namespace token — the portion before the type's
/// final `.`, stripped of pointer/const decoration. `?*const my_mod.Language`
/// and `*my_mod.Language` both yield `my_mod`.
///
/// Returns `None` when the host_type contains no dotted qualified name (e.g. a
/// bare type with no module prefix), which is a config error — callers should
/// skip or error on `None`.
fn capsule_import_name(host_type: &str) -> Option<String> {
    // A qualified name must contain a '.'. Bare types (no dot) have no module prefix.
    if !host_type.contains('.') {
        return None;
    }
    // Take the portion before the first dot; that is the namespace token (may have
    // leading sigil characters like `?`, `*`, `const`).
    let namespace_token = host_type.split('.').next().unwrap_or("");
    // Strip leading non-identifier characters (sigils: ?, *, const, etc.) to extract
    // the bare module identifier.
    let name = namespace_token
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|segment| !segment.is_empty())?;
    Some(name.to_string())
}

#[cfg(test)]
mod capsule_import_name_tests {
    use super::capsule_import_name;

    #[test]
    fn strips_zig_pointer_and_const_decoration() {
        assert_eq!(
            capsule_import_name("?*const my_mod.Language"),
            Some("my_mod".to_string())
        );
    }

    #[test]
    fn strips_bare_pointer_decoration() {
        assert_eq!(capsule_import_name("*my_mod.Language"), Some("my_mod".to_string()));
    }

    #[test]
    fn passes_through_plain_namespace() {
        assert_eq!(capsule_import_name("my_mod.Language"), Some("my_mod".to_string()));
    }

    #[test]
    fn returns_none_when_no_dotted_identifier() {
        // Namespace is pure pointer decoration with no identifier run to extract.
        assert_eq!(capsule_import_name("?*.Language"), None);
    }

    #[test]
    fn returns_none_for_bare_unqualified_type() {
        // A bare type without module prefix cannot yield an import name.
        assert_eq!(capsule_import_name("Language"), None);
    }
}
