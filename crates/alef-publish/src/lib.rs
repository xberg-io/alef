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
        let vendor_mode = lang_config.vendor_mode.unwrap_or_else(|| default_vendor_mode(lang));

        match vendor_mode {
            VendorMode::CoreOnly => {
                let core_crate_dir = resolve_core_crate_dir(config);
                let workspace_root = resolve_workspace_root(config);
                let dest_dir = resolve_vendor_dest(config, lang);
                if dry_run {
                    eprintln!("[dry-run] Would vendor core crate from {core_crate_dir} for {lang}");
                } else {
                    eprintln!("Vendoring core crate from {core_crate_dir} for {lang}...");
                    let generate_ws = matches!(lang, Language::Ruby);
                    let result = vendor::vendor_core_only(
                        Path::new(&workspace_root),
                        Path::new(&core_crate_dir),
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
    }
    Ok(())
}

/// Build release artifacts for a specific platform.
pub fn build(
    _config: &AlefConfig,
    languages: &[Language],
    target: Option<&RustTarget>,
    _use_cross: bool,
) -> Result<()> {
    for &lang in languages {
        let target_str = target.map(|t| t.triple.as_str()).unwrap_or("host");
        eprintln!("Building {lang} for target {target_str}...");
        // TODO: Phase 5 — implement build logic
        eprintln!("  build not yet implemented");
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
            _ => {
                eprintln!("  packaging not yet implemented for {lang}");
                None
            }
        };

        if let Some(artifact) = result {
            eprintln!("  produced {}", artifact.name);
        }
    }
    Ok(())
}

/// Validate that all package manifests are ready for publishing.
pub fn validate(_config: &AlefConfig) -> Result<Vec<String>> {
    // TODO: Phase 6 — extend verify_versions with file presence checks
    Ok(vec![])
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
