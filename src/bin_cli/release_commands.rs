use anyhow::Result;
use std::process;

use crate::cli::{cache, commands, dispatch, pipeline};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Cache { action } => match action {
            CacheAction::Clear => {
                cache::clear_cache()?;
                tracing::info!("Cache cleared.");
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
                    // ~keep Ask `checks_pass` rather than re-deriving the verdict. The local
                    // predicate this replaces tested only whether some check failed to match,
                    // which disagreed with `checks_pass` in both directions: it passed a crate
                    // whose check set was EMPTY, the vacuous pass `checks_pass` exists to
                    // refuse, and it failed a `blocked_on_publish` row, which `checks_pass`
                    // deliberately tolerates because such a row cannot resolve until the
                    // release being gated is published. `--json` already reported
                    // `checks_pass`, so one command could answer `ok: true` and exit 1.
                    if !commands::validate_versions::checks_pass(&checks) {
                        has_mismatches = true;
                    }
                    // ~keep alef #1528: `checks_pass` above only compares version STRINGS across
                    // manifests; it has no notion of whether a committed `Cargo.lock` can actually
                    // resolve a requirement reachable from a manifest alef generated. That drift
                    // (the `tower-http` shape: a hand-written dependency alef never watches moves,
                    // and nothing about alef's own output changes) never prompts a regen, so
                    // `check_generated_lock_freshness` -- correct, and already proven against real
                    // incidents -- never gets a chance to run before the release is cut. This is
                    // the same check, reachable from the actual release gate instead, and it
                    // shares `checks_pass`'s own tolerance for a lock genuinely waiting on this
                    // release's not-yet-published version rather than re-litigating it.
                    if let Some(version) = resolved_cfg.resolved_version()
                        && let Some(error) = pipeline::check_release_lock_freshness(&workspace_root, &version)
                    {
                        tracing::error!("[{}] {error:#}", resolved_cfg.name);
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
                commands::release_metadata::ComputeReleaseMetadataInputs {
                    targets_csv: &targets,
                    git_ref: git_ref.as_deref(),
                    event: &effective_event,
                    dry_run,
                    force_republish,
                    config: resolved_cfg_opt,
                },
            )?;
            crate::bin_cli::output::payload(meta.to_json()?);
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

#[cfg(test)]
mod tests {
    /// This module's own source, read at compile time so the guard cannot drift from the file it
    /// guards.
    ///
    /// ~keep Only the half ABOVE the test module is scanned. A self-referential source scan that
    /// searched the whole file would always match: the needle appears in this module's own
    /// assertion and prose, so the guard could never pass no matter what the production code did.
    /// That is the same vacuity-in-reverse this test exists to prevent.
    fn production_source() -> &'static str {
        include_str!("release_commands.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first element")
    }

    /// `validate_versions::checks_pass` is the single definition of "these version checks pass".
    /// This call site used to re-derive that verdict locally, and the copy disagreed in BOTH
    /// directions: vacuously false for an empty check set (the pass `checks_pass` refuses), and
    /// true for a `blocked_on_publish` row that `checks_pass` deliberately tolerates. Since
    /// `--json` already reported `checks_pass`, one invocation could print `"ok": true` and exit 1.
    ///
    /// The semantics live in `checks_pass`'s own tests. What is untestable through the CLI arm —
    /// and what actually regressed — is the re-derivation, so that is what this pins.
    #[test]
    fn the_version_gate_asks_checks_pass_instead_of_re_deriving_it() {
        let source = production_source();
        assert!(
            source.contains("validate_versions::checks_pass(&checks)"),
            "the `validate versions --exit-code` arm must ask `checks_pass` for the verdict"
        );
        assert!(
            !source.contains(".matches)"),
            "re-deriving the verdict from `matches` reintroduces the empty-set and \
             blocked_on_publish disagreements with `checks_pass`; call it instead"
        );
    }
}
