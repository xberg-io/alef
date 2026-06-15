//! Publish pipeline for alef — vendoring, building, and packaging artifacts
//! for distribution across language package registries.
//!
//! This crate provides the local logic behind `alef publish prepare`,
//! `alef publish build`, and `alef publish package`. It does NOT handle
//! registry authentication or publishing — those remain in CI actions.

pub mod dart_native;
pub mod ffi_stage;
pub mod package;
pub mod platform;
pub mod vendor;
pub mod workspace;

#[cfg(test)]
mod tests;
mod validate;

pub use validate::validate;

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::core::config::publish::{PublishLanguageConfig, VendorMode};
use anyhow::{Context, Result};
use platform::RustTarget;
use std::path::{Path, PathBuf};

/// Prepare a language package for publishing: vendor dependencies, stage FFI artifacts.
///
/// When `require_registry` is true (CI/release), the Registry vendor mode
/// regenerates each rewritten binding crate's `Cargo.lock` and fails hard if
/// resolution fails — catching the case where a referenced workspace-member
/// version is not yet published to the registry. When false (the default for
/// local/pre-release dev), the lock is simply deleted so the consumer regenerates
/// it at build time.
pub fn prepare(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    target: Option<&RustTarget>,
    dry_run: bool,
    require_registry: bool,
) -> Result<()> {
    for &lang in languages {
        let lang_config = publish_config_for_language(config, lang);

        if !dry_run && !run_publish_hooks(lang, &lang_config)? {
            continue;
        }

        let vendor_mode = lang_config
            .vendor_mode
            .as_ref()
            .unwrap_or(&default_vendor_mode(lang))
            .clone();

        match vendor_mode {
            VendorMode::CoreOnly => {
                let core_crate_dir = resolve_core_crate_dir(config);
                let core_path = Path::new(&core_crate_dir);
                if !core_path.exists() {
                    anyhow::bail!("core crate directory does not exist: {core_crate_dir}");
                }
                let workspace_root = resolve_workspace_root(config);
                let dest_dir = resolve_vendor_dest(config, lang);
                if dry_run {
                    eprintln!("[dry-run] Would vendor core crate from {core_crate_dir} for {lang}");
                } else {
                    eprintln!("Vendoring core crate from {core_crate_dir} for {lang}...");
                    let generate_ws = matches!(lang, Language::Ruby);
                    let result = vendor::vendor_core_only(
                        Path::new(&workspace_root),
                        core_path,
                        Path::new(&dest_dir),
                        generate_ws,
                    )?;
                    eprintln!("  vendored to {}", result.vendor_dir.display());
                }
                // CoreOnly vendors the core crate's sources alongside the binding,
                // but the BINDING crate's Cargo.toml still references workspace
                // members via `path = "..."` — those paths only resolve in-workspace
                // and break the gem/hex build on a consumer machine. Apply the same
                // binding-manifest rewrite that Registry mode does so the published
                // package compiles standalone. The vendored core sources remain
                // available for users who want to build from local source, while
                // cargo falls back to the registry version for normal installs.
                rewrite_binding_path_deps(config, lang, require_registry, dry_run)?;
            }
            VendorMode::Full => {
                let core_crate_dir = resolve_core_crate_dir(config);
                let workspace_root = resolve_workspace_root(config);
                let dest_dir = resolve_vendor_dest(config, lang);
                if dry_run {
                    eprintln!("[dry-run] Would vendor all dependencies from {core_crate_dir} for {lang}");
                } else {
                    eprintln!("Vendoring all dependencies from {core_crate_dir} for {lang}...");
                    let result = vendor::vendor_full(
                        Path::new(&workspace_root),
                        Path::new(&core_crate_dir),
                        Path::new(&dest_dir),
                    )?;
                    eprintln!("  vendored to {}", result.vendor_dir.display());
                }
            }
            VendorMode::Registry => {
                rewrite_binding_path_deps(config, lang, require_registry, dry_run)?;
            }
            VendorMode::None => {}
        }

        // Stage FFI artifacts for FFI-dependent languages.
        if is_ffi_dependent(lang) {
            if let Some(target) = target {
                let workspace_root = resolve_workspace_root(config);
                if dry_run {
                    let platform = target.platform_for(lang);
                    eprintln!("[dry-run] Would stage FFI artifacts for {lang} (platform: {platform})");
                } else {
                    eprintln!("Staging FFI artifacts for {lang}...");
                    let dest = ffi_stage::stage_ffi(config, lang, target, Path::new(&workspace_root))?;
                    eprintln!("  staged to {}", dest.display());
                    if let Some(header) = ffi_stage::stage_header(config, lang, target, Path::new(&workspace_root))? {
                        eprintln!("  header staged to {}", header.display());
                    }
                }
            } else {
                eprintln!("Skipping FFI staging for {lang}: no --target specified");
            }
        }

        // Run after hooks on success (before moving to next language).
        if !dry_run {
            run_publish_after_hooks(lang, &lang_config)?;
        }
    }
    Ok(())
}

/// Validate an identifier against shell-safe character set.
fn validate_identifier(s: &str, label: &str) -> Result<()> {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        Ok(())
    } else {
        anyhow::bail!(
            "{label} contains invalid characters: {s}. Only alphanumeric, underscore, dash, and period allowed."
        )
    }
}

/// Build release artifacts for a specific platform.
pub fn build(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    target: Option<&RustTarget>,
    use_cross: bool,
) -> Result<()> {
    let crate_name = &config.name;
    validate_identifier(crate_name, "crate_name")?;
    if let Some(t) = target {
        validate_identifier(&t.triple, "target.triple")?;
    }

    // For FFI-dependent languages, build the FFI crate first.
    let needs_ffi = languages.iter().any(|l| is_ffi_dependent(*l));
    let ffi_in_list = languages.contains(&Language::Ffi);
    if needs_ffi && !ffi_in_list {
        let cmd = build_command_for_lang(Language::Ffi, config, target, use_cross);
        eprintln!("Building FFI crate (dependency)...");
        run_shell_command(&cmd)?;
    }

    for &lang in languages {
        let lang_config = publish_config_for_language(config, lang);
        if !run_publish_hooks(lang, &lang_config)? {
            continue;
        }

        // Skip FFI-dependent languages if FFI was already built as dependency.
        if matches!(lang, Language::Go | Language::Java | Language::Csharp) && needs_ffi && !ffi_in_list {
            eprintln!("Skipping {lang}: FFI already built as dependency");
            continue;
        }

        // Use custom build command from [publish.languages.{lang}] if set.
        // Otherwise fall back to [build_commands.{lang}].build_release if set.
        // Otherwise use the config-driven default.
        let cmd = if let Some(custom) = &lang_config.build_command {
            substitute_target(&custom.commands().join(" && "), target)
        } else if let Some(build_cmd_cfg) = config
            .build_commands
            .get(&lang.to_string())
            .and_then(|c| c.build_release.as_ref())
        {
            substitute_target(&build_cmd_cfg.commands().join(" && "), target)
        } else {
            build_command_for_lang(lang, config, target, use_cross)
        };

        let target_str = target.map(|t| t.triple.as_str()).unwrap_or("host");
        eprintln!("Building {lang} for target {target_str}...");
        run_shell_command(&cmd)?;
        eprintln!("  build complete for {lang}");

        // Run after hooks on success.
        run_publish_after_hooks(lang, &lang_config)?;
    }
    Ok(())
}

/// Substitute `{target}` placeholder in a command string with the actual triple.
fn substitute_target(cmd: &str, target: Option<&RustTarget>) -> String {
    if let Some(t) = target {
        cmd.replace("{target}", &t.triple)
    } else {
        cmd.replace("{target}", "")
    }
}

/// Extract the Rust crate name from an output path in the config.
///
/// `"crates/example-ffi/src/"` → `Some("example-ffi")`
pub(crate) fn crate_name_from_output(config: &ResolvedCrateConfig, lang: Language) -> Option<String> {
    let output_path = match lang {
        Language::Python => config.explicit_output.python.as_deref(),
        Language::Node => config.explicit_output.node.as_deref(),
        Language::Ruby => config.explicit_output.ruby.as_deref(),
        Language::Php => config.explicit_output.php.as_deref(),
        Language::Elixir => config.explicit_output.elixir.as_deref(),
        Language::Wasm => config.explicit_output.wasm.as_deref(),
        Language::Ffi => config.explicit_output.ffi.as_deref(),
        Language::Go => config.explicit_output.go.as_deref(),
        Language::Java => config.explicit_output.java.as_deref(),
        Language::Csharp => config.explicit_output.csharp.as_deref(),
        Language::R => config.explicit_output.r.as_deref(),
        Language::Kotlin => config.explicit_output.kotlin.as_deref(),
        Language::KotlinAndroid => config.explicit_output.kotlin_android.as_deref(),
        Language::Gleam => config.explicit_output.gleam.as_deref(),
        Language::Zig => config.explicit_output.zig.as_deref(),
        Language::Rust | Language::C | Language::Jni => None,
        Language::Swift | Language::Dart => None,
    }?;
    let path = std::path::Path::new(output_path);
    // Strip trailing `src/` component if present.
    let crate_dir = if path.file_name().is_some_and(|n| n == "src") {
        path.parent()?
    } else {
        path
    };
    crate_dir.file_name()?.to_str().map(|s| s.to_string())
}

/// Generate the build command for a language, deriving crate names from output path config.
///
/// Falls back to `{crate_name}-{suffix}` when no output path is configured.
fn build_command_for_lang(
    lang: Language,
    config: &ResolvedCrateConfig,
    target: Option<&RustTarget>,
    use_cross: bool,
) -> String {
    let crate_name = &config.name;
    let cargo = if use_cross { "cross" } else { "cargo" };
    let target_flag = target.map(|t| format!(" --target {}", t.triple)).unwrap_or_default();

    match lang {
        Language::Python => {
            let pkg = crate_name_from_output(config, Language::Python).unwrap_or_else(|| format!("{crate_name}-py"));
            format!("maturin build --release --manifest-path crates/{pkg}/Cargo.toml{target_flag}")
        }
        Language::Node => {
            let pkg = crate_name_from_output(config, Language::Node).unwrap_or_else(|| format!("{crate_name}-node"));
            let napi_target = target.map(|t| format!(" --target {}", t.triple)).unwrap_or_default();
            format!(
                "napi build --manifest-path crates/{pkg}/Cargo.toml \
                 -o crates/{pkg} --platform --release{napi_target}"
            )
        }
        Language::Wasm => {
            let pkg = crate_name_from_output(config, Language::Wasm).unwrap_or_else(|| format!("{crate_name}-wasm"));
            format!("wasm-pack build crates/{pkg} --release")
        }
        Language::Ruby => {
            let pkg = crate_name_from_output(config, Language::Ruby).unwrap_or_else(|| format!("{crate_name}-rb"));
            format!("{cargo} build --release -p {pkg}{target_flag}")
        }
        Language::Php => {
            let pkg = crate_name_from_output(config, Language::Php).unwrap_or_else(|| format!("{crate_name}-php"));
            format!("{cargo} build --release -p {pkg}{target_flag}")
        }
        Language::Ffi => {
            let pkg = crate_name_from_output(config, Language::Ffi).unwrap_or_else(|| format!("{crate_name}-ffi"));
            format!("{cargo} build --release -p {pkg}{target_flag}")
        }
        Language::Go | Language::Java | Language::Csharp => {
            // FFI-dependent languages: build the FFI crate.
            let pkg = crate_name_from_output(config, Language::Ffi).unwrap_or_else(|| format!("{crate_name}-ffi"));
            format!("{cargo} build --release -p {pkg}{target_flag}")
        }
        Language::Elixir => {
            format!("{cargo} build --release{target_flag}")
        }
        Language::R => {
            let pkg = crate_name_from_output(config, Language::R).unwrap_or_else(|| format!("{crate_name}-r"));
            format!("{cargo} build --release -p {pkg}{target_flag}")
        }
        Language::Rust => {
            format!("{cargo} build --release --workspace{target_flag}")
        }
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig
        | Language::C
        | Language::Jni => {
            eprintln!("Warning: Phase 1: {lang} backend build command not yet implemented");
            String::new()
        }
    }
}

/// Run a shell command and return an error if it fails.
pub(crate) fn run_shell_command(cmd: &str) -> Result<()> {
    eprintln!("  $ {cmd}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .with_context(|| format!("running: {cmd}"))?;

    if !status.success() {
        anyhow::bail!("command failed with exit code {}: {cmd}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Run a shell command in a specific working directory.
pub(crate) fn run_shell_command_in(cmd: &str, dir: &std::path::Path) -> Result<()> {
    eprintln!("  $ {cmd}  (in {})", dir.display());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
        .with_context(|| format!("running: {cmd}"))?;

    if !status.success() {
        anyhow::bail!("command failed with exit code {}: {cmd}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Language-specific options forwarded into individual package functions.
///
/// All fields are optional so callers that don't package PHP can pass a
/// default-constructed value without knowing about PHP-specific flags.
#[derive(Default)]
pub struct PackageOptions<'a> {
    /// Options for PIE-conventional PHP packaging.  Required when `lang == php`.
    pub php: Option<package::php::PiePackageOptions<'a>>,
}

/// Package built artifacts into distributable archives.
pub fn package(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    target: Option<&RustTarget>,
    output_dir: &Path,
    version: &str,
    dry_run: bool,
    options: &PackageOptions<'_>,
) -> Result<()> {
    let workspace_root = resolve_workspace_root(config);
    let ws_root = Path::new(&workspace_root);
    std::fs::create_dir_all(output_dir)?;

    for &lang in languages {
        let lang_config = publish_config_for_language(config, lang);
        let platform = target
            .map(|t| t.platform_for(lang))
            .unwrap_or_else(|| "host".to_string());
        if dry_run {
            eprintln!(
                "[dry-run] Would package {lang} for platform {platform} into {}",
                output_dir.display()
            );
            continue;
        }

        if !run_publish_hooks(lang, &lang_config)? {
            continue;
        }

        eprintln!("Packaging {lang} for platform {platform}...");

        // Defense-in-depth: if this language vendors in Registry mode, re-scan
        // the shipped binding manifest and bail if any workspace-member dep still
        // has a `path` (catches a skipped `prepare`). Cheap — a single read+parse.
        let pkg_vendor_mode = lang_config
            .vendor_mode
            .as_ref()
            .unwrap_or(&default_vendor_mode(lang))
            .clone();
        if matches!(pkg_vendor_mode, VendorMode::Registry) {
            if let Some(manifest) = resolve_binding_manifest(config, lang) {
                let manifest_abs = if manifest.is_absolute() {
                    manifest
                } else {
                    ws_root.join(&manifest)
                };
                let members = workspace::workspace_member_crates(ws_root)?;
                assert_no_member_path_deps(&manifest_abs, &members, lang)?;
            }
        }

        let result = match lang {
            Language::Ffi => {
                let t = target.context("--target required for FFI packaging")?;
                let artifact = package::c_ffi::package_c_ffi(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Php => {
                let t = target.context("--target required for PHP packaging")?;
                let pie_opts = options
                    .php
                    .as_ref()
                    .context("--php-version (and other PHP flags) required for PHP packaging")?;
                let artifact = package::php::package_php(config, t, ws_root, output_dir, version, pie_opts)?;
                Some(vec![artifact])
            }
            Language::Go => {
                let t = target.context("--target required for Go packaging")?;
                let artifact = package::go::package_go_ffi(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Python => {
                let t = target.context("--target required for Python packaging")?;
                let artifacts = package::python::package_python(config, t, ws_root, output_dir, version)?;
                Some(artifacts)
            }
            Language::Wasm => {
                let artifacts = package::wasm::package_wasm(config, ws_root, output_dir, version)?;
                Some(vec![artifacts])
            }
            Language::Node => {
                let t = target.context("--target required for Node packaging")?;
                let artifact = package::node::package_node(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Ruby => {
                let t = target.context("--target required for Ruby packaging")?;
                let artifact = package::ruby::package_ruby(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Elixir => {
                let t = target.context("--target required for Elixir packaging")?;
                let artifacts = package::elixir::package_elixir(config, t, ws_root, output_dir, version)?;
                Some(artifacts)
            }
            Language::Java => {
                let t = target.context("--target required for Java packaging")?;
                let artifact = package::java::package_java(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Csharp => {
                let t = target.context("--target required for C# packaging")?;
                let artifact = package::csharp::package_csharp(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Kotlin => {
                // Kotlin/JVM packaging is target-independent — Gradle produces a JVM jar.
                let artifact = package::kotlin::package_kotlin(config, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Gleam => {
                // Gleam source packaging is target-independent.
                let artifact = package::gleam::package_gleam(config, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Zig => {
                let t = target.context("--target required for Zig packaging")?;
                let artifact = package::zig::package_zig(config, t, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Dart => {
                // Dart source packaging is target-independent (FRB handles cross-compilation).
                let artifact = package::dart::package_dart(config, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Swift => {
                // Swift source packaging is target-independent; XCFramework requires xcodebuild.
                let artifact = package::swift::package_swift(config, ws_root, output_dir, version)?;
                Some(vec![artifact])
            }
            Language::Rust => {
                // CLI packaging is invoked explicitly from alef-cli, not through the language dispatch.
                eprintln!("  CLI (Rust) packaging handled separately");
                None
            }
            _ => {
                eprintln!("  packaging not yet implemented for {lang}");
                None
            }
        };

        if let Some(artifacts) = result {
            for artifact in &artifacts {
                eprintln!("  produced {}", artifact.name);
            }
        }

        // Run after hooks on success.
        run_publish_after_hooks(lang, &lang_config)?;
    }
    Ok(())
}

/// Get the publish configuration for a language, falling back to defaults.
fn publish_config_for_language(config: &ResolvedCrateConfig, lang: Language) -> PublishLanguageConfig {
    if let Some(publish) = &config.publish {
        let lang_str = lang.to_string();
        if let Some(lang_config) = publish.languages.get(&lang_str) {
            return lang_config.clone();
        }
    }
    PublishLanguageConfig::default()
}

/// Resolve the core crate directory path.
fn resolve_core_crate_dir(config: &ResolvedCrateConfig) -> String {
    if let Some(publish) = &config.publish {
        if let Some(core_crate) = &publish.core_crate {
            return core_crate.clone();
        }
    }
    // Fall back to deriving from [crate].sources.
    let dir = config.core_crate_dir();
    if !config.sources.is_empty() {
        let first = config.sources[0].to_string_lossy();
        if first.contains("crates/") {
            return format!("crates/{dir}");
        }
    }
    dir
}

/// Resolve the workspace root directory.
fn resolve_workspace_root(config: &ResolvedCrateConfig) -> String {
    config
        .workspace_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

/// Rewrite the shipped binding manifest's workspace-member path deps to
/// registry version-deps so the crate builds standalone on a consumer machine
/// (no workspace present). Shared by both `VendorMode::Registry` and
/// `VendorMode::CoreOnly` — CoreOnly vendors core sources alongside the
/// binding but the binding manifest still references workspace members via
/// `path = "..."`, which only resolves in-workspace.
fn rewrite_binding_path_deps(
    config: &ResolvedCrateConfig,
    lang: Language,
    require_registry: bool,
    dry_run: bool,
) -> Result<()> {
    let Some(manifest) = resolve_binding_manifest(config, lang) else {
        eprintln!("Skipping path-dep rewrite for {lang}: no shipped binding manifest");
        return Ok(());
    };

    let workspace_root = resolve_workspace_root(config);
    let ws_root = Path::new(&workspace_root);
    let manifest_abs = if manifest.is_absolute() {
        manifest
    } else {
        ws_root.join(&manifest)
    };

    if !manifest_abs.exists() {
        eprintln!(
            "Skipping path-dep rewrite for {lang}: binding manifest not found at {}",
            manifest_abs.display()
        );
        return Ok(());
    }

    // Canonicalize after the exists() check so we have a guarantee the path
    // is on disk. This ensures the absolute path passed to
    // scrub_or_regenerate_lock (and via manifest_dir.parent()) is truly
    // absolute even when ws_root is "." (the resolve_workspace_root fallback).
    // CI runners using /github/workspace symlink mounts can trip canonicalize
    // even for existing paths; fall back to cwd-prefix in that case so the
    // path is at least absolute.
    let manifest_abs = manifest_abs
        .canonicalize()
        .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(&manifest_abs)))
        .with_context(|| {
            format!(
                "could not make binding manifest path absolute for {lang}: {}",
                manifest_abs.display()
            )
        })?;

    let members = workspace::workspace_member_crates(ws_root)?;
    let version = config
        .resolved_version()
        .context("cannot resolve crate version for path-dep rewrite")?;
    if dry_run {
        eprintln!(
            "[dry-run] Would rewrite workspace-member path deps to registry \
             version-deps (v{version}) in {} for {lang}",
            manifest_abs.display()
        );
        return Ok(());
    }

    eprintln!(
        "Rewriting workspace-member path deps to registry version-deps \
         (v{version}) in {} for {lang}...",
        manifest_abs.display()
    );
    vendor::rewrite_path_deps_to_registry(&manifest_abs, &members, &version)?;
    if let Some(manifest_dir) = manifest_abs.parent() {
        // In require_registry (CI/release) mode, regenerate the lock and fail
        // hard if a member version is not yet on the registry. Otherwise,
        // delete the lock (lenient default). The workspace Cargo.lock seeds
        // the regen so transitive deps stay pinned at workspace versions.
        let ws_lock = ws_root.join("Cargo.lock");
        vendor::scrub_or_regenerate_lock(
            manifest_dir,
            require_registry,
            require_registry,
            Some(&ws_lock),
            &members,
        )?;
    }
    eprintln!("  rewrote {}", manifest_abs.display());
    Ok(())
}

/// Resolve the path of the binding `Cargo.toml` that SHIPS for a source-build
/// language — the manifest a consumer compiles on their own machine.
///
/// These are the manifests whose workspace-member `path` dependencies must be
/// rewritten to registry version-dependencies (since the workspace is not
/// shipped alongside the package). Crate/directory names are derived from config
/// accessors, never hardcoded.
///
/// Returns `None` for languages that ship no compilable manifest (e.g. Zig).
fn resolve_binding_manifest(config: &ResolvedCrateConfig, lang: Language) -> Option<PathBuf> {
    let pkg_dir = config.package_dir(lang);
    match lang {
        // Ruby: rb-sys compiles `{pkg}/ext/{ext}/native/Cargo.toml`, where the
        // ext dir is `{core_crate_dir}_rb` (matching scaffold_ruby_cargo).
        Language::Ruby => {
            let ext = format!("{}_rb", config.core_crate_dir().replace('-', "_"));
            Some(
                Path::new(&pkg_dir)
                    .join("ext")
                    .join(ext)
                    .join("native")
                    .join("Cargo.toml"),
            )
        }
        // Elixir: the rustler NIF crate at `{pkg}/native/{app}_nif/Cargo.toml`.
        Language::Elixir => {
            let nif = format!("{}_nif", config.elixir_app_name());
            Some(Path::new(&pkg_dir).join("native").join(nif).join("Cargo.toml"))
        }
        // Python: the maturin source build uses the binding crate manifest at
        // `crates/{py_crate}/Cargo.toml` (same crate the python packager uses).
        Language::Python => {
            let py_crate =
                crate_name_from_output(config, Language::Python).unwrap_or_else(|| format!("{}-py", config.name));
            Some(Path::new("crates").join(py_crate).join("Cargo.toml"))
        }
        // PHP: the ext-php-rs binding crate at `crates/{php_crate}/Cargo.toml`.
        Language::Php => {
            let php_crate =
                crate_name_from_output(config, Language::Php).unwrap_or_else(|| format!("{}-php", config.name));
            Some(Path::new("crates").join(php_crate).join("Cargo.toml"))
        }
        // Swift: the swift-bridge crate ships at `{pkg}/rust/Cargo.toml`.
        Language::Swift => Some(Path::new(&pkg_dir).join("rust").join("Cargo.toml")),
        _ => None,
    }
}

/// Re-scan a shipped binding manifest and bail if any workspace-member dep still
/// carries a `path` (a cheap defense-in-depth check that `prepare()` ran).
fn assert_no_member_path_deps(
    manifest_path: &Path,
    members: &workspace::WorkspaceMembers,
    lang: Language,
) -> Result<()> {
    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        // A missing manifest → nothing to assert (matches prepare()'s skip).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        // Any other IO error (permissions, dangling symlink) is a real failure.
        Err(e) => return Err(e).with_context(|| format!("reading {}", manifest_path.display())),
    };
    let doc: toml_edit::DocumentMut = content
        .parse()
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    let section_has_member_path = |table: Option<&toml_edit::Item>| -> Option<String> {
        let table = table?.as_table_like()?;
        for (key, item) in table.iter() {
            if members.names.contains(key) && item.as_table_like().is_some_and(|t| t.contains_key("path")) {
                return Some(key.to_string());
            }
        }
        None
    };

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dep) = section_has_member_path(doc.get(section)) {
            anyhow::bail!(
                "{lang}: workspace-member dependency '{dep}' in [{section}] of {} still has a `path` — \
                 did `alef publish prepare` run for Registry vendor mode?",
                manifest_path.display()
            );
        }
    }
    if let Some(targets) = doc.get("target").and_then(|t| t.as_table_like()) {
        for (cfg, cfg_item) in targets.iter() {
            let Some(cfg_tbl) = cfg_item.as_table_like() else {
                continue;
            };
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(dep) = section_has_member_path(cfg_tbl.get(section)) {
                    anyhow::bail!(
                        "{lang}: workspace-member dependency '{dep}' in \
                         [target.{cfg}.{section}] of {} still has a `path` — \
                         did `alef publish prepare` run for Registry vendor mode?",
                        manifest_path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Resolve the vendor destination directory for a language.
fn resolve_vendor_dest(config: &ResolvedCrateConfig, lang: Language) -> String {
    let pkg_dir = config.package_dir(lang);
    match lang {
        Language::Ruby => format!("{pkg_dir}/vendor"),
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            format!("{pkg_dir}/native/{app_name}/vendor")
        }
        Language::R => format!("{pkg_dir}/src/rust"),
        _ => format!("{pkg_dir}/vendor"),
    }
}

/// Return the default vendor mode for a language.
fn default_vendor_mode(lang: Language) -> VendorMode {
    match lang {
        // Source-build languages compile the Rust crate from source on the
        // user's machine, so their path dependencies are rewritten to
        // registry version-dependencies rather than vendored.
        Language::Ruby | Language::Elixir | Language::Python | Language::Php | Language::Swift => VendorMode::Registry,
        Language::R => VendorMode::Full,
        _ => VendorMode::None,
    }
}

/// Whether a language depends on the C FFI crate for its bindings.
fn is_ffi_dependent(lang: Language) -> bool {
    matches!(lang, Language::Go | Language::Java | Language::Csharp)
}

/// Run precondition check and before hooks for a language.
///
/// Returns `true` if the main command should proceed, `false` if the
/// precondition failed (skip with warning).
fn run_publish_hooks(lang: Language, lang_config: &PublishLanguageConfig) -> Result<bool> {
    // Check precondition.
    if let Some(precondition) = &lang_config.precondition {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(precondition)
            .status()
            .with_context(|| format!("running precondition for {lang}: {precondition}"))?;
        if !status.success() {
            eprintln!("Skipping {lang}: precondition failed ({precondition})");
            return Ok(false);
        }
    }

    // Run before hooks.
    if let Some(before) = &lang_config.before {
        for cmd in before.commands() {
            run_shell_command(cmd)?;
        }
    }

    Ok(true)
}

/// Run after hooks for a language after successful completion.
///
/// After hooks run only when the main operation succeeds (symmetrical with before hooks,
/// which run only before a successful start). This ensures cleanup/finalization logic
/// only runs when the operation completed.
fn run_publish_after_hooks(lang: Language, lang_config: &PublishLanguageConfig) -> Result<()> {
    if let Some(after) = &lang_config.after {
        for cmd in after.commands() {
            run_shell_command(cmd).with_context(|| format!("running after hook for {lang}: {cmd}"))?;
        }
    }
    Ok(())
}
