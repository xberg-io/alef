//! Publish pipeline for alef — vendoring, building, and packaging artifacts
//! for distribution across language package registries.
//!
//! This crate provides the local logic behind `alef publish prepare`,
//! `alef publish build`, and `alef publish package`. It does NOT handle
//! registry authentication or publishing — those remain in CI actions.

pub mod ffi_stage;
pub mod package;
pub mod platform;
pub mod vendor;

use alef_core::config::AlefConfig;
use alef_core::config::extras::Language;
use alef_core::config::publish::{PublishLanguageConfig, VendorMode};
use anyhow::{Context, Result};
use platform::RustTarget;
use std::path::Path;

/// Prepare a language package for publishing: vendor dependencies, stage FFI artifacts.
pub fn prepare(config: &AlefConfig, languages: &[Language], target: Option<&RustTarget>, dry_run: bool) -> Result<()> {
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
pub fn build(config: &AlefConfig, languages: &[Language], target: Option<&RustTarget>, use_cross: bool) -> Result<()> {
    let crate_name = &config.crate_config.name;
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
            .as_ref()
            .and_then(|m| m.get(&lang.to_string()))
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
/// `"crates/html-to-markdown-ffi/src/"` → `Some("html-to-markdown-ffi")`
pub(crate) fn crate_name_from_output(config: &AlefConfig, lang: Language) -> Option<String> {
    let output_path = match lang {
        Language::Python => config.output.python.as_deref(),
        Language::Node => config.output.node.as_deref(),
        Language::Ruby => config.output.ruby.as_deref(),
        Language::Php => config.output.php.as_deref(),
        Language::Elixir => config.output.elixir.as_deref(),
        Language::Wasm => config.output.wasm.as_deref(),
        Language::Ffi => config.output.ffi.as_deref(),
        Language::Go => config.output.go.as_deref(),
        Language::Java => config.output.java.as_deref(),
        Language::Csharp => config.output.csharp.as_deref(),
        Language::R => config.output.r.as_deref(),
        Language::Kotlin => config.output.kotlin.as_deref(),
        Language::Gleam => config.output.gleam.as_deref(),
        Language::Zig => config.output.zig.as_deref(),
        Language::Rust => None,
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
fn build_command_for_lang(lang: Language, config: &AlefConfig, target: Option<&RustTarget>, use_cross: bool) -> String {
    let crate_name = &config.crate_config.name;
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
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            eprintln!("Warning: Phase 1: {lang} backend build command not yet implemented");
            String::new()
        }
    }
}

/// Run a shell command and return an error if it fails.
fn run_shell_command(cmd: &str) -> Result<()> {
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

/// Package built artifacts into distributable archives.
pub fn package(
    config: &AlefConfig,
    languages: &[Language],
    target: Option<&RustTarget>,
    output_dir: &Path,
    version: &str,
    dry_run: bool,
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

        let result = match lang {
            Language::Ffi => {
                let t = target.context("--target required for FFI packaging")?;
                let artifact = package::c_ffi::package_c_ffi(config, t, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Php => {
                let t = target.context("--target required for PHP packaging")?;
                let artifact = package::php::package_php(config, t, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Go => {
                let t = target.context("--target required for Go packaging")?;
                let artifact = package::go::package_go_ffi(config, t, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Kotlin => {
                // Kotlin/JVM packaging is target-independent — Gradle produces a JVM jar.
                let artifact = package::kotlin::package_kotlin(config, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Gleam => {
                // Gleam source packaging is target-independent.
                let artifact = package::gleam::package_gleam(config, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Zig => {
                let t = target.context("--target required for Zig packaging")?;
                let artifact = package::zig::package_zig(config, t, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Dart => {
                // Dart source packaging is target-independent (FRB handles cross-compilation).
                let artifact = package::dart::package_dart(config, ws_root, output_dir, version)?;
                Some(artifact)
            }
            Language::Swift => {
                // Swift source packaging is target-independent; XCFramework requires xcodebuild.
                let artifact = package::swift::package_swift(config, ws_root, output_dir, version)?;
                Some(artifact)
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

        if let Some(artifact) = result {
            eprintln!("  produced {}", artifact.name);
        }

        // Run after hooks on success.
        run_publish_after_hooks(lang, &lang_config)?;
    }
    Ok(())
}

/// Validate that all package manifests are ready for publishing.
///
/// Checks:
/// - All required package directories exist
/// - Key manifest files are present (pyproject.toml, package.json, gemspec, etc.)
/// - Cargo.toml version can be read
pub fn validate(config: &AlefConfig, languages: &[Language]) -> Result<Vec<String>> {
    let mut issues = Vec::new();

    // Check version is readable.
    if config.resolved_version().is_none() {
        issues.push(format!("cannot read version from {}", config.crate_config.version_from));
    }

    // Check package directories and key manifest files exist.
    for &lang in languages {
        let pkg_dir = config.package_dir(lang);
        let pkg_path = std::path::Path::new(&pkg_dir);

        // Skip languages that don't have package dirs (Rust, FFI).
        if matches!(lang, Language::Rust | Language::Ffi) {
            continue;
        }

        if !pkg_path.exists() {
            issues.push(format!("{lang}: package directory {pkg_dir} does not exist"));
            continue;
        }

        // Check for key manifest files per language.
        let expected_files: Vec<&str> = match lang {
            Language::Python => vec!["pyproject.toml"],
            Language::Node => vec!["package.json"],
            Language::Ruby => vec![], // gemspec name varies
            Language::Php => vec!["composer.json"],
            Language::Elixir => vec!["mix.exs"],
            Language::Go => vec!["go.mod"],
            Language::Java => vec!["pom.xml"],
            Language::Csharp => vec![], // .csproj name varies
            Language::Wasm => vec![],
            Language::R => vec!["DESCRIPTION"],
            Language::Kotlin => vec!["build.gradle.kts"],
            Language::Gleam => vec!["gleam.toml"],
            Language::Zig => vec!["build.zig"],
            Language::Dart => vec!["pubspec.yaml"],
            Language::Swift => vec!["Package.swift"],
            _ => vec![],
        };

        for file in expected_files {
            if !pkg_path.join(file).exists() {
                issues.push(format!("{lang}: missing {pkg_dir}/{file}"));
            }
        }
    }

    Ok(issues)
}

/// Get the publish configuration for a language, falling back to defaults.
fn publish_config_for_language(config: &AlefConfig, lang: Language) -> PublishLanguageConfig {
    if let Some(publish) = &config.publish {
        let lang_str = lang.to_string();
        if let Some(lang_config) = publish.languages.get(&lang_str) {
            return lang_config.clone();
        }
    }
    PublishLanguageConfig::default()
}

/// Resolve the core crate directory path.
fn resolve_core_crate_dir(config: &AlefConfig) -> String {
    if let Some(publish) = &config.publish {
        if let Some(core_crate) = &publish.core_crate {
            return core_crate.clone();
        }
    }
    // Fall back to deriving from [crate].sources.
    let dir = config.core_crate_dir();
    if !config.crate_config.sources.is_empty() {
        let first = config.crate_config.sources[0].to_string_lossy();
        if first.contains("crates/") {
            return format!("crates/{dir}");
        }
    }
    dir
}

/// Resolve the workspace root directory.
fn resolve_workspace_root(config: &AlefConfig) -> String {
    config
        .crate_config
        .workspace_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

/// Resolve the vendor destination directory for a language.
fn resolve_vendor_dest(config: &AlefConfig, lang: Language) -> String {
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
        Language::Ruby | Language::Elixir => VendorMode::CoreOnly,
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

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::output::StringOrVec;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_temp_marker_file() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let marker = temp_dir.path().join("marker.txt");
        (temp_dir, marker)
    }

    #[test]
    fn test_run_publish_hooks_runs_before_only() {
        let (_temp_dir, marker) = make_temp_marker_file();
        let marker_str = marker.to_str().unwrap();

        // Config with before hook only.
        let config = PublishLanguageConfig {
            before: Some(StringOrVec::Single(format!("echo 'before' > {marker_str}"))),
            ..Default::default()
        };

        let result = run_publish_hooks(Language::Python, &config);
        assert!(result.is_ok());
        assert!(marker.exists(), "before hook should have created marker file");
    }

    #[test]
    fn test_run_publish_hooks_precondition_failure_skips() {
        let (_temp_dir, marker) = make_temp_marker_file();
        let marker_str = marker.to_str().unwrap();

        let config = PublishLanguageConfig {
            precondition: Some("false".to_string()), // Always fails
            before: Some(StringOrVec::Single(format!("echo 'before' > {marker_str}"))),
            ..Default::default()
        };

        let result = run_publish_hooks(Language::Python, &config);
        assert!(result.is_ok());
        // Precondition failed, so before hook should not run
        assert!(!marker.exists(), "before hook should not run when precondition fails");
    }

    #[test]
    fn test_run_publish_after_hooks_runs_after_only() {
        let (_temp_dir, marker) = make_temp_marker_file();
        let marker_str = marker.to_str().unwrap();

        let config = PublishLanguageConfig {
            after: Some(StringOrVec::Single(format!("echo 'after' > {marker_str}"))),
            ..Default::default()
        };

        let result = run_publish_after_hooks(Language::Python, &config);
        assert!(result.is_ok());
        assert!(marker.exists(), "after hook should have created marker file");

        let content = fs::read_to_string(&marker).unwrap();
        assert!(content.contains("after"));
    }

    #[test]
    fn test_run_publish_after_hooks_no_after_is_noop() {
        let config = PublishLanguageConfig::default();
        // Config has no after hook
        let result = run_publish_after_hooks(Language::Python, &config);
        assert!(result.is_ok(), "after hooks should succeed when not specified");
    }

    #[test]
    fn test_run_publish_after_hooks_multiple_commands() {
        let temp_dir = TempDir::new().unwrap();
        let marker1 = temp_dir.path().join("marker1.txt");
        let marker2 = temp_dir.path().join("marker2.txt");

        let marker1_str = marker1.to_str().unwrap();
        let marker2_str = marker2.to_str().unwrap();

        let config = PublishLanguageConfig {
            after: Some(StringOrVec::Multiple(vec![
                format!("echo 'after1' > {marker1_str}"),
                format!("echo 'after2' > {marker2_str}"),
            ])),
            ..Default::default()
        };

        let result = run_publish_after_hooks(Language::Python, &config);
        assert!(result.is_ok());
        assert!(marker1.exists(), "first after command should execute");
        assert!(marker2.exists(), "second after command should execute");
    }

    #[test]
    fn test_run_publish_after_hooks_failure_propagates_error() {
        let config = PublishLanguageConfig {
            after: Some(StringOrVec::Single("false".to_string())), // Command fails
            ..Default::default()
        };

        let result = run_publish_after_hooks(Language::Python, &config);
        assert!(result.is_err(), "after hook failure should propagate error");
    }

    #[test]
    fn test_publish_hooks_full_lifecycle_success() {
        let temp_dir = TempDir::new().unwrap();
        let before_marker = temp_dir.path().join("before.txt");
        let after_marker = temp_dir.path().join("after.txt");

        let before_str = before_marker.to_str().unwrap();
        let after_str = after_marker.to_str().unwrap();

        let config = PublishLanguageConfig {
            before: Some(StringOrVec::Single(format!("echo 'before' > {before_str}"))),
            after: Some(StringOrVec::Single(format!("echo 'after' > {after_str}"))),
            ..Default::default()
        };

        // Simulate before hook
        let before_result = run_publish_hooks(Language::Python, &config);
        assert!(before_result.is_ok());
        assert!(before_marker.exists(), "before hook should run");

        // Simulate successful operation (no actual work)

        // Simulate after hook
        let after_result = run_publish_after_hooks(Language::Python, &config);
        assert!(after_result.is_ok());
        assert!(after_marker.exists(), "after hook should run on success");
    }
}
