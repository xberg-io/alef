use anyhow::Result;
use std::process;

use crate::cli::{cache, commands, dispatch};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Cache { action } => match action {
            CacheAction::Clear => {
                cache::clear_cache()?;
                println!("Cache cleared.");
                Ok(None)
            }
            CacheAction::Status => {
                cache::show_status();
                Ok(None)
            }
        },
        Commands::Validate { action } => match action {
            ValidateAction::Versions { json, exit_code } => {
                let (_workspace, resolved) = load_config(config_path)?;
                let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
                let workspace_root = std::env::current_dir()?;
                let mut has_mismatches = false;
                for resolved_cfg in &crates_to_process {
                    let checks = commands::validate_versions::run(resolved_cfg, &workspace_root, json)?;
                    if checks.iter().any(|c| !c.matches) {
                        has_mismatches = true;
                    }
                }
                if has_mismatches && exit_code {
                    process::exit(1);
                }
                Ok(None)
            }
        },
        Commands::ReleaseMetadata {
            tag,
            targets,
            git_ref,
            event,
            dry_run,
            force_republish,
            json: _,
        } => {
            let effective_event = if event.is_empty() {
                std::env::var("GITHUB_EVENT_NAME").unwrap_or_default()
            } else {
                event.clone()
            };
            let resolved_opt = load_config(config_path).ok().map(|(_ws, r)| r);
            let resolved_cfg_opt: Option<&crate::core::config::ResolvedCrateConfig> =
                resolved_opt.as_ref().and_then(|r| {
                    dispatch::select_crates(r, &context.crate_filter)
                        .ok()
                        .and_then(|v| v.into_iter().next())
                });
            let meta = commands::release_metadata::compute(
                &tag,
                &targets,
                git_ref.as_deref(),
                &effective_event,
                dry_run,
                force_republish,
                resolved_cfg_opt,
            )?;
            println!("{}", meta.to_json()?);
            Ok(None)
        }
        Commands::CheckRegistry {
            registry,
            package,
            version,
            tap_repo,
            repo,
            source,
            asset_prefix,
            required_assets,
            json,
        } => {
            let extra = commands::check_registry::ExtraParams {
                nuget_source: source,
                tap_repo,
                repo,
                asset_prefix,
                required_assets,
            };
            commands::check_registry::check(registry, &package, &version, &extra, json)?;
            Ok(None)
        }
        Commands::GoTag {
            version,
            remote,
            dry_run,
            json,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let workspace_root = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                let params = commands::go_tag::GoTagParams {
                    version: &version,
                    remote: &remote,
                    dry_run,
                    output_json: json,
                    config: resolved_cfg,
                    workspace_root: &workspace_root,
                };
                commands::go_tag::run(&params)?;
            }
            Ok(None)
        }
        Commands::Snippets { action } => {
            let exit_code = commands::snippets::run(action);
            if exit_code != std::process::ExitCode::SUCCESS {
                process::exit(1);
            }
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}
