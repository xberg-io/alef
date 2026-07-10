use anyhow::{Context, Result};

use crate::cli::dispatch;

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Publish { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            match action {
                PublishAction::Prepare {
                    lang,
                    target,
                    dry_run,
                    require_registry,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(crate::publish::platform::RustTarget::parse)
                        .transpose()?;
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        if multi {
                            eprintln!(
                                "[{}] Preparing publish for: {}",
                                resolved_cfg.name,
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!("Preparing publish for: {}", format_languages(&languages));
                        }
                        crate::publish::prepare(
                            resolved_cfg,
                            &languages,
                            rust_target.as_ref(),
                            dry_run,
                            require_registry,
                        )?;
                    }
                    println!("Prepare complete");
                    Ok(None)
                }
                PublishAction::Build {
                    lang,
                    target,
                    use_cross,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(crate::publish::platform::RustTarget::parse)
                        .transpose()?;
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        if multi {
                            eprintln!(
                                "[{}] Building publish artifacts for: {}",
                                resolved_cfg.name,
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!("Building publish artifacts for: {}", format_languages(&languages));
                        }
                        crate::publish::build(resolved_cfg, &languages, rust_target.as_ref(), use_cross)?;
                    }
                    println!("Build complete");
                    Ok(None)
                }
                PublishAction::Package {
                    lang,
                    target,
                    output,
                    version,
                    dry_run,
                    php_version,
                    php_ts,
                    php_libc,
                    windows_compiler,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(crate::publish::platform::RustTarget::parse)
                        .transpose()?;
                    let output_dir = std::path::Path::new(&output);
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        let ver = version
                            .clone()
                            .or_else(|| resolved_cfg.resolved_version())
                            .context("could not determine version — set --version or version_from in alef.toml")?;

                        let needs_php = languages.contains(&crate::core::config::Language::Php);
                        let pie_opts: Option<crate::publish::package::php::PiePackageOptions<'_>> = if needs_php {
                            let php_ver = php_version
                                .as_deref()
                                .context("--php-version is required when packaging --lang php")?;
                            let ts_mode = crate::publish::package::php::TsMode::parse(&php_ts)?;
                            if let Some(ref rt) = rust_target {
                                if rt.os == crate::publish::platform::Os::Windows && windows_compiler.is_none() {
                                    anyhow::bail!(
                                        "--windows-compiler is required when packaging PHP for a Windows target"
                                    );
                                }
                            }
                            Some(crate::publish::package::php::PiePackageOptions {
                                php_version: php_ver,
                                ts_mode,
                                debug_mode: crate::publish::package::php::DebugMode::NoDebug,
                                libc_override: php_libc.as_deref(),
                                windows_compiler: windows_compiler.as_deref(),
                            })
                        } else {
                            None
                        };

                        let pkg_options = crate::publish::PackageOptions { php: pie_opts };

                        if multi {
                            eprintln!(
                                "[{}] Packaging {} (v{ver}) for: {}",
                                resolved_cfg.name,
                                output_dir.display(),
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!(
                                "Packaging {} (v{ver}) for: {}",
                                output_dir.display(),
                                format_languages(&languages)
                            );
                        }
                        crate::publish::package(
                            resolved_cfg,
                            &languages,
                            rust_target.as_ref(),
                            output_dir,
                            &ver,
                            dry_run,
                            &pkg_options,
                        )?;
                    }
                    println!("Package complete");
                    Ok(None)
                }
                PublishAction::Validate => {
                    let mut all_issues: Vec<String> = Vec::new();
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, None)?;
                        let issues = crate::publish::validate(resolved_cfg, &languages)?;
                        all_issues.extend(issues);
                    }
                    if all_issues.is_empty() {
                        println!("All package manifests are consistent");
                    } else {
                        eprintln!("Validation issues:");
                        for issue in &all_issues {
                            eprintln!("  - {issue}");
                        }
                        anyhow::bail!("{} validation issue(s) found", all_issues.len());
                    }
                    Ok(None)
                }
            }
        }
        other => Ok(Some(other)),
    }
}
