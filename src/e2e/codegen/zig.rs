//! Zig e2e test generator using std.testing.
//!
//! Generates `packages/zig/src/<crate>_test.zig` files from JSON fixtures,
//! driven entirely by `E2eConfig` and `CallConfig`.

use crate::core::backend::GeneratedFile;
use crate::core::config::AdapterPattern;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
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
        let explicit_hash = zig_pkg.as_ref().and_then(|p| p.hash.clone());
        let platform_hash_overrides = zig_pkg.as_ref().map(|p| p.platform_hashes.clone()).unwrap_or_default();

        // Use the crate name for constructing the release URL (hyphenated form).
        let crate_name = &config.name;

        // Detect if the explicit hash is stale: if it contains an embedded version
        // string (format: `<pkg_name>-X.Y.Z-<hash>`) and that version doesn't match
        // the current pkg_version, warn and recommend regeneration.
        let hash_is_stale = if let Some(ref h) = explicit_hash {
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
                let Some(explicit_hash) = explicit_hash.as_deref() else {
                    bail!(
                        "zig registry mode requires explicit `[crates.e2e.registry.packages.zig] hash` or platform_hashes for crate `{}`",
                        config.name
                    );
                };
                let url =
                    format!("{github_repo}/releases/download/v{pkg_version}/{crate_name}-zig-v{pkg_version}.tar.gz");
                hashes.insert(
                    "generic".to_string(),
                    (url.clone(), resolve_zig_hash(Some(explicit_hash), &url)),
                );
            } else {
                for platform in supported_zig_platforms() {
                    let Some(platform_hash) = platform_hash_overrides.get(*platform) else {
                        bail!(
                            "zig registry mode requires `[crates.e2e.registry.packages.zig.platform_hashes.{platform}]` for crate `{}`",
                            config.name
                        );
                    };
                    let url = format!(
                        "{github_repo}/releases/download/v{pkg_version}/{crate_name}-zig-v{pkg_version}-{platform}.tar.gz"
                    );
                    hashes.insert(
                        platform.to_string(),
                        (url.clone(), resolve_zig_hash(Some(platform_hash), &url)),
                    );
                }
            }
            hashes
        } else {
            BTreeMap::new()
        };
        let use_platform_registry_deps = uses_platform_registry_deps(&platform_hashes);

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

// ---------------------------------------------------------------------------
// Zig content-multihash resolution
// ---------------------------------------------------------------------------

/// Detect if a Zig package hash contains a stale embedded version.
///
/// Zig package hashes are formatted as `<pkg_name>-<version>-<multihash>` where
/// `pkg_name` is the snake_case module name (e.g., `demo_client`, not `demo-client`).
/// This function extracts the embedded version and compares it against
/// the current package version. If they differ, the hash is stale and
/// should be regenerated with `alef sync-versions`.
///
/// Returns `true` if the hash is stale (embedded version != current version).
/// Logs a warning in that case.
fn detect_stale_zig_hash(hash: &str, current_version: &str, pkg_name: &str) -> bool {
    // Hash format: `{pkg_name}-{version}-{multihash}`
    // Example: `demo_client-1.4.0-rc.50-Jfgk_NcsAQBpkv3XrckgE9vZmwDERDOandv0Ud6LXpHH`
    let prefix = format!("{pkg_name}-");
    if !hash.starts_with(&prefix) {
        return false;
    }

    // Remove the crate name prefix and split the rest by dashes.
    let rest = &hash[prefix.len()..];
    let parts: Vec<&str> = rest.split('-').collect();

    // Reconstruct the version by iterating through parts until we hit
    // the hash-like segment (long alphanumeric or underscore string).
    let mut version_parts: Vec<&str> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        // Last part is always the hash; don't include it.
        if i == parts.len() - 1 {
            break;
        }

        version_parts.push(part);

        // Heuristic: if this part looks like a hash (>20 chars or contains underscores/alphanumerics),
        // and we've accumulated at least one version part, stop here.
        if part.len() > 20 || (part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && i > 0) {
            // This is likely the hash segment; remove it from version_parts.
            version_parts.pop();
            break;
        }
    }

    let embedded_version = version_parts.join("-");

    if embedded_version != current_version {
        tracing::warn!(
            "zig package hash mismatch: hash contains version '{}', but current version is '{}'; \
             regenerate with `alef sync-versions`",
            embedded_version,
            current_version
        );
        return true;
    }

    false
}

/// Path to the on-disk hash cache: `~/.cache/alef/zig-hashes.json` on Unix /
/// `%LOCALAPPDATA%\alef\zig-hashes.json` on Windows.
///
/// Returns `None` when the home / local-app-data environment variable is unset.
fn zig_hash_cache_path() -> Option<std::path::PathBuf> {
    // XDG_CACHE_HOME takes precedence on Linux.
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("alef").join("zig-hashes.json"));
        }
    }
    // macOS and Linux: $HOME/.cache/alef/zig-hashes.json
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(
                std::path::PathBuf::from(home)
                    .join(".cache")
                    .join("alef")
                    .join("zig-hashes.json"),
            );
        }
    }
    // Windows: %LOCALAPPDATA%\alef\zig-hashes.json
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        if !local_app.is_empty() {
            return Some(std::path::PathBuf::from(local_app).join("alef").join("zig-hashes.json"));
        }
    }
    None
}

/// Read the (URL → hash) cache. Returns an empty map on any I/O error.
fn read_zig_hash_cache() -> std::collections::HashMap<String, String> {
    let Some(path) = zig_hash_cache_path() else {
        return std::collections::HashMap::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Persist a single (url → hash) entry into the cache.
fn write_zig_hash_cache_entry(url: &str, hash: &str) {
    let Some(path) = zig_hash_cache_path() else {
        return;
    };
    let mut map = read_zig_hash_cache();
    map.insert(url.to_string(), hash.to_string());
    let Ok(json) = serde_json::to_string_pretty(&map) else {
        return;
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, json).ok();
}

/// Fetch the content multihash for a Zig package tarball URL by shelling out
/// to `zig fetch <url>` from a scratch directory.
///
/// Returns the hash string (printed by `zig fetch` on stdout) on success, or
/// `None` when `zig fetch` is unavailable / returns a non-zero exit code /
/// produces no recognisable hash output.
fn fetch_zig_hash_from_network(url: &str) -> Option<String> {
    let tmp = tempfile::tempdir().ok()?;
    // Write a minimal stub build.zig.zon so `zig fetch` has a valid package
    // context to operate from. Without it, older zig versions refuse to run.
    let stub = r#".{
    .name = .zig_hash_fetch_stub,
    .version = "0.0.0",
    .fingerprint = 0x0000000000000001,
    .dependencies = .{},
    .paths = .{"build.zig.zon"},
}
"#;
    std::fs::write(tmp.path().join("build.zig.zon"), stub).ok()?;
    // `zig fetch <url>` (hash-only, no `--save`) still aborts with "no build.zig
    // file found" unless a build.zig exists in the directory tree, so write a
    // no-op one alongside the manifest.
    std::fs::write(
        tmp.path().join("build.zig"),
        "pub fn build(b: *@import(\"std\").Build) void {\n    _ = b;\n}\n",
    )
    .ok()?;

    let output = std::process::Command::new("zig")
        .arg("fetch")
        .arg(url)
        .current_dir(tmp.path())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // `zig fetch` prints the content multihash on stdout as a single line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

/// Resolve the content multihash for a Zig registry tarball URL.
///
/// Resolution order:
/// 1. `explicit` — a `hash` value set directly in `alef.toml` under
///    `[crates.e2e.registry.packages.zig]`. Takes precedence over everything.
/// 2. Cache — `~/.cache/alef/zig-hashes.json` keyed by URL.
/// 3. Network — shells out to `zig fetch <url>`, parses the printed hash,
///    writes the result back to the cache, and returns it.
/// 4. Fallback — logs a warning and returns `None`. Registry generation
///    requires an explicit hash before calling this helper, so this path is
///    only available to tests and non-publishable dry-run callers.
fn resolve_zig_hash(explicit: Option<&str>, url: &str) -> Option<String> {
    // 1. Explicit override wins.
    if let Some(h) = explicit {
        return Some(h.to_string());
    }

    // 2. On-disk cache.
    let cache = read_zig_hash_cache();
    if let Some(h) = cache.get(url) {
        return Some(h.clone());
    }

    // 3. Network fetch.
    match fetch_zig_hash_from_network(url) {
        Some(h) => {
            write_zig_hash_cache_entry(url, &h);
            Some(h)
        }
        None => {
            tracing::warn!(
                "zig hash skipped — asset {} not yet published; regen after release",
                url
            );
            None
        }
    }
}

fn supported_zig_platforms() -> &'static [&'static str] {
    &[
        "linux-x86_64",
        "linux-aarch64",
        "macos-arm64",
        "macos-x86_64",
        "windows-x86_64",
    ]
}

fn uses_platform_registry_deps(platform_hashes: &BTreeMap<String, (String, Option<String>)>) -> bool {
    platform_hashes.keys().any(|platform| platform != "generic")
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_build_zig_zon(
    pkg_name: &str,
    pkg_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    version: &str,
    platform_hashes: &BTreeMap<String, (String, Option<String>)>,
    hash_is_stale: bool,
) -> String {
    let dep_block = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            let use_platform_registry_deps = uses_platform_registry_deps(platform_hashes);
            let mut entries = Vec::new();
            for (platform, (url, hash_opt)) in platform_hashes {
                let dep_name = if use_platform_registry_deps {
                    format!("{}_{}", pkg_name, platform.replace('-', "_"))
                } else {
                    pkg_name.to_string()
                };
                let lazy_line = if use_platform_registry_deps {
                    "\n            .lazy = true,"
                } else {
                    ""
                };
                let entry = match hash_opt {
                    Some(h) if hash_is_stale => {
                        format!(
                            "        // STALE hash (embedded version != current); regenerate via `alef sync-versions`\n        // expected to match crate v{version}, was: {h}\n        .{dep_name} = .{{\n            .url = \"{url}\",{lazy_line}\n        }},",
                            version = version,
                            url = url,
                            h = h,
                            dep_name = dep_name,
                            lazy_line = lazy_line
                        )
                    }
                    Some(h) => {
                        format!(
                            "        .{dep_name} = .{{\n            .url = \"{url}\",\n            .hash = \"{h}\",{lazy_line}\n        }},",
                            dep_name = dep_name,
                            url = url,
                            h = h,
                            lazy_line = lazy_line
                        )
                    }
                    None => {
                        format!(
                            "        .{dep_name} = .{{\n            .url = \"{url}\",{lazy_line}\n        }},",
                            dep_name = dep_name,
                            url = url,
                            lazy_line = lazy_line
                        )
                    }
                };
                entries.push(entry);
            }
            entries.join("\n")
        }
        crate::e2e::config::DependencyMode::Local => {
            // Zig 0.16+ requires named dependencies. Use the package name as the key.
            format!("        .{pkg_name} = .{{\n            .path = \"{pkg_path}\",\n        }},")
        }
    };

    let min_zig = toolchain::MIN_ZIG_VERSION;
    // Zig 0.16+ requires a fingerprint of the form (crc32_ieee(name) << 32) | id.
    let name_bytes: &[u8] = b"e2e_zig";
    let mut crc: u32 = 0xffff_ffff;
    for byte in name_bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    let name_crc: u32 = !crc;
    let mut id: u32 = 0x811c_9dc5;
    for byte in name_bytes {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    let fingerprint: u64 = ((name_crc as u64) << 32) | (id as u64);

    let dep_content = format!(".{{\n{dep_block}\n    }}");

    format!(
        r#".{{
    .name = .e2e_zig,
    .version = "0.1.0",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = {dep_content},
    .paths = .{{
        "build.zig",
        "build.zig.zon",
        "src",
    }},
}}
"#
    )
}

/// Fixture-shape flags that toggle optional `build.zig` wiring.
#[derive(Debug, Clone, Copy)]
struct ZigBuildFlags {
    /// Any fixture loads files by path (`file_path`/`bytes` args) and so the
    /// test run step must `setCwd` into the test-documents directory.
    has_file_fixtures: bool,
    /// Any fixture hits the mock server, so `build.zig` must spawn it and export
    /// `MOCK_SERVER_URL` into the test run steps.
    needs_mock_server: bool,
}

#[allow(clippy::too_many_arguments)]
fn render_build_zig(
    test_filenames: &[String],
    pkg_name: &str,
    module_name: &str,
    ffi_lib_name: &str,
    ffi_crate_path: &str,
    flags: ZigBuildFlags,
    test_documents_path: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    use_platform_registry_deps: bool,
) -> String {
    let ZigBuildFlags {
        has_file_fixtures,
        needs_mock_server,
    } = flags;
    if test_filenames.is_empty() {
        return match dep_mode {
            crate::e2e::config::DependencyMode::Registry => {
                if !use_platform_registry_deps {
                    return format!(
                        r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const {module_name}_module = b.dependency("{pkg_name}", .{{
        .target = target,
        .optimize = optimize,
    }}).module("{module_name}");

    const test_step = b.step("test", "Run tests");
}}
"#
                    );
                }
                format!(
                    r#"const std = @import("std");

pub fn build(b: *std.Build) void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    // Fetch the published Zig package from the registry (multi-target lazy dependency).
    const target_os = target.result.os.tag;
    const target_arch = target.result.cpu.arch;

    const {pkg_name}_dep_name = if (target_os == .linux and target_arch == .x86_64)
        "{pkg_name}_linux_x86_64"
    else if (target_os == .linux and target_arch == .aarch64)
        "{pkg_name}_linux_aarch64"
    else if (target_os == .macos and target_arch == .aarch64)
        "{pkg_name}_macos_arm64"
    else if (target_os == .macos and target_arch == .x86_64)
        "{pkg_name}_macos_x86_64"
    else if (target_os == .windows and target_arch == .x86_64)
        "{pkg_name}_windows_x86_64"
    else
        @compileError("unsupported target: " ++ target.result.cpu.arch.genericName() ++ " on " ++ @tagName(target_os));

    const {module_name}_module = (b.lazyDependency({pkg_name}_dep_name, .{{
        .target = target,
        .optimize = optimize,
    }}) orelse return).module("{module_name}");

    const test_step = b.step("test", "Run tests");
}}
"#
                )
            }
            crate::e2e::config::DependencyMode::Local => r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run tests");
}
"#
            .to_string(),
        };
    }

    // The Zig build script wires up three names that all derive from the
    // crate config:
    //   * `ffi_lib_name`     — the dynamic library to link (e.g. `mylib_ffi`).
    //   * `pkg_name`         — the Zig package directory and source file stem
    //                          under `packages/zig/src/<pkg_name>.zig`.
    //   * `module_name`      — the Zig `@import("...")` identifier other test
    //                          files use to import the binding module.
    // Callers pass these in resolved form so this function never embeds a
    // binding crate's name.
    let mut content = String::from(
        "const std = @import(\"std\");\nconst builtin = @import(\"builtin\");\n\npub fn build(b: *std.Build) void {\n",
    );
    content.push_str("    const target = b.standardTargetOptions(.{});\n");
    content.push_str("    const optimize = b.standardOptimizeOption(.{});\n");
    content.push_str("    const test_step = b.step(\"test\", \"Run tests\");\n");
    match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            if !use_platform_registry_deps {
                content.push_str("\n    // Fetch the published Zig package from the registry.\n");
                let _ = writeln!(content, "    const {pkg_name}_dep = b.dependency(\"{pkg_name}\", .{{");
                content.push_str("        .target = target,\n");
                content.push_str("        .optimize = optimize,\n");
                let _ = writeln!(content, "    }});");
                let _ = writeln!(
                    content,
                    "    const {module_name}_module = {pkg_name}_dep.module(\"{module_name}\");"
                );
                let _ = writeln!(content, "    const {pkg_name}_lib_path = {pkg_name}_dep.path(\"lib\");");
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_include_path = {pkg_name}_dep.path(\"include\");"
                );
                let _ = writeln!(content, "    {module_name}_module.addLibraryPath({pkg_name}_lib_path);");
                let _ = writeln!(
                    content,
                    "    {module_name}_module.addIncludePath({pkg_name}_include_path);"
                );
                let _ = writeln!(
                    content,
                    "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
                );
                let _ = writeln!(content);
            } else {
                // Registry mode with per-platform assets: use multi-target lazy dependencies (Zig 0.13+).
                // Each platform variant is declared with .lazy = true so Zig only fetches
                // the one matching this build's target triple. The build script selects the
                // right dependency name based on the target via `b.lazyDependency(name, .{})`.
                content.push_str(
                    "\n    // Fetch the published Zig package from the registry (multi-target lazy dependency).\n",
                );
                content.push_str("    // Select the appropriate platform variant based on the target triple.\n");
                content.push_str("    const target_os = target.result.os.tag;\n");
                content.push_str("    const target_arch = target.result.cpu.arch;\n");
                content.push('\n');
                content.push_str(&format!(
                    "    const {pkg_name}_dep_name = if (target_os == .linux and target_arch == .x86_64)\n"
                ));
                content.push_str(&format!("        \"{pkg_name}_linux_x86_64\"\n"));
                content.push_str("    else if (target_os == .linux and target_arch == .aarch64)\n");
                content.push_str(&format!("        \"{pkg_name}_linux_aarch64\"\n"));
                content.push_str("    else if (target_os == .macos and target_arch == .aarch64)\n");
                content.push_str(&format!("        \"{pkg_name}_macos_arm64\"\n"));
                content.push_str("    else if (target_os == .macos and target_arch == .x86_64)\n");
                content.push_str(&format!("        \"{pkg_name}_macos_x86_64\"\n"));
                content.push_str("    else if (target_os == .windows and target_arch == .x86_64)\n");
                content.push_str(&format!("        \"{pkg_name}_windows_x86_64\"\n"));
                content.push_str("    else\n");
                content.push_str("        @compileError(\"unsupported target: \" ++ target.result.cpu.arch.genericName() ++ \" on \" ++ @tagName(target_os));\n");
                content.push('\n');
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_dep = b.lazyDependency({pkg_name}_dep_name, .{{"
                );
                content.push_str("        .target = target,\n");
                content.push_str("        .optimize = optimize,\n");
                let _ = writeln!(content, "    }}) orelse return;");
                let _ = writeln!(
                    content,
                    "    const {module_name}_module = {pkg_name}_dep.module(\"{module_name}\");"
                );
                // Conditionally link FFI from the fetched package's bundled lib/include.
                // If the fetched package's build.zig is the new distributable version,
                // it already exports a module with FFI linked, and these lines are
                // redundant but harmless. If the fetched package's build.zig is an old
                // development version (still references ../../target/release), these
                // lines ensure FFI linking works from the tarball's own lib/ directory.
                let _ = writeln!(content, "    const {pkg_name}_lib_path = {pkg_name}_dep.path(\"lib\");");
                let _ = writeln!(
                    content,
                    "    const {pkg_name}_include_path = {pkg_name}_dep.path(\"include\");"
                );
                let _ = writeln!(content, "    {module_name}_module.addLibraryPath({pkg_name}_lib_path);");
                let _ = writeln!(
                    content,
                    "    {module_name}_module.addIncludePath({pkg_name}_include_path);"
                );
                let _ = writeln!(
                    content,
                    "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
                );
                let _ = writeln!(content);
            }
        }
        crate::e2e::config::DependencyMode::Local => {
            let _ = writeln!(
                content,
                "    const ffi_path = b.option([]const u8, \"ffi_path\", \"Path to directory containing lib{ffi_lib_name}\") orelse \"../../target/release\";"
            );
            let _ = writeln!(
                content,
                "    const ffi_include = b.option([]const u8, \"ffi_include_path\", \"Path to directory containing FFI header\") orelse \"{ffi_crate_path}/include\";"
            );
            let _ = writeln!(content);
            let _ = writeln!(
                content,
                "    const {module_name}_module = b.addModule(\"{module_name}\", .{{"
            );
            let _ = writeln!(
                content,
                "        .root_source_file = b.path(\"../../packages/zig/src/{module_name}.zig\"),"
            );
            content.push_str("        .target = target,\n");
            content.push_str("        .optimize = optimize,\n");
            // Zig 0.16 requires explicit libc linking for any module that transitively
            // references stdlib C bindings (e.g. `c.getenv` via std.posix). The shared
            // binding module pulls in the FFI header, so libc is always required.
            content.push_str("        .link_libc = true,\n");
            content.push_str("    });\n");
            let _ = writeln!(
                content,
                "    {module_name}_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});"
            );
            let _ = writeln!(
                content,
                "    {module_name}_module.addIncludePath(.{{ .cwd_relative = ffi_include }});"
            );
            let _ = writeln!(
                content,
                "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
            );
            let _ = writeln!(content);
        }
    }

    // Spawn the mock-server at configure time and capture its ephemeral URL so
    // every test run step can read it via `MOCK_SERVER_URL`. Zig has no
    // test-suite init hook (unlike Go's TestMain or the Python conftest), so the
    // build script itself owns the server's lifetime: it lives as long as the
    // `zig build` process, which spans test execution. A pre-set
    // `MOCK_SERVER_URL` (external CI orchestration) short-circuits the spawn.
    if needs_mock_server {
        content.push_str(render_zig_mock_server_spawn());
        let _ = writeln!(content);
    }

    for filename in test_filenames {
        // Convert filename like "basic_test.zig" to a test name
        let test_name = filename.trim_end_matches("_test.zig");
        content.push_str(&format!("    const {test_name}_module = b.createModule(.{{\n"));
        content.push_str(&format!("        .root_source_file = b.path(\"src/{filename}\"),\n"));
        content.push_str("        .target = target,\n");
        content.push_str("        .optimize = optimize,\n");
        // Each test module also needs libc linking because it imports the binding
        // module (which references C stdlib symbols) and may directly call helpers
        // like `std.c.getenv` for env-var-driven mock-server URLs.
        content.push_str("        .link_libc = true,\n");
        content.push_str("    });\n");
        content.push_str(&format!(
            "    {test_name}_module.addImport(\"{module_name}\", {module_name}_module);\n"
        ));
        // Zig 0.16: addTest hashes its output binary path off the artifact `.name`.
        // Without an explicit name, every addTest call defaults to "test", colliding
        // in the cache — only one binary survives, every other addRunArtifact fails
        // with FileNotFound at its computed path. Setting a unique name per test
        // module produces a distinct .zig-cache/o/<hash>/<name> binary for each.
        //
        // Zig 0.16 ALSO defaults to the self-hosted backend on aarch64-linux for
        // Debug builds. That backend emits the test binary at a different cache
        // path (or with different permissions) than the build system's RunStep
        // computes when reading `getEmittedBin()`, so every `addRunArtifact` call
        // fails with `FileNotFound` at `.zig-cache/o/<hash>/<name>` even though
        // the compile step reports success. Forcing `.use_llvm = true` pins the
        // LLVM backend, which keeps the emitted binary at the path the RunStep
        // expects. Other Zig backends (x86_64 macOS/Linux) already default to
        // LLVM, so this is a no-op there.
        content.push_str(&format!("    const {test_name}_tests = b.addTest(.{{\n"));
        content.push_str(&format!("        .name = \"{test_name}_test\",\n"));
        content.push_str(&format!("        .root_module = {test_name}_module,\n"));
        content.push_str("        .use_llvm = true,\n");
        content.push_str("    });\n");
        // Run the test binary via `addRunArtifact`. When any fixture reads
        // files from `test_documents/` (arg type `file_path` or `bytes`),
        // also point the working directory at the repo-root `test_documents/`
        // so that `std.Io.Dir.cwd().readFileAlloc(...)` resolves paths like
        // `pdf/fake_memo.pdf` correctly. Other languages perform this chdir
        // in a per-suite hook (Go `TestMain`, Python conftest, Kotlin Gradle
        // `workingDir`); Zig has no equivalent test-suite init hook, so it
        // must happen at the build-step level.
        //
        // IMPORTANT: `setCwd` is only emitted when `has_file_fixtures` is
        // true. For consumers whose fixtures are mock-server-only, there is
        // no `test_documents/` directory. Zig's
        // RunStep chdirs into the path before execing the test binary; if
        // the directory does not exist, `chdir(2)` returns ENOENT and the
        // spawn fails with `FileNotFound` — even though the binary itself
        // was compiled successfully and exists in the zig cache.
        content.push_str(&format!(
            "    const {test_name}_run = b.addRunArtifact({test_name}_tests);\n"
        ));
        if has_file_fixtures {
            content.push_str(&format!(
                "    {test_name}_run.setCwd(b.path(\"{test_documents_path}\"));\n"
            ));
        }
        if needs_mock_server {
            // Forward the captured mock-server URL into the test binary's
            // environment so `std.c.getenv(\"MOCK_SERVER_URL\")` resolves to the
            // live ephemeral address.
            content.push_str("    if (mock_server_url) |_url| {\n");
            content.push_str(&format!(
                "        {test_name}_run.setEnvironmentVariable(\"MOCK_SERVER_URL\", _url);\n"
            ));
            content.push_str("    }\n");
            content.push_str("    if (mock_servers_json) |_json| {\n");
            content.push_str(&format!(
                "        {test_name}_run.setEnvironmentVariable(\"MOCK_SERVERS\", _json);\n"
            ));
            content.push_str("    }\n");
            content.push_str("    {\n");
            content.push_str("        var _it = mock_servers_map.iterator();\n");
            content.push_str("        while (_it.next()) |_entry| {\n");
            content.push_str(&format!(
                "            {test_name}_run.setEnvironmentVariable(_entry.key_ptr.*, _entry.value_ptr.*);\n"
            ));
            content.push_str("        }\n");
            content.push_str("    }\n");
        }
        content.push_str(&format!("    test_step.dependOn(&{test_name}_run.step);\n\n"));
    }

    content.push_str("}\n");
    content
}

/// Emit the `build.zig` block that spawns the standalone mock-server binary at
/// configure time and captures its URL.
///
/// The mock-server binds an ephemeral `127.0.0.1` port and prints
/// `MOCK_SERVER_URL=http://127.0.0.1:<port>` (plus an optional
/// `MOCK_SERVERS={...}` JSON line for host-root fixtures) on stdout once it is
/// listening. The block produces three bindings consumed by the test run steps:
///   * `mock_server_url: ?[]const u8` — the base URL, or `null` when no binary
///     was found and no preset env var was supplied.
///   * `mock_servers_json: ?[]const u8` — the raw `MOCK_SERVERS=` JSON payload.
///   * `mock_servers_map: std.StringHashMap([]const u8)` — `MOCK_SERVER_<ID>`
///     env-var name → per-fixture URL, for host-root fixtures.
///
/// The spawned child is intentionally not awaited: it lives for the duration of
/// the `zig build` process, which spans test execution. A pre-set
/// `MOCK_SERVER_URL` short-circuits the spawn. Targets Zig 0.16 std APIs.
fn render_zig_mock_server_spawn() -> &'static str {
    r#"    const _alloc = b.allocator;
    var mock_server_url: ?[]const u8 = b.graph.environ_map.get("MOCK_SERVER_URL");
    var mock_servers_json: ?[]const u8 = null;
    var mock_servers_map = std.StringHashMap([]const u8).init(_alloc);
    if (mock_server_url == null) {
        const _bin = b.pathFromRoot("../rust/target/release/mock-server");
        const _fixtures = b.pathFromRoot("../../fixtures");
        var _threaded = std.Io.Threaded.init(_alloc, .{});
        const _io = _threaded.io();
        const _spawned = std.process.spawn(_io, .{
            .argv = &.{ _bin, _fixtures },
            .stdin = .pipe,
            .stdout = .pipe,
            .stderr = .inherit,
        });
        if (_spawned) |_child| {
            // The child is intentionally not awaited: it lives for the duration
            // of the `zig build` process, which spans test execution.
            const _stdout = _child.stdout.?;
            var _buf: [65536]u8 = undefined;
            var _file_reader = _stdout.readerStreaming(_io, &_buf);
            const _r = &_file_reader.interface;
            // Read startup lines: MOCK_SERVER_URL= then MOCK_SERVERS= (always
            // emitted, possibly `{}`). Cap the loop so a misbehaving server
            // cannot block the build indefinitely.
            var _saw_url = false;
            var _i: usize = 0;
            while (_i < 64) : (_i += 1) {
                const _line_raw = _r.takeDelimiterExclusive('\n') catch break;
                const _line = std.mem.trim(u8, _line_raw, " \r\t");
                if (std.mem.startsWith(u8, _line, "MOCK_SERVER_URL=")) {
                    mock_server_url = _alloc.dupe(u8, _line["MOCK_SERVER_URL=".len..]) catch null;
                    _saw_url = true;
                } else if (std.mem.startsWith(u8, _line, "MOCK_SERVERS=")) {
                    const _json = _line["MOCK_SERVERS=".len..];
                    mock_servers_json = _alloc.dupe(u8, _json) catch null;
                    if (std.json.parseFromSlice(std.json.Value, _alloc, _json, .{})) |_parsed| {
                        if (_parsed.value == .object) {
                            var _entries = _parsed.value.object.iterator();
                            while (_entries.next()) |_entry| {
                                if (_entry.value_ptr.* == .string) {
                                    const _key = std.fmt.allocPrint(_alloc, "MOCK_SERVER_{s}", .{_entry.key_ptr.*}) catch continue;
                                    for (_key) |*_c| _c.* = std.ascii.toUpper(_c.*);
                                    const _val = _alloc.dupe(u8, _entry.value_ptr.*.string) catch continue;
                                    mock_servers_map.put(_key, _val) catch {};
                                }
                            }
                        }
                    } else |_| {}
                    break;
                } else if (_saw_url) {
                    break;
                }
            }
        } else |_| {
            // Binary not built — leave mock_server_url null so tests surface a
            // clear connection error rather than a build failure.
        }
    }
"#
}

// ---------------------------------------------------------------------------
// HTTP server test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Renderer that emits Zig `test "..." { ... }` blocks targeting a mock server
/// via `std.http.Client`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct ZigTestClientRenderer;

impl client::TestClientRenderer for ZigTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "zig"
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        if let Some(reason) = skip_reason {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
            let _ = writeln!(out, "    // skipped: {reason}");
            let _ = writeln!(out, "    return error.SkipZigTest;");
        } else {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_id = ctx.path.trim_start_matches("/fixtures/");
        // Escape curly braces in fixture_id so they don't get interpreted as format specs by bufPrint.
        let escaped_fixture_id = fixture_id.replace('{', "{{").replace('}', "}}");

        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");

        let _ = writeln!(out, "    var url_buf: [512]u8 = undefined;");
        let _ = writeln!(
            out,
            "    const url = try std.fmt.bufPrint(&url_buf, \"{{s}}/fixtures/{escaped_fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
        );

        // Headers
        if !ctx.headers.is_empty() {
            let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
            header_pairs.sort_by_key(|(k, _)| k.as_str());
            let _ = writeln!(out, "    const headers = [_]std.http.Header{{");
            for (k, v) in &header_pairs {
                let ek = escape_zig(k);
                let ev = escape_zig(v);
                let _ = writeln!(out, "        .{{ .name = \"{ek}\", .value = \"{ev}\" }},");
            }
            let _ = writeln!(out, "    }};");
        }

        let headers_arg = if ctx.headers.is_empty() { "&.{}" } else { "&headers" };
        let has_body = ctx.body.is_some();
        // zig 0.16's std.http.Client.fetch asserts in `sendBodilessUnflushed` when a
        // body-requiring method (POST/PUT/PATCH) is sent without a `.payload`. The mock server
        // replays by fixture id and ignores the request body, so emit an empty payload for such
        // methods when the fixture itself carries no body, avoiding the `reached unreachable` panic.
        let method_requires_body = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
        let emit_payload = has_body || method_requires_body;

        // Body
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_zig(&json_str);
            let _ = writeln!(out, "    const body_bytes: []const u8 = \"{escaped}\";");
        } else if emit_payload {
            let _ = writeln!(out, "    const body_bytes: []const u8 = \"\";");
        }

        // zig 0.16: std.http.Client requires an `io: Io` (the new std.Io abstraction), and
        // the response body is captured through a std.Io.Writer rather than the removed
        // `response_storage`/ArrayList API. A blocking `Io.Threaded` instance backs the client.
        let _ = writeln!(out, "    var threaded = std.Io.Threaded.init(allocator, .{{}});");
        let _ = writeln!(out, "    defer threaded.deinit();");
        let _ = writeln!(out, "    const io = threaded.io();");
        let _ = writeln!(
            out,
            "    var http_client = std.http.Client{{ .allocator = allocator, .io = io }};"
        );
        let _ = writeln!(out, "    defer http_client.deinit();");
        let _ = writeln!(out, "    var response_body = std.Io.Writer.Allocating.init(allocator);");
        let _ = writeln!(out, "    defer response_body.deinit();");

        let method_zig = match method.as_str() {
            "GET" => ".GET",
            "POST" => ".POST",
            "PUT" => ".PUT",
            "DELETE" => ".DELETE",
            "PATCH" => ".PATCH",
            "HEAD" => ".HEAD",
            "OPTIONS" => ".OPTIONS",
            _ => ".GET",
        };

        let payload_field = if emit_payload { ", .payload = body_bytes" } else { "" };
        // `.keep_alive = false` sends `Connection: close` so the server closes the socket after
        // the response. Without it, the std.http.Client blocks reading a kept-alive connection
        // waiting for data/EOF that never arrives — under the e2e load this deadlocks the test
        // binaries (0% CPU, hundreds of lingering connections). Each test uses a fresh client,
        // so there is no keep-alive reuse benefit to preserve.
        let _ = writeln!(
            out,
            "    const {rv} = try http_client.fetch(.{{ .location = .{{ .url = url }}, .method = {method_zig}, .extra_headers = {headers_arg}{payload_field}, .keep_alive = false, .redirect_behavior = .unhandled, .response_writer = &response_body.writer }});",
            rv = ctx.response_var,
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "    try testing.expectEqual(@as(u10, {status}), @intFromEnum({response_var}.status));"
        );
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let ename = escape_zig(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is present (header inspection not yet implemented)"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is absent (header inspection not yet implemented)"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' matches UUID pattern (header inspection not yet implemented)"
                );
            }
            exact => {
                let evalue = escape_zig(exact);
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' == \"{evalue}\" (header inspection not yet implemented)"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        // A string-valued expected body is a plain-text response (e.g. `text/plain` "foo bar 10"),
        // so compare the raw string contents — JSON-serializing it would wrap it in quotes and
        // never match the unquoted response bytes. Structured bodies keep their serialized form.
        let escaped = match expected {
            serde_json::Value::String(s) => escape_zig(s),
            other => escape_zig(&serde_json::to_string(other).unwrap_or_default()),
        };
        let _ = writeln!(
            out,
            "    try testing.expectEqualStrings(\"{escaped}\", response_body.written());"
        );
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let ekey = escape_zig(key);
                let eval = escape_zig(&serde_json::to_string(val).unwrap_or_default());
                let _ = writeln!(
                    out,
                    "    // assert body contains field \"{ekey}\" = \"{eval}\" (partial JSON not yet implemented)"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[crate::e2e::fixture::ValidationErrorExpectation],
    ) {
        for ve in errors {
            let loc = ve.loc.join(".");
            let escaped_loc = escape_zig(&loc);
            let escaped_msg = escape_zig(&ve.msg);
            let _ = writeln!(
                out,
                "    // assert validation error at \"{escaped_loc}\": \"{escaped_msg}\" (not yet implemented)"
            );
        }
    }
}

/// Render a Zig `test "..." { ... }` block for an HTTP server fixture.
///
/// Delegates to the shared [`client::http_call::render_http_test`] driver via
/// [`ZigTestClientRenderer`].
fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &ZigTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "const std = @import(\"std\");");
    let _ = writeln!(out, "const testing = std.testing;");
    let _ = writeln!(out, "const {module_name} = @import(\"{module_name}\");");
    let _ = writeln!(out);

    // Suppress C++ static destructors that may abort during exit (e.g., leptonica's ObjectCache cleanup).
    // The Zig test runner's --listen=- IPC protocol expects a clean exit, but C++ cleanup can trigger
    // SIGABRT. Using SIG_IGN (signal number 1) ignores SIGABRT entirely, allowing normal exit.
    let _ = writeln!(
        out,
        "// Suppress C++ global destructor aborts that break zig's --listen=- IPC"
    );
    let _ = writeln!(out, "extern \"c\" fn signal(sig: i32, handler: usize) usize;");
    let _ = writeln!(out, "var _abort_handler_installed: bool = false;");
    let _ = writeln!(out, "fn suppress_abort() void {{");
    let _ = writeln!(out, "    if (!_abort_handler_installed) {{");
    let _ = writeln!(out, "        // SIGABRT = 6 on POSIX; SIG_IGN = 1");
    let _ = writeln!(out, "        _ = signal(6, 1);");
    let _ = writeln!(out, "        _abort_handler_installed = true;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.http.is_some() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_fn(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                module_name,
                ffi_prefix,
                config,
                type_defs,
            );
        }
        let _ = writeln!(out);
    }

    out
}

#[derive(Debug, Clone)]
struct ZigStreamingAdapterMetadata {
    owner_type: String,
    item_type: String,
    request_type: String,
    adapter_name: String,
}

fn resolve_zig_streaming_adapter(
    config: &ResolvedCrateConfig,
    function_name: &str,
) -> Option<ZigStreamingAdapterMetadata> {
    config
        .adapters
        .iter()
        .find(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.name == function_name)
        .and_then(|adapter| {
            Some(ZigStreamingAdapterMetadata {
                owner_type: adapter.owner_type.clone()?,
                item_type: adapter.item_type.clone()?,
                request_type: adapter
                    .request_type
                    .as_deref()
                    .and_then(|path| path.rsplit("::").next())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)?,
                adapter_name: adapter.name.clone(),
            })
        })
}

#[allow(clippy::too_many_arguments)]
fn render_test_fn(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    module_name: &str,
    ffi_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
    );
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;
    // Client factory: when set, the test instantiates a client object via
    // `module.factory_fn(...)` and calls methods on the instance rather than
    // calling top-level package functions directly.
    // Mirrors the go codegen pattern (go.rs:981-1028 / CallOverride.client_factory).
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // When `result_is_json_struct = true`, the Zig function returns `[]u8` JSON.
    // The test parses it with `std.json.parseFromSlice(std.json.Value, ...)` and
    // traverses the dynamic JSON object for field assertions.
    //
    // Client-factory methods on opaque handles always return JSON `[]u8` because
    // the zig backend serializes struct results via the FFI's `*_to_json` helper
    // (see alef-backend-zig/src/gen_bindings/opaque_handles.rs). Force the flag
    // on whenever a client_factory is in play so the test path parses the JSON
    // result rather than attempting direct field access on `[]u8`.
    //
    // Exception: when the call returns raw bytes (e.g. speech/file_content use the
    // FFI byte-buffer out-pointer shape and return `[]u8` audio/file bytes rather
    // than a serialised struct). Detect this by checking the call-level flag first
    // and then falling back to any per-language override that declares `result_is_bytes`.
    // The zig and C bindings share the same byte-buffer convention, so a C override
    // of `result_is_bytes = true` is a reliable proxy when no zig override exists.
    let call_result_is_bytes = call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_json_struct =
        !call_result_is_bytes && (call_overrides.is_some_and(|o| o.result_is_json_struct) || client_factory.is_some());

    // Whether the bare wrapper return type is `?T` (Optional). The zig backend
    // emits `?[]u8` for nullable JSON results and `?<Primitive>` for nullable
    // primitives, so assertions on the bare result must use null-checks rather
    // than `.len`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and fall back to the per-call language override for
    // backwards compatibility.
    let result_is_simple = call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple);

    // Whether the Zig wrapper returns an error union (`try` is required).
    //
    // The Zig backend nearly always returns an error union: any function with
    // string/path/json_object/bytes parameters must allocate a null-terminated
    // copy (→ `error{OutOfMemory}!T`), any fallible function (`returns_result`)
    // wraps a `DomainError||error{OutOfMemory}!T`, and any function whose return
    // type is a string/JSON/collection blob also needs heap allocation.
    //
    // The ONLY case where `try` is incorrect is a function that is:
    //   - genuinely infallible (no Rust Result<T,E>)
    //   - takes no allocating parameters (no string/path/bytes/json_object args)
    //   - returns a primitive directly (u64, bool, etc.)
    //
    // Rather than attempting to infer this from incomplete config information,
    // we default to emitting `try` and require an explicit opt-out:
    //
    //   [crates.e2e.calls.language_count.overrides.zig]
    //   returns_result = false
    //
    // Special case: functions named `unregister_*` always return error unions
    // (plugin trait unregister calls) and must always use `try`, regardless
    // of the `returns_result` override.
    //
    // This is safer than guessing wrong and producing un-compilable Zig.
    let call_returns_error_union =
        function_name.starts_with("unregister_") || call_overrides.and_then(|o| o.returns_result) != Some(false);

    let test_name = fixture.id.to_snake_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str, setup_needs_gpa) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        module_name,
        config,
        type_defs,
        fixture,
    );
    // Append per-call zig extra_args (e.g. `["null"]` for the trailing
    // optional `query` parameter on `list_files` / `list_batches`). Mirrors
    // the same mechanism used by go/python/swift codegen — zig's method
    // signatures require every optional positional argument to be supplied
    // explicitly, so the e2e config carries a per-language extras list.
    let extra_args = recipe.extra_args;
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // Pre-compute whether any assertion will emit code that references `result` /
    // `allocator`. Used to decide whether to emit the GPA allocator binding.
    let any_happy_emits_code = fixture
        .assertions
        .iter()
        .any(|a| assertion_emits_code(a, field_resolver));
    let any_non_error_emits_code = fixture
        .assertions
        .iter()
        .filter(|a| a.assertion_type != "error")
        .any(|a| assertion_emits_code(a, field_resolver));

    // Pre-compute streaming-virtual path conditions.
    let has_streaming_virtual_assertions = fixture.assertions.iter().any(|a| {
        a.field
            .as_ref()
            .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
    });
    let is_stream_fn = function_name.contains("stream");
    let streaming_adapter = if has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some() {
        resolve_zig_streaming_adapter(config, &function_name)
    } else {
        None
    };
    let uses_streaming_virtual_path =
        result_is_json_struct && has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some();
    // Whether the streaming-virtual path also parses JSON (for non-streaming assertions).
    let streaming_path_has_non_streaming = uses_streaming_virtual_path
        && fixture.assertions.iter().any(|a| {
            !a.field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
                && !matches!(a.assertion_type.as_str(), "not_error" | "error")
                && a.field
                    .as_ref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_valid_for_result(f))
        });

    let _ = writeln!(out, "test \"{test_name}\" {{");
    let _ = writeln!(out, "    // {description}");
    let _ = writeln!(out, "    suppress_abort();");

    // Visitor fixtures bypass the high-level `convert(html, options)` wrapper
    // and inline the FFI sequence so we can attach the generated visitor callbacks
    // vtable to the options handle. The vtable is populated by per-fixture
    // C-callable thunks emitted by `zig_visitors::build_zig_visitor`.
    if let Some(visitor_spec) = &fixture.visitor {
        let html = fixture.input.get("html").and_then(|v| v.as_str()).unwrap_or_default();
        let options_value = fixture.input.get("options").cloned();
        let visitor_symbols = resolve_zig_visitor_call_symbols(call_config, &recipe, ffi_prefix);
        emit_visitor_test_body(
            out,
            &fixture.id,
            html,
            options_value.as_ref(),
            visitor_spec,
            module_name,
            &visitor_symbols,
            &fixture.assertions,
            expects_error,
            field_resolver,
        );
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        return;
    }

    // Emit GPA allocator only when it will actually be used: setup lines that
    // need GPA allocation (mock_url), or a JSON-struct result path where the test
    // will call `std.json.parseFromSlice`. The binding is not needed for
    // error-only paths or tests with no field assertions.
    // Note: `bytes` arg setup uses c_allocator directly and does NOT require GPA.
    // For the streaming-virtual path, `allocator` is only needed if there are also
    // non-streaming assertions that require JSON parsing via parseFromSlice.
    let needs_gpa = setup_needs_gpa
        || streaming_path_has_non_streaming
        || (!uses_streaming_virtual_path && result_is_json_struct && !expects_error && any_happy_emits_code)
        || (!uses_streaming_virtual_path && result_is_json_struct && expects_error && any_non_error_emits_code);
    if needs_gpa {
        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");
        let _ = writeln!(out);
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    // Client factory: when configured, instantiate a client object via the named
    // constructor function and call the method on the instance.
    // The client is pointed at MOCK_SERVER_URL/fixtures/<id> (mirrors go.rs:981-1028).
    // When not configured, fall back to calling the top-level package function directly.
    let call_prefix = if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "    const _mock_url = try std.fmt.allocPrintSentinel(std.heap.c_allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}}, 0);"
        );
        let _ = writeln!(out, "    defer std.heap.c_allocator.free(_mock_url);");
        let _ = writeln!(
            out,
            "    var _client = try {module_name}.{factory}(\"test-key\", _mock_url, null, null, null);"
        );
        let _ = writeln!(out, "    defer _client.free();");
        "_client".to_string()
    } else {
        module_name.to_string()
    };

    if expects_error {
        // Error-path test: use error union syntax `!T` and try-catch.
        // Async functions execute via tokio::runtime::block_on in the FFI shim,
        // so the call site is synchronous from Zig's perspective.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        } else {
            let _ = writeln!(
                out,
                "    const result = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        }
        let _ = writeln!(out, "        try testing.expect(true); // Error occurred as expected");
        let _ = writeln!(out, "        return;");
        let _ = writeln!(out, "    }};");
        // Whether any non-error assertion will emit code that references `result`.
        // If not, we must explicitly discard `result` to satisfy Zig's
        // strict-unused-locals rule.
        let any_emits_code = fixture
            .assertions
            .iter()
            .filter(|a| a.assertion_type != "error")
            .any(|a| assertion_emits_code(a, field_resolver));
        if result_is_json_struct && any_emits_code {
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let _ = writeln!(
                out,
                "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
            );
            let _ = writeln!(out, "    defer _parsed.deinit();");
            let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_json_assertion(out, assertion, result_var, field_resolver, false);
                }
            }
        } else if result_is_json_struct {
            let _ = writeln!(out, "    _ = _result_json;");
        } else if any_emits_code {
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_assertion(
                        out,
                        assertion,
                        result_var,
                        field_resolver,
                        enum_fields,
                        result_is_option,
                        result_is_simple,
                    );
                }
            }
        } else {
            let _ = writeln!(out, "    _ = result;");
        }
    } else if fixture.assertions.is_empty() {
        // No assertions: emit a call to verify compilation.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    } else {
        // Happy path: call and assert. Detect whether any assertion actually
        // emits code that references `result` (some — like `not_error` — emit
        // nothing) so we don't leave an unused local, which Zig 0.16 rejects.
        let any_emits_code = fixture
            .assertions
            .iter()
            .any(|a| assertion_emits_code(a, field_resolver));
        if call_result_is_bytes && client_factory.is_some() {
            // Bytes path: the function returns raw `[]u8` (audio/file bytes), not
            // a JSON struct. Call, defer-free, then check len for not_empty/is_empty.
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let has_bytes_assertions = fixture
                .assertions
                .iter()
                .any(|a| matches!(a.assertion_type.as_str(), "not_empty" | "is_empty"));
            if has_bytes_assertions {
                for assertion in &fixture.assertions {
                    match assertion.assertion_type.as_str() {
                        "not_empty" => {
                            let _ = writeln!(out, "    try testing.expect(_result_json.len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), _result_json.len);");
                        }
                        "not_error" | "error" => {}
                        _ => {
                            let atype = &assertion.assertion_type;
                            let _ = writeln!(
                                out,
                                "    // bytes result: assertion '{atype}' not implemented for zig bytes"
                            );
                        }
                    }
                }
            }
        } else if result_is_json_struct {
            // When streaming-virtual field assertions are present (pre-computed above),
            // emit raw FFI code to collect all chunks instead of calling
            // the high-level streaming wrapper (which only returns the last chunk's JSON).
            if uses_streaming_virtual_path {
                let Some(streaming_adapter) = streaming_adapter.as_ref() else {
                    let _ = writeln!(
                        out,
                        "    // skipped: streaming fixture requires matching [[crates.adapters]] metadata for zig e2e codegen"
                    );
                    let _ = writeln!(out, "    return error.SkipZigTest;");
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    return;
                };
                let owner_snake = streaming_adapter.owner_type.to_snake_case();
                let request_snake = streaming_adapter.request_type.to_snake_case();
                let request_from_json = format!("{ffi_prefix}_{request_snake}_from_json");
                let request_free = format!("{ffi_prefix}_{request_snake}_free");
                let stream_start = format!("{ffi_prefix}_{owner_snake}_{}_start", streaming_adapter.adapter_name);
                let stream_free = format!("{ffi_prefix}_{owner_snake}_{}_free", streaming_adapter.adapter_name);
                let client_c_type = format!("{}{}", ffi_prefix.to_shouty_snake_case(), streaming_adapter.owner_type);

                // Streaming-virtual path: inline FFI collect.
                // Build a sentinel-terminated request string.
                let _ = writeln!(
                    out,
                    "    const _req_z = try std.heap.c_allocator.dupeZ(u8, {args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_req_z);");
                let _ = writeln!(
                    out,
                    "    const _req_handle = {module_name}.c.{request_from_json}(_req_z.ptr);"
                );
                let _ = writeln!(out, "    defer {module_name}.c.{request_free}(_req_handle);");
                let _ = writeln!(
                    out,
                    "    const _stream_handle = {module_name}.c.{stream_start}(@as(*{module_name}.c.{client_c_type}, @ptrCast(_client._handle)), _req_handle);"
                );
                let _ = writeln!(out, "    if (_stream_handle == null) return error.StreamStartFailed;");
                let _ = writeln!(out, "    defer {module_name}.c.{stream_free}(_stream_handle);");
                // Emit the collect snippet (already has 4-space indentation baked in).
                let snip = StreamingFieldResolver::collect_snippet_zig(
                    "_stream_handle",
                    "chunks",
                    module_name,
                    ffi_prefix,
                    &streaming_adapter.owner_type,
                    &streaming_adapter.adapter_name,
                    &streaming_adapter.item_type,
                );
                out.push_str("    ");
                out.push_str(&snip);
                out.push('\n');
                // For non-streaming assertions (e.g. usage), we also need _result_json.
                // Re-serialize the last chunk in `chunks` to get the JSON.
                if streaming_path_has_non_streaming {
                    let _ = writeln!(
                        out,
                        "    const _result_json = if (chunks.items.len > 0) chunks.items[chunks.items.len - 1] else &[_]u8{{}};"
                    );
                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                }
                for assertion in &fixture.assertions {
                    render_json_assertion(out, assertion, result_var, field_resolver, true);
                }
            } else {
                // JSON struct path: parse result JSON and access fields dynamically.
                let _ = writeln!(
                    out,
                    "    const _result_json = try {call_prefix}.{function_name}({args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
                if any_emits_code {
                    // For certain functions like `interact()`, the result is a struct that
                    // the fixture expects to access via a wrapper field (e.g. "interaction.action_results").
                    // Since the Zig binding returns the serialized struct directly (without wrapping),
                    // we wrap it in a JSON object with the appropriate key before parsing.
                    let wrap_field = match function_name.as_str() {
                        "interact" => Some("interaction"),
                        _ => None,
                    };

                    let parse_json_var = if let Some(field) = wrap_field {
                        // Build the Zig format string for wrapping: {"field":{s}}
                        // In Zig: `std.fmt.allocPrint(..., "{\"field\":{s}}", .{value})`
                        // In Rust string literal: "{{{{\\\"field\\\":{{s}}}}}}" (each { → {{, each \ → \\)
                        let _ = writeln!(
                            out,
                            "    const _wrapped_json = try std.fmt.allocPrint(allocator, \"{{{{\\\"{}\\\":{{s}}}}}}\", .{{_result_json}});",
                            field
                        );
                        let _ = writeln!(out, "    defer allocator.free(_wrapped_json);");
                        "_wrapped_json".to_string()
                    } else {
                        "_result_json".to_string()
                    };

                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, {parse_json_var}, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                    for assertion in &fixture.assertions {
                        render_json_assertion(out, assertion, result_var, field_resolver, false);
                    }
                }
            }
        } else if any_emits_code {
            let try_kw = if call_returns_error_union { "try " } else { "" };
            let _ = writeln!(
                out,
                "    const {result_var} = {try_kw}{call_prefix}.{function_name}({args_str});"
            );
            for assertion in &fixture.assertions {
                render_assertion(
                    out,
                    assertion,
                    result_var,
                    field_resolver,
                    enum_fields,
                    result_is_option,
                    result_is_simple,
                );
            }
        } else if call_returns_error_union {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        } else {
            let _ = writeln!(out, "    _ = {call_prefix}.{function_name}({args_str});");
        }
    }

    let _ = writeln!(out, "}}");
}

struct ZigVisitorCallSymbols {
    visitor_prefix: String,
    visitor_create: String,
    visitor_free: String,
    options_from_json: String,
    options_free: String,
    options_set_visitor_handle: String,
    function_name: String,
    result_free: String,
    result_to_json: String,
    free_string: String,
    last_error_code: String,
}

fn resolve_zig_visitor_call_symbols(
    call_config: &crate::core::config::e2e::CallConfig,
    recipe: &crate::e2e::codegen::recipe::ResolvedE2eCallRecipe<'_>,
    ffi_prefix: &str,
) -> ZigVisitorCallSymbols {
    let c_override = call_config.overrides.get("c");
    let function_name = c_override
        .and_then(|override_config| override_config.function.as_ref())
        .cloned()
        .or_else(|| {
            recipe
                .override_config
                .and_then(|override_config| override_config.function.as_ref())
                .cloned()
        })
        .unwrap_or_else(|| call_config.function.clone());
    let options_type_name = c_override
        .and_then(|override_config| override_config.options_type.as_deref())
        .or(recipe.options_type)
        .unwrap_or_default()
        .to_string();
    let options_type_snake = options_type_name.to_snake_case();
    let result_type_name = c_override
        .and_then(|override_config| override_config.result_type.as_ref())
        .cloned()
        .or_else(|| {
            recipe
                .override_config
                .and_then(|override_config| override_config.result_type.as_ref())
                .cloned()
        })
        .unwrap_or_else(|| call_config.function.to_pascal_case());
    let result_type_snake = result_type_name.to_snake_case();

    ZigVisitorCallSymbols {
        visitor_prefix: ffi_prefix.to_string(),
        visitor_create: format!("{ffi_prefix}_visitor_create"),
        visitor_free: format!("{ffi_prefix}_visitor_free"),
        options_from_json: format!("{ffi_prefix}_{options_type_snake}_from_json"),
        options_free: format!("{ffi_prefix}_{options_type_snake}_free"),
        options_set_visitor_handle: format!("{ffi_prefix}_options_set_visitor_handle"),
        function_name,
        result_free: format!("{ffi_prefix}_{result_type_snake}_free"),
        result_to_json: format!("{ffi_prefix}_{result_type_snake}_to_json"),
        free_string: format!("{ffi_prefix}_free_string"),
        last_error_code: format!("{ffi_prefix}_last_error_code"),
    }
}

/// Emit the body of a visitor-bearing test. Drives the FFI directly so we
/// can attach a generated visitor callbacks vtable to the configured options
/// handle before calling the configured FFI function. The high-level wrapper
/// cannot carry a visitor because the visitor is a Rust
/// trait object, not a JSON-encodable field.
#[allow(clippy::too_many_arguments)]
fn emit_visitor_test_body(
    out: &mut String,
    fixture_id: &str,
    html: &str,
    options_value: Option<&serde_json::Value>,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    module_name: &str,
    symbols: &ZigVisitorCallSymbols,
    assertions: &[Assertion],
    expects_error: bool,
    field_resolver: &FieldResolver,
) {
    // Allocator for the JSON-parse of the result blob (and any helper allocs).
    let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
    let _ = writeln!(out, "    defer _ = gpa.deinit();");
    let _ = writeln!(out, "    const allocator = gpa.allocator();");
    let _ = writeln!(out);

    // 1. Per-fixture visitor struct + callbacks table.
    let c_prefix = symbols.visitor_prefix.to_uppercase();
    let visitor_type_stem = symbols.visitor_prefix.to_pascal_case();
    let c_types = super::zig_visitors::ZigVisitorCTypes {
        context_type: format!("{c_prefix}{visitor_type_stem}NodeContext"),
        callbacks_type: format!("{c_prefix}{visitor_type_stem}VisitorCallbacks"),
    };
    let visitor_block = super::zig_visitors::build_zig_visitor(fixture_id, module_name, visitor_spec, &c_types);
    out.push_str(&visitor_block);

    // 2. Materialise the visitor handle and attach it to the configured options handle.
    let _ = writeln!(
        out,
        "    const _visitor = {module_name}.c.{visitor_create}(&_callbacks);",
        visitor_create = symbols.visitor_create
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{visitor_free}(_visitor);",
        visitor_free = symbols.visitor_free
    );

    // 3. Options handle: always allocate one (even when the fixture supplies
    //    no `options`) so we have somewhere to attach the visitor. The FFI
    //    accepts `"{}"` as an empty options JSON.
    let options_json = match options_value {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    };
    let escaped_options = escape_zig(&options_json);
    let _ = writeln!(
        out,
        "    const _options_z = try std.heap.c_allocator.dupeZ(u8, \"{escaped_options}\");"
    );
    let _ = writeln!(out, "    defer std.heap.c_allocator.free(_options_z);");
    let _ = writeln!(
        out,
        "    const _options = {module_name}.c.{options_from_json}(_options_z.ptr);",
        options_from_json = symbols.options_from_json
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{options_free}(_options);",
        options_free = symbols.options_free
    );
    let _ = writeln!(
        out,
        "    {module_name}.c.{options_set_visitor_handle}(_options, _visitor);",
        options_set_visitor_handle = symbols.options_set_visitor_handle
    );

    // 4. HTML buffer + convert call.
    let escaped_html = escape_zig(html);
    let _ = writeln!(
        out,
        "    const _html_z = try std.heap.c_allocator.dupeZ(u8, \"{escaped_html}\");"
    );
    let _ = writeln!(out, "    defer std.heap.c_allocator.free(_html_z);");
    let _ = writeln!(
        out,
        "    const _result = {module_name}.c.{function_name}(_html_z.ptr, _options);",
        function_name = symbols.function_name
    );

    if expects_error {
        // Error-path: _result null OR last error code non-zero.
        let _ = writeln!(
            out,
            "    try testing.expect(_result == null or {module_name}.c.{last_error_code}() != 0);",
            last_error_code = symbols.last_error_code
        );
        let _ = writeln!(
            out,
            "    if (_result) |r| {module_name}.c.{result_free}(r);",
            result_free = symbols.result_free
        );
        return;
    }

    let _ = writeln!(out, "    try testing.expect(_result != null);");
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{result_free}(_result.?);",
        result_free = symbols.result_free
    );
    let _ = writeln!(
        out,
        "    const _json_ptr = {module_name}.c.{result_to_json}(_result.?);",
        result_to_json = symbols.result_to_json
    );
    let _ = writeln!(
        out,
        "    defer {module_name}.c.{free_string}(_json_ptr);",
        free_string = symbols.free_string
    );
    let _ = writeln!(out, "    const _result_json = std.mem.sliceTo(_json_ptr, 0);");
    let _ = writeln!(
        out,
        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
    );
    let _ = writeln!(out, "    defer _parsed.deinit();");
    let _ = writeln!(out, "    const result = &_parsed.value;");

    for assertion in assertions {
        if assertion.assertion_type != "error" {
            render_json_assertion(out, assertion, "result", field_resolver, false);
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-struct assertion rendering (for result_is_json_struct = true)
// ---------------------------------------------------------------------------

/// Convert a dot-separated field path into a chain of `std.json.Value` lookups.
///
/// Each segment uses `.object.get("key").?` to traverse the JSON object tree.
/// The final segment stops before the leaf-type accessor so callers can append
/// the appropriate accessor (`.string`, `.integer`, `.array.items`, etc.).
///
/// Returns `(base_expr, last_key)` where `base_expr` already includes all
/// intermediate `.object.get("…").?` dereferences up to (but not including)
/// the leaf, and `last_key` is the last path segment.
/// Variant names of `FormatMetadata` (snake_case, from `#[serde(rename_all = "snake_case")]`).
/// These appear as typed accessors in fixture paths (e.g. `format.excel.sheet_count`)
/// but are NOT JSON keys — `FormatMetadata` is internally tagged so variant fields are
/// flattened directly into the `format` object alongside the `format_type` discriminant.
const FORMAT_METADATA_VARIANTS: &[&str] = &[
    "pdf",
    "docx",
    "excel",
    "email",
    "pptx",
    "archive",
    "image",
    "xml",
    "text",
    "html",
    "ocr",
    "csv",
    "bibtex",
    "citation",
    "fiction_book",
    "dbf",
    "jats",
    "epub",
    "pst",
    "code",
];

fn json_path_expr(result_var: &str, field_path: &str) -> String {
    let segments: Vec<&str> = field_path.split('.').collect();
    let mut expr = result_var.to_string();
    let mut prev_seg: Option<&str> = None;
    for seg in &segments {
        // Skip variant-name accessor segments that follow a `format` key.
        // FormatMetadata is an internally-tagged enum (`#[serde(tag = "format_type")]`),
        // so variant fields are flattened directly into the format object — there is no
        // intermediate JSON key for the variant name.
        if prev_seg == Some("format") && FORMAT_METADATA_VARIANTS.contains(seg) {
            prev_seg = Some(seg);
            continue;
        }
        // Handle array accessor notation:
        //   "links[]"     → access the array, then first element.
        //   "results[0]"  → access the array, then specific index N.
        if let Some(key) = seg.strip_suffix("[]") {
            expr = format!("{expr}.object.get(\"{key}\").?.array.items[0]");
        } else if let Some(bracket_pos) = seg.find('[') {
            if let Some(end_pos) = seg.find(']') {
                if end_pos > bracket_pos + 1 && end_pos == seg.len() - 1 {
                    let key = &seg[..bracket_pos];
                    let idx = &seg[bracket_pos + 1..end_pos];
                    if idx.chars().all(|c| c.is_ascii_digit()) {
                        expr = format!("{expr}.object.get(\"{key}\").?.array.items[{idx}]");
                        prev_seg = Some(seg);
                        continue;
                    }
                    // Non-numeric bracket: HashMap<String, _> key access. FRB / serde
                    // serialize maps as JSON objects, so `field[key]` resolves to
                    // `.object.get("field").?.object.get("key").?`. Used by nested fixture objects.
                    // `metadata.document.open_graph[title]` alias pattern where
                    // `open_graph` is a `HashMap<String, String>`.
                    expr = format!("{expr}.object.get(\"{key}\").?.object.get(\"{idx}\").?");
                    prev_seg = Some(seg);
                    continue;
                }
            }
            expr = format!("{expr}.object.get(\"{seg}\").?");
        } else {
            expr = format!("{expr}.object.get(\"{seg}\").?");
        }
        prev_seg = Some(seg);
    }
    expr
}

/// Emit a Zig predicate over the `chunks` array of a JSON-parsed extraction
/// result. The predicate body should be a Zig expression yielding an
/// `?std.json.Value` for each chunk element bound as `c`. When `require_non_empty_string`
/// is `true`, the predicate also requires the value to be a non-empty string.
fn emit_zig_chunks_predicate(
    out: &mut String,
    result_var: &str,
    assertion_type: &str,
    chunk_field_accessor: &str,
    field_name: &str,
    require_non_empty_string: bool,
) {
    let _ = writeln!(out, "    {{");
    let _ = writeln!(out, "        const _chunks_opt = {result_var}.object.get(\"chunks\");");
    let _ = writeln!(out, "        var _all: bool = true;");
    let _ = writeln!(out, "        if (_chunks_opt) |_chunks_val| {{");
    let _ = writeln!(out, "            if (_chunks_val == .array) {{");
    let _ = writeln!(
        out,
        "                if (_chunks_val.array.items.len == 0) _all = false;"
    );
    let _ = writeln!(out, "                for (_chunks_val.array.items) |c| {{");
    let _ = writeln!(out, "                    if (c != .object) {{ _all = false; break; }}");
    let _ = writeln!(out, "                    const _v = {chunk_field_accessor};");
    if require_non_empty_string {
        let _ = writeln!(
            out,
            "                    if (_v == null or _v.? != .string or _v.?.string.len == 0) {{ _all = false; break; }}"
        );
    } else {
        let _ = writeln!(
            out,
            "                    if (_v == null or _v.? == .null) {{ _all = false; break; }}"
        );
    }
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "            }} else {{ _all = false; }}");
    let _ = writeln!(out, "        }} else {{ _all = false; }}");
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "        try testing.expect(_all);");
        }
        "is_false" => {
            let _ = writeln!(out, "        try testing.expect(!_all);");
        }
        _ => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported assertion type on synthetic field '{field_name}'"
            );
        }
    }
    let _ = writeln!(out, "    }}");
}

/// Render a single assertion for a JSON-struct result (result_is_json_struct = true).
///
/// The `result_var` variable is `*std.json.Value` (pointer to the parsed root object).
/// Field paths are traversed via `.object.get("key").?` chains.
fn render_json_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    uses_streaming: bool,
) {
    // Intercept streaming-virtual fields before the result-type validity check,
    // but ONLY when the test is actually using the streaming-virtual path.
    // When `uses_streaming = false` the `chunks` local is never declared, so
    // generating `chunks.items.len` would produce a compile error. Fields like
    // "chunks" that happen to share a streaming-virtual name are regular JSON
    // fields in non-streaming results and must fall through to the JSON path.
    if let Some(f) = &assertion.field {
        if uses_streaming && !f.is_empty() && is_streaming_virtual_field(f) {
            if let Some(expr) = StreamingFieldResolver::accessor(f, "zig", "chunks") {
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(out, "    try testing.expect({expr}.len >= {n});");
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, {n}), {expr}.len);");
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_zig(s);
                            let _ = writeln!(out, "    try testing.expectEqualStrings(\"{escaped}\", {expr});");
                        } else if let Some(v) = &assertion.value {
                            let zig_val = json_to_zig(v);
                            let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {expr});");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "    try testing.expect({expr}.len > 0);");
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    try testing.expect({expr});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    try testing.expect(!{expr});");
                    }
                    _ => {
                        let atype = &assertion.assertion_type;
                        let _ = writeln!(
                            out,
                            "    // streaming virtual field '{f}' assertion '{atype}' not implemented for zig"
                        );
                    }
                }
            }
            return;
        }
    }

    // Synthetic `embeddings` field on a JSON-array result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` → JSON `[[...],[...]]`). The field name is a
    // convention from the fixture schema — the JSON value IS the embeddings
    // array. Apply the assertion against `result.array.items` directly. The
    // synthetic path is only used when no explicit result_fields configure
    // `embeddings` as a real struct field.
    if let Some(f) = &assertion.field {
        if f == "embeddings" && !field_resolver.has_explicit_field("embeddings") {
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len >= {n});");
                    }
                    return;
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(
                            out,
                            "    try testing.expectEqual(@as(usize, {n}), {result_var}.array.items.len);"
                        );
                    }
                    return;
                }
                "not_empty" => {
                    let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len > 0);");
                    return;
                }
                "is_empty" => {
                    let _ = writeln!(
                        out,
                        "    try testing.expectEqual(@as(usize, 0), {result_var}.array.items.len);"
                    );
                    return;
                }
                _ => {}
            }
        }
    }

    // Synthesised chunk-inspection virtual fields. These are not real JSON
    // fields but are derived predicates over a result object's `chunks` array.
    // Other backends (python, ruby, java, etc.) compute
    // these inline; zig parses to `std.json.Value`, so we compute them
    // against `result.object.get("chunks").?.array`.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                emit_zig_chunks_predicate(
                    out,
                    result_var,
                    assertion.assertion_type.as_str(),
                    "c.object.get(\"content\")",
                    "chunks_have_content",
                    true,
                );
                return;
            }
            "chunks_have_heading_context" => {
                // `heading_context` is `Option<HeadingContext>` and serde drops
                // `None` from the JSON, so chunks without a heading produce no
                // key — making an "all chunks have it" predicate spuriously
                // fail. Matching the Ruby codegen, skip this synthetic field.
                let _ = writeln!(
                    out,
                    "    // skipped: synthetic field 'chunks_have_heading_context' not derivable from JSON value alone"
                );
                return;
            }
            "first_chunk_starts_with_heading" => {
                let _ = writeln!(
                    out,
                    "    // skipped: synthetic field 'first_chunk_starts_with_heading' not derivable from JSON value alone"
                );
                return;
            }
            "chunks_have_embeddings" => {
                emit_zig_chunks_predicate(
                    out,
                    result_var,
                    assertion.assertion_type.as_str(),
                    "c.object.get(\"embedding\")",
                    "chunks_have_embeddings",
                    false,
                );
                return;
            }
            // `keywords` is a fixture alias that does not map cleanly onto the
            // serialized JSON result shape. Matching the Python codegen, skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "    // skipped: field '{f}' not available on the JSON-struct result"
                );
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }
    // error/not_error are handled at the call level, not assertion level.
    if matches!(assertion.assertion_type.as_str(), "not_error" | "error") {
        return;
    }

    let raw_field_path = assertion.field.as_deref().unwrap_or("").trim();
    let field_path = if raw_field_path.is_empty() {
        raw_field_path.to_string()
    } else {
        field_resolver.resolve(raw_field_path).to_string()
    };
    let field_path = field_path.trim();

    // "{array_field}.length" → strip suffix; use .array.items.len in the template.
    let (field_path_for_expr, is_length_access) = if let Some(parent) = field_path.strip_suffix(".length") {
        (parent, true)
    } else {
        (field_path, false)
    };

    let field_expr = if field_path_for_expr.is_empty() {
        result_var.to_string()
    } else {
        json_path_expr(result_var, field_path_for_expr)
    };

    // Special-case `metadata.format` equals-string: `FormatMetadata` is an
    // internally-tagged enum serialized as a JSON object (`{"format_type": "image",
    // "format": "PNG", ...}`), so `metadata.format` resolves to a JSON object,
    // not a string. The fixture asserts the `Display` impl: for Image variant
    // emit the inner `format` field; otherwise emit the `format_type` discriminant.
    if field_path_for_expr == "metadata.format"
        && matches!(
            assertion.assertion_type.as_str(),
            "equals" | "contains" | "not_empty" | "is_empty" | "starts_with" | "ends_with"
        )
    {
        let base = json_path_expr(result_var, field_path_for_expr);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        const _fmt_obj = {base}.object;");
        let _ = writeln!(out, "        const _fmt_type = _fmt_obj.get(\"format_type\").?.string;");
        let _ = writeln!(
            out,
            "        const _fmt_display: []const u8 = if (std.mem.eql(u8, _fmt_type, \"image\")) _fmt_obj.get(\"format\").?.string else _fmt_type;"
        );
        match assertion.assertion_type.as_str() {
            "equals" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expectEqualStrings(\"{escaped}\", std.mem.trim(u8, _fmt_display, \" \\n\\r\\t\"));"
                    );
                }
            }
            "contains" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.indexOf(u8, _fmt_display, \"{escaped}\") != null);"
                    );
                }
            }
            "starts_with" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.startsWith(u8, _fmt_display, \"{escaped}\"));"
                    );
                }
            }
            "ends_with" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.endsWith(u8, _fmt_display, \"{escaped}\"));"
                    );
                }
            }
            "not_empty" => {
                let _ = writeln!(out, "        try testing.expect(_fmt_display.len > 0);");
            }
            "is_empty" => {
                let _ = writeln!(out, "        try testing.expectEqual(@as(usize, 0), _fmt_display.len);");
            }
            _ => {}
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    // Compute context variables for the template.
    let zig_val = match &assertion.value {
        Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_zig(s)),
        _ => String::new(),
    };
    let is_string_val = matches!(&assertion.value, Some(serde_json::Value::String(_)));
    let is_bool_val = matches!(&assertion.value, Some(serde_json::Value::Bool(_)));
    let bool_val = match &assertion.value {
        Some(serde_json::Value::Bool(b)) if *b => "true",
        _ => "false",
    };
    let is_null_val = matches!(&assertion.value, Some(serde_json::Value::Null));
    let n = assertion.value.as_ref().map(json_to_zig).unwrap_or_default();
    let has_n = assertion.value.as_ref().is_some_and(|v| v.is_number() || v.is_u64());
    // Distinguish float vs integer JSON values: `std.json.Value` exposes
    // `.integer` (i64) and `.float` (f64) as separate variants. Comparing
    // `.integer` against a literal with a fractional part (e.g. `0.9`) is a
    // Zig compile error, so the template must select the right tag.
    let is_float_val = matches!(&assertion.value, Some(serde_json::Value::Number(n)) if !n.is_i64() && !n.is_u64());
    let n_as_i64 = if has_n {
        format!("@as(i64, {})", n)
    } else {
        String::new()
    };
    // For usize comparisons, use i64 if n is negative (can't cast -1 to usize directly).
    // Zig comparison operators handle i64 on both sides implicitly.
    let n_as_usize = if has_n {
        if n.starts_with('-') {
            format!("@as(i64, {})", n)
        } else {
            format!("@as(usize, {})", n)
        }
    } else {
        String::new()
    };
    let n_as_f64 = if is_float_val {
        format!("@as(f64, {})", n)
    } else {
        String::new()
    };
    let values_list: Vec<String> = assertion
        .values
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| {
            if let serde_json::Value::String(s) = v {
                Some(format!("\"{}\"", escape_zig(s)))
            } else {
                None
            }
        })
        .collect();

    let rendered = crate::e2e::template_env::render(
        "zig/json_assertion.jinja",
        minijinja::context! {
            assertion_type => assertion.assertion_type.as_str(),
            field_expr => field_expr,
            is_length_access => is_length_access,
            zig_val => zig_val,
            is_string_val => is_string_val,
            is_bool_val => is_bool_val,
            bool_val => bool_val,
            is_null_val => is_null_val,
            n => n,
            n_as_i64 => n_as_i64,
            n_as_usize => n_as_usize,
            n_as_f64 => n_as_f64,
            has_n => has_n,
            is_float_val => is_float_val,
            values_list => values_list,
        },
    );
    out.push_str(&rendered);
}

/// Predicate matching `render_assertion`: returns true when the assertion
/// would emit at least one statement that references the result variable.
fn assertion_emits_code(assertion: &Assertion, field_resolver: &FieldResolver) -> bool {
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming_virtual_field(f) {
            // Streaming virtual fields always emit code — they are handled in a
            // dedicated collect path, not skipped.
        } else if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            return false;
        }
    }
    matches!(
        assertion.assertion_type.as_str(),
        "equals"
            | "contains"
            | "contains_all"
            | "not_contains"
            | "not_empty"
            | "is_empty"
            | "starts_with"
            | "ends_with"
            | "min_length"
            | "max_length"
            | "count_min"
            | "count_equals"
            | "is_true"
            | "is_false"
            | "greater_than"
            | "less_than"
            | "greater_than_or_equal"
            | "less_than_or_equal"
            | "contains_any"
    )
}

/// Build setup lines and the argument list for the function call.
///
/// Returns `(setup_lines, args_str, setup_needs_gpa)` where `setup_needs_gpa`
/// is `true` when at least one setup line requires the GPA `allocator` binding.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    _module_name: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
) -> (Vec<String>, String, bool) {
    if args.is_empty() {
        return (Vec::new(), String::new(), false);
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut setup_needs_gpa = false;

    for arg in args {
        if arg.arg_type == "mock_url" {
            let name = arg.name.clone();
            let id_upper = fixture_id.to_uppercase();
            setup_lines.push(format!(
                "const {name} = if (std.c.getenv(\"MOCK_SERVER_{id_upper}\")) |_pf| try std.fmt.allocPrint(allocator, \"{{s}}\", .{{std.mem.span(_pf)}}) else try std.fmt.allocPrint(allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
            ));
            setup_lines.push(format!("defer allocator.free({name});"));
            parts.push(name);
            setup_needs_gpa = true;
            continue;
        }

        // Handle args (engine handle): serialize config to JSON string literal, or null.
        // The Zig binding accepts ?[]const u8 for engine params (creates handle internally).
        if arg.arg_type == "handle" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "null".to_string(),
                Some(v) => format!("\"{}\"", escape_zig(&serde_json::to_string(v).unwrap_or_default())),
            };
            parts.push(json_str);
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let excluded_named =
                        crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(config, type_defs, &methods);
                    let emission = emit_test_backend_with_excluded(trait_bridge, &methods, fixture, &excluded_named);
                    // emit_test_backend uses "lib." as a placeholder; substitute the real module.
                    let setup_block = emission.setup_block.replace("lib.", &format!("{_module_name}."));
                    let arg_expr = emission.arg_expr.replace("lib.", &format!("{_module_name}."));
                    // setup_block lines already carry no indentation (the caller adds 4 spaces).
                    // Push each logical line individually so the render loop adds uniform indent.
                    for line in setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(arg_expr);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("zig");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        // The Zig wrapper accepts struct parameters
        // as JSON `[]const u8`, converting them to opaque FFI handles via the
        // `<prefix>_<snake>_from_json` helper at the binding layer. Emit the
        // fixture's configuration value as a JSON string literal, falling back
        // to `"{}"` when the fixture omits a config so callers exercise the
        // default path.
        if arg.name == "config" && arg.arg_type == "json_object" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            parts.push(format!("\"{}\"", escape_zig(&json_str)));
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        // When `field` is empty or refers to `input` itself (no dotted subfield),
        // the entire fixture `input` value is the payload — most commonly for
        // `json_object` request bodies (chat/embed/etc.). Without this guard
        // `input.get("input")` returns `None` and we fall through to `"{}"`,
        // which the FFI rejects as a deserialization error.
        let val = if field.is_empty() || field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Zig functions don't have default arguments, so we must
                // pass `null` explicitly for every optional parameter.
                parts.push("null".to_string());
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => "\"{}\"".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For `json_object` arguments other than `config` (handled
                // above) the Zig binding accepts a JSON `[]const u8`, so we
                // serialize the entire fixture value as a single JSON string
                // literal rather than rendering it as a Zig array/struct.
                if arg.arg_type == "json_object" {
                    let json_str = serde_json::to_string(v).unwrap_or_default();
                    parts.push(format!("\"{}\"", escape_zig(&json_str)));
                } else if arg.arg_type == "bytes" {
                    // `bytes` args are file paths in fixtures — read the file into a
                    // local buffer. The cwd is set to test_documents/ at runtime.
                    // Zig 0.16 uses std.Io.Dir.cwd() (not std.fs.cwd()) and requires
                    // an `io` instance from std.testing.io in test context.
                    if let serde_json::Value::String(path) = v {
                        let var_name = format!("{}_bytes", arg.name);
                        let epath = escape_zig(path);
                        setup_lines.push(format!(
                            "const {var_name} = try std.Io.Dir.cwd().readFileAlloc(std.testing.io, \"{epath}\", std.heap.c_allocator, .unlimited);"
                        ));
                        setup_lines.push(format!("defer std.heap.c_allocator.free({var_name});"));
                        parts.push(var_name);
                    } else {
                        parts.push(json_to_zig(v));
                    }
                } else {
                    parts.push(json_to_zig(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "), setup_needs_gpa)
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    result_is_option: bool,
    result_is_simple: bool,
) {
    // Bare-result assertions on `?T` (Optional) translate to null-checks instead
    // of `.len`. Mirrors the same behaviour in kotlin.rs (bare_result_is_option).
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    if bare_result_is_option {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var} == null);");
                return;
            }
            "not_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var} != null);");
                return;
            }
            "not_error" => {
                // not_error is covered by `try` propagation — the call would have
                // returned early on error. Emit a comment-only line so the assertion
                // is visible but inert, avoiding contradictory checks when paired
                // with `is_empty` on an Optional result.
                let _ = writeln!(out, "    // not_error: covered by try propagation");
                return;
            }
            "equals" => {
                if let Some(expected) = &assertion.value {
                    let zig_val = json_to_zig(expected);
                    let _ = writeln!(out, "    try testing.expectEqualStrings({zig_val}, {result_var}.?);");
                    return;
                }
            }
            _ => {}
        }
    }
    // Synthetic-field 'embeddings' on a JSON-bytes result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` serialised as JSON). Parse the JSON array and
    // apply count_min/count_equals/not_empty/is_empty against the element count.
    //
    // The Zig binding for `Vec<T>`/`result_is_array` returns `[]u8` (the JSON
    // payload), not a typed struct — so a fixture field named `embeddings` is
    // a convention for "the bare JSON array is the embeddings". Gate on
    // `has_explicit_field` rather than `is_valid_for_result`, because the
    // latter is permissive (returns true) when `result_fields` is empty —
    // which is the common case for these bare-JSON returns and would
    // wrongly route through `result.embeddings.len` direct field access on
    // a `[]u8` slice.
    if let Some(f) = &assertion.field {
        if f == "embeddings" && !field_resolver.has_explicit_field(f) {
            match assertion.assertion_type.as_str() {
                "count_min" | "count_equals" | "not_empty" | "is_empty" => {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(
                        out,
                        "        var _eparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {result_var}, .{{}});"
                    );
                    let _ = writeln!(out, "        defer _eparse.deinit();");
                    let _ = writeln!(out, "        const _embeddings_len = _eparse.value.array.items.len;");
                    match assertion.assertion_type.as_str() {
                        "count_min" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "        try testing.expect(_embeddings_len >= {n});");
                            }
                        }
                        "count_equals" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(
                                    out,
                                    "        try testing.expectEqual(@as(usize, {n}), _embeddings_len);"
                                );
                            }
                        }
                        "not_empty" => {
                            let _ = writeln!(out, "        try testing.expect(_embeddings_len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "        try testing.expectEqual(@as(usize, 0), _embeddings_len);");
                        }
                        _ => {}
                    }
                    let _ = writeln!(out, "    }}");
                    return;
                }
                _ => {}
            }
        }
    }

    // When result_is_simple, the Zig binding returns a scalar type like []u8 or ?T.
    // Skip assertions on fields that don't exist on the scalar (e.g., metadata,
    // document, structure fields).
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                let _ = writeln!(out, "    // skipped: field '{}' not available when result_is_simple", f);
                return;
            }
        }
    }

    // Synthetic-field 'result' on a bare-string/JSON-bytes return (e.g.
    // `detect_mime_type_from_bytes` returns `String` → Zig `[]u8`). The
    // fixture convention is `field: "result", contains: "pdf"` meaning the
    // bare result itself contains the substring. The Zig binding returns
    // `[]u8`, so the substring check applies directly to `result_var`.
    if let Some(f) = &assertion.field {
        if f == "result" && !field_resolver.has_explicit_field(f) {
            match assertion.assertion_type.as_str() {
                "contains" => {
                    if let Some(expected) = &assertion.value {
                        let zig_val = json_to_zig(expected);
                        let _ = writeln!(
                            out,
                            "    try testing.expect(std.mem.indexOf(u8, {result_var}, {zig_val}) != null);"
                        );
                        return;
                    }
                }
                "not_contains" => {
                    if let Some(expected) = &assertion.value {
                        let zig_val = json_to_zig(expected);
                        let _ = writeln!(
                            out,
                            "    try testing.expect(std.mem.indexOf(u8, {result_var}, {zig_val}) == null);"
                        );
                        return;
                    }
                }
                "equals" => {
                    if let Some(expected) = &assertion.value {
                        let zig_val = json_to_zig(expected);
                        let _ = writeln!(out, "    try testing.expectEqualStrings({zig_val}, {result_var});");
                        return;
                    }
                }
                "not_empty" => {
                    let _ = writeln!(out, "    try testing.expect({result_var}.len > 0);");
                    return;
                }
                "is_empty" => {
                    let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), {result_var}.len);");
                    return;
                }
                _ => {}
            }
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let _field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = match &assertion.field {
        // When result_is_simple, the result is a scalar ([]u8 or ?T, etc.) — any
        // field access on it would fail. Treat all assertions as referring to the
        // result itself.
        _ if result_is_simple => result_var.to_string(),
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "zig", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {field_expr});");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) == null);"
                );
            } else if let Some(values) = &assertion.values {
                // not_contains with a plural `values` list: assert none of the entries
                // appear in the field. Emit one expect line per needle so failures
                // pinpoint the offending value.
                for val in values {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) == null);"
                    );
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len > 0);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len == 0);");
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.startsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.endsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len <= {n});");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // When there is no field (field_expr == result_var), the result
                    // is `[]u8` JSON (e.g. batch functions). Parse the JSON array
                    // and count its elements; `.len` would give byte count, not item count.
                    let has_field = assertion.field.as_deref().is_some_and(|f| !f.is_empty());
                    if has_field {
                        let _ = writeln!(out, "    try testing.expectEqual(@as(usize, {n}), {field_expr}.len);");
                    } else {
                        let _ = writeln!(out, "    {{");
                        let _ = writeln!(
                            out,
                            "        var _cparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {field_expr}, .{{}});"
                        );
                        let _ = writeln!(out, "        defer _cparse.deinit();");
                        let _ = writeln!(
                            out,
                            "        try testing.expectEqual(@as(usize, {n}), _cparse.value.array.items.len);"
                        );
                        let _ = writeln!(out, "    }}");
                    }
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    try testing.expect({field_expr});");
        }
        "is_false" => {
            let _ = writeln!(out, "    try testing.expect(!{field_expr});");
        }
        "not_error" => {
            // Already handled by the call succeeding.
        }
        "error" => {
            // Handled at the test function level.
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons like `len > -1` when the value is negative: they are always-true
                // tautologies for unsigned types and create invalid Zig code (@as(usize, -1)).
                let is_negative = matches!(val, serde_json::Value::Number(n) if n.as_i64().is_some_and(|i| i < 0));
                if !is_negative {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(out, "    try testing.expect({field_expr} > {zig_val});");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} < {zig_val});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons like `len >= -1` when the value is negative: they are always-true
                // tautologies for unsigned types and create invalid Zig code (@as(usize, -1)).
                let is_negative = matches!(val, serde_json::Value::Number(n) if n.as_i64().is_some_and(|i| i < 0));
                if !is_negative {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(out, "    try testing.expect({field_expr} >= {zig_val});");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} <= {zig_val});");
            }
        }
        "contains_any" => {
            // At least ONE of the values must be found in the field (OR logic).
            if let Some(values) = &assertion.values {
                let string_values: Vec<String> = values
                    .iter()
                    .filter_map(|v| {
                        if let serde_json::Value::String(s) = v {
                            Some(format!(
                                "std.mem.indexOf(u8, {field_expr}, \"{}\") != null",
                                escape_zig(s)
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();
                if !string_values.is_empty() {
                    let condition = string_values.join(" or\n        ");
                    let _ = writeln!(out, "    try testing.expect(\n        {condition}\n    );");
                }
            }
        }
        "matches_regex" => {
            let _ = writeln!(out, "    // regex match not yet implemented for Zig");
        }
        "method_result" => {
            let _ = writeln!(out, "    // method_result assertions not yet implemented for Zig");
        }
        other => {
            panic!("Zig e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Zig literal string.
fn json_to_zig(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_zig(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_zig).collect();
            format!("&.{{{}}}", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_zig(&json_str))
        }
    }
}

/// Map an IR `TypeRef` to a Zig type string for stub method signatures.
///
/// Used only by `emit_test_backend` — not the full production type-map.
/// Keeps stub generation self-contained and avoids a dependency on the
/// private `backends::zig::type_map` module.
///
/// Plugin trait method stubs receive C FFI types from the vtable thunks, not Zig-friendly
/// wrapper types. All struct/enum parameters are opaque `[*c]const u8` pointers, and
/// string/bytes are also `[*c]const u8`. Therefore, all TypeRef::Named types are
/// substituted with `[*c]const u8` to match the actual C FFI signatures the thunks work with.
///
/// `_excluded_types` — unused, kept for compatibility with potential future extensions.
fn zig_type_for_stub(ty: &crate::core::ir::TypeRef, _excluded_types: &std::collections::HashSet<&str>) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "i32".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 | PrimitiveType::Usize => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 | PrimitiveType::Isize => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes => "[*c]const u8".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Optional(inner) => {
            // In C FFI, optional values are passed as nullable pointers.
            // For Optional[String] returning from methods, use ?[*c]const u8.
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes => {
                    "?[*c]const u8".to_string()
                }
                _ => format!("?{}", zig_type_for_stub(inner, _excluded_types)),
            }
        }
        TypeRef::Vec(_inner) => {
            // All collections in trait bridge stubs are marshalled as JSON: [*c]const u8.
            // This includes Vec[String], Vec[Vec[f32]], Vec[Struct], etc.
            "[*c]const u8".to_string()
        }
        TypeRef::Map(_, _v) => "[*c]const u8".to_string(),
        // All Named types (structs, enums) map to opaque C FFI pointers.
        // The vtable thunks pass these as [*c]const u8 to user method stubs.
        TypeRef::Named(_) => "[*c]const u8".to_string(),
        TypeRef::Duration => "i64".to_string(),
    }
}

/// Emit FFI-appropriate default value for stub return type.
/// Stub types are C FFI types, so we use Zig/C appropriate literals.
fn zig_stub_default_value(stub_type: &str) -> String {
    match stub_type {
        "[*c]const u8" => "\"\"".to_string(),
        "?[*c]const u8" => "null".to_string(),
        "void" => "".to_string(),
        "i32" | "i16" | "i8" => "0".to_string(),
        "i64" => "0".to_string(),
        "u8" | "u16" | "u32" | "u64" => "0".to_string(),
        "f32" | "f64" => "0.0".to_string(),
        _ => "undefined".to_string(),
    }
}

/// Determine if a method needs JSON-encoded default values for out_result parameters.
/// This occurs for infallible (non-error) methods with complex return types that are
/// wrapped in out_result parameters at the FFI boundary.
fn method_needs_json_default(method: &crate::core::ir::MethodDef) -> bool {
    // Only infallible methods need JSON defaults
    if method.error_type.is_some() {
        return false;
    }

    // Skip Unit and primitive types
    use crate::core::ir::TypeRef;
    match &method.return_type {
        TypeRef::Unit => false,
        TypeRef::Primitive(_) => false,
        _ => true, // String, Vec, Named types, etc. need JSON encoding
    }
}

/// Generate appropriate JSON default for a method return type.
/// For complex types that are serialized to JSON, return a sensible empty/default JSON value.
fn zig_json_default_for_type(return_type: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::TypeRef;
    match return_type {
        TypeRef::Vec(_) => "\"[]\"".to_string(),    // Empty array
        TypeRef::Map(_, _) => "\"{}\"".to_string(), // Empty object
        TypeRef::String => "\"\"".to_string(),      // Empty string
        TypeRef::Named(_) => "\"{}\"".to_string(),  // Default JSON object for custom types
        _ => "\"{}\"".to_string(),                  // Fallback to empty object
    }
}

/// Emit a Zig test backend stub with excluded type handling.
///
/// Wraps `emit_test_backend_inner` with an excluded types set passed through
/// to `zig_type_for_stub` for proper type substitution in trait bridge stubs.
fn emit_test_backend_with_excluded(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
) -> super::TestBackendEmission {
    emit_test_backend_inner(trait_bridge, methods, fixture, excluded_types)
}

/// Emit a Zig test backend stub.
///
/// Generates a Zig struct type for the stub, then builds a vtable via the
/// `make_{trait_snake}_vtable` helper and registers it.
///
/// Rules:
/// - Struct name: `TestStub_{sanitized_snake_fixture_id}`.
/// - Required methods (without `has_default_impl`) are stubbed with Zig
///   defaults from `ZigDefaults`.
/// - Super-trait `name` method returns the literal `"test"` string.
/// - The `register_fn` from `trait_bridge.register_fn` drives the
///   registration expression; snake_case convention for Zig.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    let excluded_types = std::collections::HashSet::new();
    emit_test_backend_inner(trait_bridge, methods, fixture, &excluded_types)
}

/// Internal implementation of test backend emission with excluded type handling.
fn emit_test_backend_inner(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::core::ir::TypeRef;

    let _defaults = language_defaults("zig");
    let id_snake = crate::e2e::escape::sanitize_ident(&fixture.id.to_snake_case());
    let struct_name = format!("TestStub_{id_snake}");
    let var_name = format!("stub_{id_snake}");
    let vtable_var = format!("vtable_{id_snake}");
    let trait_snake = trait_bridge.trait_name.to_snake_case();

    let mut setup = String::new();

    // No leading indent: caller splits by lines and adds 4 spaces per line (test body indent).
    let _ = writeln!(setup, "const {struct_name} = struct {{");

    // Use standard defaults for super-trait methods that don't return test-specific values.
    let _defaults = language_defaults("zig");

    // Plugin super-trait: `name()` returns a sentinel C-string.
    // Driven from IR — no method names are hardcoded.
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_snake = method.name.to_snake_case();
            if method.name == "name" {
                let _ = writeln!(
                    setup,
                    "    pub fn {method_snake}() ?[*:0]const u8 {{ return \"test\"; }}"
                );
            } else if method.name == "version" {
                let _ = writeln!(
                    setup,
                    "    pub fn {method_snake}() ?[*:0]const u8 {{ return \"0.0.1\"; }}"
                );
            } else {
                // Initialize/shutdown and other super-trait methods: emit a void stub.
                // Use @This() instead of struct_name to avoid self-reference inside struct definition.
                let _ = writeln!(setup, "    pub fn {method_snake}(_: *@This()) !void {{}}");
            }
        }
    }

    // Emit ALL trait methods (both required and optional with defaults).
    // The trait-bridge vtable will call all of them, so stubs must implement them all.
    for method in methods.iter() {
        // Skip super-trait methods already emitted above.
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        let method_snake = method.name.to_snake_case();
        let ret_ty = zig_type_for_stub(&method.return_type, excluded_types);

        // For infallible methods with complex return types, use JSON-encoded defaults.
        // These methods are wrapped in out_result parameters at the FFI boundary.
        let default_val = if method_needs_json_default(method) {
            zig_json_default_for_type(&method.return_type)
        } else {
            zig_stub_default_value(&ret_ty)
        };
        let _ = _defaults; // unused but imported for future use

        // Build Zig parameter list (self first using @This(), then method params).
        // Zig does not allow using a type name inside its own definition, so use @This().
        let mut params = vec!["_: *@This()".to_string()];
        for p in &method.params {
            let p_ty = zig_type_for_stub(&p.ty, excluded_types);
            params.push(format!("_: {}", p_ty)); // Mark all method params as unused with _
        }
        let param_list = params.join(", ");

        // For trait bridge methods, emit error-union returns if the method is
        // fallible in the Rust trait. This lets the vtable thunk use `if` syntax
        // to handle the error union result.
        let ret_sig = if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                "!void".to_string()
            } else {
                format!("!{}", ret_ty)
            }
        } else {
            if matches!(method.return_type, TypeRef::Unit) {
                "void".to_string()
            } else {
                ret_ty.clone()
            }
        };

        if matches!(method.return_type, TypeRef::Unit) {
            let _ = writeln!(setup, "    pub fn {method_snake}({param_list}) {ret_sig} {{}}");
        } else {
            let _ = writeln!(
                setup,
                "    pub fn {method_snake}({param_list}) {ret_sig} {{ return {default_val}; }}"
            );
        }
    }

    let _ = writeln!(setup, "}};");
    let _ = writeln!(setup, "var {var_name} = {struct_name}{{}};");
    // lib. is a placeholder; the caller replaces it with the real module name.
    let _ = writeln!(
        setup,
        "const {vtable_var} = lib.make_{trait_snake}_vtable({struct_name}, &{var_name});"
    );

    let out_err_var = format!("out_err_{id_snake}");
    let _ = writeln!(setup, "var {out_err_var}: ?[*c]u8 = null;");

    // arg_expr expands into the argument list for the registration call site:
    // `<binding>.register_fn("test", vtable, &stub, @ptrCast(&out_err))`
    // The caller places arg_expr into args_str, which is used as the full argument list
    // of the top-level `{module}.{register_fn}(args_str)` call.
    let arg_expr = format!("\"test\", {vtable_var}, &{var_name}, @ptrCast(&{out_err_var})");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}

#[cfg(test)]
mod zig_visitor_tests {
    use super::{emit_visitor_test_body, resolve_zig_visitor_call_symbols};
    use crate::core::config::e2e::{CallConfig, CallOverride};
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};
    use std::collections::{BTreeMap, HashMap, HashSet};

    #[test]
    fn visitor_body_uses_configured_ffi_call_symbols() {
        let c_override = CallOverride {
            function: Some("abc_render_document".to_string()),
            options_type: Some("RenderOptions".to_string()),
            result_type: Some("RenderResult".to_string()),
            ..Default::default()
        };
        let zig_override = CallOverride {
            function: Some("renderDocument".to_string()),
            options_type: Some("WrapperOptions".to_string()),
            result_type: Some("WrapperResult".to_string()),
            ..Default::default()
        };
        let call = CallConfig {
            function: "render".to_string(),
            overrides: [("c".to_string(), c_override), ("zig".to_string(), zig_override)].into(),
            ..Default::default()
        };
        let fixture = crate::e2e::fixture::Fixture {
            id: "configured_symbols".to_string(),
            category: None,
            description: "configured symbols".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "html": "<p>Hello</p>", "options": { "trim": true } }),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: String::new(),
            http: None,
        };
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("zig", &fixture, &call, &[]);
        let symbols = resolve_zig_visitor_call_symbols(&call, &recipe, "abc");
        let mut callbacks = BTreeMap::new();
        callbacks.insert("visit_text".to_string(), CallbackAction::Continue);
        let visitor_spec = VisitorSpec { callbacks };
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        let mut content = String::new();
        emit_visitor_test_body(
            &mut content,
            "configured_symbols",
            "<p>Hello</p>",
            fixture.input.get("options"),
            &visitor_spec,
            "sample",
            &symbols,
            &[],
            false,
            &resolver,
        );

        assert!(content.contains("sample.c.abc_render_options_from_json"));
        assert!(content.contains("sample.c.abc_options_set_visitor_handle"));
        assert!(content.contains("sample.c.abc_render_document(_html_z.ptr, _options)"));
        assert!(content.contains("sample.c.abc_render_result_to_json"));
        assert!(content.contains("sample.c.abc_render_result_free"));

        for hardcoded in [
            "htm_conversion_options_from_json",
            "htm_options_set_visitor_handle",
            "htm_convert",
            "htm_conversion_result_to_json",
            "htm_conversion_result_free",
            "WrapperOptions",
            "WrapperResult",
            "renderDocument",
        ] {
            assert!(
                !content.contains(hardcoded),
                "visitor Zig output leaked `{hardcoded}`:\n{content}"
            );
        }
    }
}

#[cfg(test)]
mod tests_trait_bridge {
    /// Verify `emit_test_backend` is generic: output must not contain any
    /// hardcoded domain trait or method names — only names derived from the
    /// synthetic `TestTrait` / `do_work` inputs.
    #[test]
    fn test_emit_test_backend_is_generic_no_domain_names() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
        use crate::e2e::fixture::Fixture;

        let method = MethodDef {
            name: "do_work".to_string(),
            params: vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
            }],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };

        let fixture = Fixture {
            id: "my_fixture".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };

        let methods = vec![&method];
        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        // The setup_block must contain the Zig struct with the method.
        assert!(
            emission.setup_block.contains("do_work"),
            "setup_block should contain method 'do_work', got:\n{}",
            emission.setup_block
        );
        // The vtable helper must use the trait snake name.
        assert!(
            emission.setup_block.contains("make_test_trait_vtable"),
            "setup_block should invoke make_test_trait_vtable, got:\n{}",
            emission.setup_block
        );
        // arg_expr expands into the argument list of the registration call.
        // It must contain the vtable variable and @ptrCast for the out_err pointer.
        assert!(
            emission.arg_expr.contains("vtable_my_fixture"),
            "arg_expr should reference vtable_my_fixture, got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.contains("@ptrCast"),
            "arg_expr should contain @ptrCast for out_err, got:\n{}",
            emission.arg_expr
        );

        // Must not contain any hardcoded domain-specific names.
        for name in &[
            "ImageBackend",
            "RecordProvider",
            "processImage",
            "process_image_fn",
            "sample_lib",
        ] {
            assert!(
                !emission.setup_block.contains(name),
                "setup_block must not contain domain name '{name}', got:\n{}",
                emission.setup_block
            );
        }
    }
}

#[cfg(test)]
mod zig_hash_tests {
    use super::{render_build_zig_zon, resolve_zig_hash};
    use crate::e2e::config::DependencyMode;

    /// When an explicit hash is supplied via alef.toml it must be emitted
    /// verbatim — no network fetch, no cache lookup.
    #[test]
    fn explicit_hash_override_is_used_verbatim() {
        let url = "https://example.invalid/example-org/demo-client/releases/download/v1.4.0/demo-client-zig-v1.4.0-linux-x86_64.tar.gz";
        let pinned = "1220abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
        let result = resolve_zig_hash(Some(pinned), url);
        assert_eq!(
            result.as_deref(),
            Some(pinned),
            "explicit hash must be returned unchanged; got: {result:?}"
        );
    }

    /// When the explicit hash is used it must be emitted in build.zig.zon (single generic tarball).
    #[test]
    fn build_zig_zon_emits_explicit_hash() {
        let hash = "12208badf00d";
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-client/releases/download/v1.4.0-rc.32/demo-client-zig-v1.4.0-rc.32.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, Some(hash.to_string())));
        let content = render_build_zig_zon(
            "demo_client",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.4.0-rc.32",
            &platform_hashes,
            false,
        );
        assert!(
            content.contains(&format!(".hash = \"{hash}\"")),
            "build.zig.zon must embed the explicit hash, got:\n{content}"
        );
        assert!(
            !content.contains(".hash = \"TODO\""),
            "build.zig.zon must not emit TODO when hash is provided, got:\n{content}"
        );
        // Verify the single generic (no-suffix) URL is present.
        assert!(
            content.contains("demo-client-zig-v1.4.0-rc.32.tar.gz"),
            "build.zig.zon must emit the generic source tarball URL (no platform suffix), got:\n{content}"
        );
    }

    #[test]
    fn build_zig_zon_emits_platform_hashes_as_lazy_dependencies() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        platform_hashes.insert(
            "linux-x86_64".to_string(),
            (
                "https://example.invalid/example-org/sample-lib/releases/download/v1.2.3/sample-lib-zig-v1.2.3-linux-x86_64.tar.gz"
                    .to_string(),
                Some("1220linux".to_string()),
            ),
        );
        platform_hashes.insert(
            "macos-arm64".to_string(),
            (
                "https://example.invalid/example-org/sample-lib/releases/download/v1.2.3/sample-lib-zig-v1.2.3-macos-arm64.tar.gz"
                    .to_string(),
                Some("1220macos".to_string()),
            ),
        );

        let content = render_build_zig_zon(
            "sample_lib",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.2.3",
            &platform_hashes,
            false,
        );

        assert!(content.contains(".sample_lib_linux_x86_64"));
        assert!(content.contains(".sample_lib_macos_arm64"));
        assert!(content.contains(".lazy = true"));
        assert!(content.contains(".hash = \"1220linux\""));
        assert!(content.contains(".hash = \"1220macos\""));
        assert!(
            !content.contains(".sample_lib = .{"),
            "platform-specific registry mode must not also emit a generic dependency: {content}"
        );
    }

    /// When no hash is available (None), no fake hash may be emitted for the single generic tarball entry.
    #[test]
    fn build_zig_zon_omits_hash_when_no_hash() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-client/releases/download/v1.4.0-rc.32/demo-client-zig-v1.4.0-rc.32.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, None));
        let content = render_build_zig_zon(
            "demo_client",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.4.0-rc.32",
            &platform_hashes,
            false,
        );
        assert!(
            !content.contains(".hash"),
            "build.zig.zon must omit fake hash metadata when no hash is available, got:\n{content}"
        );
    }

    /// Regression test for the malformed asset URL bug: the rendered URL must
    /// include the repo segment (`<org>/<repo>/releases/...`).  Previously the
    /// codegen defaulted `github_repo` to `https://github.com/<org>` (no
    /// repo), producing `https://github.com/<org>/releases/...` which 404s.
    /// Now the URL is a single generic (no platform suffix) source tarball.
    #[test]
    fn build_zig_zon_emits_full_release_url_with_repo_segment_and_platform_suffix() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-markup/releases/download/v3.5.1/demo-markup-rs-zig-v3.5.1.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, None));
        let content = render_build_zig_zon(
            "demo_markup",
            "../../packages/zig",
            DependencyMode::Registry,
            "3.5.1",
            &platform_hashes,
            false,
        );
        // Verify the generic (no-suffix) URL is present with proper repo segment.
        let expected_url =
            "https://example.invalid/example-org/demo-markup/releases/download/v3.5.1/demo-markup-rs-zig-v3.5.1.tar.gz";
        assert!(
            content.contains(expected_url),
            "build.zig.zon must emit the generic source tarball URL with proper repo segment; got:\n{content}"
        );
    }
}

#[cfg(test)]
mod detect_stale_zig_hash_tests {
    use super::detect_stale_zig_hash;

    /// Stale hash detection: hash contains rc.50, current version is rc.57 → true (stale).
    #[test]
    fn detects_stale_hash_with_older_rc_version() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(result, "expected stale hash detection (rc.50 vs rc.57), but got false");
    }

    /// Matching hash and version: hash contains rc.57, current version is rc.57 → false (fresh).
    #[test]
    fn accepts_matching_version_in_hash() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-rc.57-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(!result, "expected fresh hash (rc.57 matches), but got true (stale)");
    }

    /// Matching stable version: hash contains 1.4.0, current version is 1.4.0 → false (fresh).
    #[test]
    fn accepts_matching_stable_version() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0",
            "demo_client",
        );
        assert!(
            !result,
            "expected fresh hash (1.4.0 matches stable), but got true (stale)"
        );
    }

    /// Malformed hash (wrong pkg_name prefix) → false (no prefix match, silent fail).
    #[test]
    fn returns_false_for_wrong_pkg_name_prefix() {
        let result = detect_stale_zig_hash(
            "wrong_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(
            !result,
            "expected no detection for mismatched pkg_name prefix, but got true"
        );
    }
}

#[cfg(test)]
mod zig_build_tests {
    use super::{ZigBuildFlags, render_build_zig};
    use crate::e2e::config::DependencyMode;

    /// Registry mode test_app build.zig must NOT reference `../../target/release`
    /// (the local workspace layout). Instead, it must link the FFI from the
    /// fetched package's bundled lib/include directories, ensuring compatibility
    /// with published tarballs.
    #[test]
    fn registry_mode_build_zig_links_ffi_from_bundled_paths() {
        let test_filenames = vec!["basic_test.zig".to_string()];
        let content = render_build_zig(
            &test_filenames,
            "demo_client",
            "demo_client",
            "demo_client_ffi",
            "../../crates/demo-client-ffi",
            ZigBuildFlags {
                has_file_fixtures: false,
                needs_mock_server: false,
            },
            "test_documents",
            DependencyMode::Registry,
            false,
        );

        // Must NOT reference the workspace-local target directory.
        assert!(
            !content.contains("../../target/release"),
            "registry mode build.zig must not reference workspace target dir, got:\n{content}"
        );

        // Must link the FFI from the dependency's bundled lib/ directory.
        assert!(
            content.contains("demo_client_dep.path(\"lib\")"),
            "registry mode build.zig must resolve FFI library path from fetched package's lib/ dir, got:\n{content}"
        );

        // Must link the C header from the dependency's bundled include/ directory.
        assert!(
            content.contains("demo_client_dep.path(\"include\")"),
            "registry mode build.zig must resolve FFI header path from fetched package's include/ dir, got:\n{content}"
        );

        // Must explicitly link the FFI system library.
        assert!(
            content.contains("linkSystemLibrary(\"demo_client_ffi\""),
            "registry mode build.zig must link the FFI system library, got:\n{content}"
        );
    }

    /// Local mode test_app build.zig may reference `../../target/release` and
    /// workspace-relative FFI paths (required for local development).
    #[test]
    fn local_mode_build_zig_uses_workspace_paths() {
        let test_filenames = vec!["basic_test.zig".to_string()];
        let content = render_build_zig(
            &test_filenames,
            "demo_client",
            "demo_client",
            "demo_client_ffi",
            "../../crates/demo-client-ffi",
            ZigBuildFlags {
                has_file_fixtures: false,
                needs_mock_server: false,
            },
            "test_documents",
            DependencyMode::Local,
            false,
        );

        // In local mode, workspace paths are expected for development.
        assert!(
            content.contains("../../target/release"),
            "local mode build.zig must reference workspace target dir for local development, got:\n{content}"
        );

        // Must link the FFI system library.
        assert!(
            content.contains("linkSystemLibrary(\"demo_client_ffi\""),
            "local mode build.zig must link the FFI system library, got:\n{content}"
        );
    }
}
