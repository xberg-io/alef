use anyhow::Result;
use std::path::PathBuf;
use std::process;

use crate::cli::{cache, dispatch, pipeline};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;
use super::verify_orphans;

mod docs;
mod generate;
mod verify;
mod verify_flags;

use verify_flags::refuse_unimplemented_verify_flags;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Extract { output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let effective_output = if multi {
                    output
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(format!("{}.ir.json", resolved_cfg.name))
                } else {
                    output.clone()
                };
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                if let Some(parent) = effective_output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&effective_output, serde_json::to_string_pretty(&api)?)?;
                if multi {
                    tracing::info!("[{}] Wrote IR to {}", resolved_cfg.name, effective_output.display());
                } else {
                    tracing::info!("Wrote IR to {}", effective_output.display());
                }
            }
            Ok(None)
        }
        Commands::Generate {
            lang,
            clean,
            skip_frb,
            strict,
            skip_compile,
        } => generate::handle_generate(lang, clean, skip_frb, strict, skip_compile, config_path, context),
        Commands::Stubs { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    tracing::info!(
                        "[{}] Generating type stubs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating type stubs for: {}", format_languages(&languages));
                }
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let hashes: Vec<(String, String)> = files
                    .iter()
                    .flat_map(|(_, fs)| {
                        fs.iter().map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                    })
                    .collect();

                let cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                if all_match {
                    if multi {
                        tracing::info!("[{}] Stubs up to date (skipping)", resolved_cfg.name);
                    } else {
                        tracing::info!("Stubs up to date (skipping)");
                    }
                    continue;
                }

                let count = pipeline::write_files(&files, &base_dir)?;
                let _ = cache::write_generation_hashes(&cache_key, &hashes);

                // `alef stubs` exposes no `--strict` flag, so it always takes the lenient
                // default -- but it goes through the same reporting entry point as every other
                // surface, so a skipped formatter is named in the run output instead of being
                // dropped into a warning nothing collects. ~keep
                pipeline::format_generated_reporting(resolved_cfg, &base_dir, None, false)?;

                let stub_paths: std::collections::HashSet<PathBuf> = files
                    .iter()
                    .flat_map(|(_, fs)| pipeline::stampable_output_paths(fs, &base_dir))
                    .collect();
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes_after_tree_format(&stub_paths, &base_dir, &sources_hash, &alef_toml_bytes)?;
                grand_total += count;
            }
            tracing::info!("Generated {grand_total} stub files");
            Ok(None)
        }
        Commands::Scaffold { lang } => {
            let (workspace, resolved) = load_config(config_path)?;
            crate::bin_cli::version_pin_sync::sync_alef_version_pin(
                &workspace,
                config_path,
                crate::bin_cli::build_info::running_build_is_clean(),
            )?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                // Runs regardless of the stage-cache check below: a manifest a prior scaffold run
                // could not prove ownership of (see `scaffold::repair`'s doc) stays broken forever
                // on a cache hit otherwise, since the cache records this run as complete even
                // though that one write was refused. ~keep
                crate::scaffold::repair_missing_cfg_binding_features(&api, resolved_cfg, &languages);
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "scaffold", &config_toml, &[]);
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                if cache::is_stage_cached(&resolved_cfg.name, "scaffold", &stage_hash) {
                    if multi {
                        tracing::info!("[{}] Scaffold up to date (cached)", resolved_cfg.name);
                    } else {
                        tracing::info!("Scaffold up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    tracing::info!(
                        "[{}] Generating scaffolding for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating scaffolding for: {}", format_languages(&languages));
                }
                let files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let count = pipeline::write_scaffold_files(&files, &base_dir)?;
                let scaffold_paths = pipeline::stampable_output_paths(&files, &base_dir);
                pipeline::finalize_hashes(&scaffold_paths, &sources_hash, &alef_toml_bytes)?;
                // The stage manifest passed to `write_stage_hash` is deliberately every path
                // `pipeline::scaffold` returned, not `scaffold_paths`'s marker-filtered subset.
                // `is_stage_cached`'s disk-presence check (`cache::outputs_exist`) only ever
                // inspects paths recorded in that manifest, so a create-once seed file --
                // `generated_header: false`, unmarked by design so a hand-grown suite is never
                // clobbered on a later run -- was invisible to it. Deleting one left the "scaffold"
                // stage hash unchanged (source, config, and fixtures were untouched) and the cache
                // read as a hit, so `pipeline::scaffold`'s own create-if-absent logic never ran
                // again to replace it: the alef #C incident. Presence is a weaker claim than
                // ownership -- it only says "a path this stage is responsible for still exists",
                // which is exactly what a create-once file's absence should invalidate, independent
                // of whether alef may ever overwrite its content. ~keep
                let all_output_paths: Vec<PathBuf> = files.iter().map(|file| base_dir.join(&file.path)).collect();
                cache::write_stage_hash(&resolved_cfg.name, "scaffold", stage_hash.as_str(), &all_output_paths)?;
                grand_total += count;
            }

            pipeline::install_poly_hooks(&base_dir);

            // downstream crates can use `#[cfg_attr(alef, alef(skip))]` and
            // `#[cfg_attr(feature = "alef-meta", alef(since = "..."))]`
            match pipeline::ensure_workspace_alef_meta_check_cfg() {
                Ok(true) => tracing::info!(
                    "Patched Cargo.toml: added [workspace.lints.rust] unexpected_cfgs allowlist for alef and alef-meta"
                ),
                Ok(false) => {}
                Err(e) => tracing::warn!("could not patch workspace lints for alef/alef-meta: {e}"),
            }

            tracing::info!("Generated {grand_total} scaffold files");
            Ok(None)
        }
        Commands::Readme { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = crate::readme::expand_configured_readme_languages(
                    resolved_cfg,
                    &resolve_readme_languages(resolved_cfg, lang.as_deref())?,
                );
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "readme", &config_toml, &[]);
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                if cache::is_stage_cached(&resolved_cfg.name, "readme", &stage_hash) {
                    if multi {
                        tracing::info!("[{}] READMEs up to date (cached)", resolved_cfg.name);
                    } else {
                        tracing::info!("READMEs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    tracing::info!(
                        "[{}] Generating READMEs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Generating READMEs for: {}", format_languages(&languages));
                }
                let files = pipeline::readme(&api, resolved_cfg, &languages)?;
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files
                    .iter()
                    .filter(|file| file.carries_alef_marker())
                    .map(|file| base_dir.join(&file.path))
                    .collect();
                let readme_paths = pipeline::stampable_output_paths(&files, &base_dir);
                pipeline::finalize_hashes(&readme_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "readme", stage_hash.as_str(), &output_paths)?;
                grand_total += count;
            }
            tracing::info!("Generated {grand_total} README files");
            Ok(None)
        }
        Commands::Docs {
            lang,
            output,
            skip_snippet_validation,
        } => docs::handle(config_path, context, lang, output, skip_snippet_validation),
        Commands::SyncVersions {
            bump,
            set,
            regen,
            skip_swift_checksum,
            release_date,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                if let Some(version) = &set {
                    if multi {
                        tracing::info!("[{}] Setting version to {version}", resolved_cfg.name);
                    } else {
                        tracing::info!("Setting version to {version}");
                    }
                    pipeline::set_version(resolved_cfg, version)?;
                }
                if multi {
                    tracing::info!("[{}] Syncing versions from Cargo.toml", resolved_cfg.name);
                } else {
                    tracing::info!("Syncing versions from Cargo.toml");
                }
                pipeline::sync_versions(
                    resolved_cfg,
                    config_path,
                    bump.as_deref(),
                    !regen,
                    skip_swift_checksum,
                    release_date.as_deref(),
                )?;
            }
            tracing::info!("Version sync complete");
            Ok(None)
        }
        Commands::Build { lang, release, strict } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                let profile = if release { "release" } else { "dev" };
                if multi {
                    tracing::info!(
                        "[{}] Building bindings ({profile}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Building bindings ({profile}) for: {}", format_languages(&languages));
                }
                pipeline::build(resolved_cfg, &languages, release, strict)?;
            }
            tracing::info!("Build complete");
            Ok(None)
        }
        Commands::Fmt { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    tracing::info!("[{}] Formatting generated output...", resolved_cfg.name);
                } else {
                    tracing::info!("Formatting generated output...");
                }
                pipeline::fmt(resolved_cfg, &base_dir)?;
            }
            tracing::info!("Format complete");
            Ok(None)
        }
        Commands::Lint { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    tracing::info!("[{}] Linting generated output...", resolved_cfg.name);
                } else {
                    tracing::info!("Linting generated output...");
                }
                pipeline::lint(resolved_cfg, &base_dir)?;
            }
            tracing::info!("Lint complete");
            Ok(None)
        }
        Commands::Test { lang, e2e, coverage } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_test_languages(resolved_cfg, lang.as_deref(), e2e)?;
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                if multi {
                    tracing::info!(
                        "[{}] Running tests for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Running tests for: {}", format_languages(&languages));
                }
                if e2e {
                    tracing::info!("  (with e2e tests)");
                }
                if coverage {
                    tracing::info!("  (with coverage)");
                }
                pipeline::test(resolved_cfg, &languages, e2e, coverage)?;
            }
            tracing::info!("Tests complete");
            Ok(None)
        }
        Commands::Setup { lang, timeout } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                if multi {
                    tracing::info!(
                        "[{}] Setting up dependencies for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Setting up dependencies for: {}", format_languages(&languages));
                }
                pipeline::setup(resolved_cfg, &languages, timeout)?;
            }
            tracing::info!("Setup complete");
            Ok(None)
        }
        Commands::Clean { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                if multi {
                    tracing::info!(
                        "[{}] Cleaning build artifacts for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Cleaning build artifacts for: {}", format_languages(&languages));
                }
                pipeline::clean(resolved_cfg, &languages)?;
            }
            tracing::info!("Clean complete");
            Ok(None)
        }
        Commands::Update { lang, latest } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                let mode = if latest { "latest" } else { "compatible" };
                if multi {
                    tracing::info!(
                        "[{}] Updating dependencies ({mode}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Updating dependencies ({mode}) for: {}", format_languages(&languages));
                }
                pipeline::update(resolved_cfg, &languages, latest)?;
            }
            tracing::info!("Update complete");
            Ok(None)
        }
        Commands::Verify {
            exit_code: _,
            report_only,
            compile,
            lint,
            lang,
        } => {
            // ~keep `exit_code` is deliberately ignored: it is `hide = true` and documented as a
            // deprecated no-op because verification fails by default now. These three are not.
            // They are visible, documented as doing extra work ("Also run compilation check"),
            // and were destructured away — so `alef verify --compile` exited 0 having compiled
            // nothing, which is indistinguishable from a passing compile check. Refuse instead:
            // a flag that cannot be honored must not report success.
            refuse_unimplemented_verify_flags(compile, lint, lang.as_deref())?;
            verify::run(context, report_only)
        }
        Commands::Diff { exit_code } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            tracing::info!("Computing diff of generated bindings...");
            let base_dir = std::env::current_dir()?;
            let mut all_diffs: Vec<String> = Vec::new();
            // Unioned across every crate before the orphan diff runs below, exactly like
            // `Commands::Verify` above -- see that arm's `all_managed_paths` for why a file
            // legitimately owned by crate B must never look orphaned merely because crate A's
            // own managed surface doesn't mention it. ~keep
            let mut all_managed_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                // `write_cache: false` -- `alef diff` is documented as "without writing" (see its
                // clap doc comment) and must stay read-only the same way `alef verify` does. Passing
                // `true` here regenerated bindings in memory only to preview a diff, yet still ran
                // `pipeline::generate`'s internal `write_lang_hash`, which unconditionally overwrites
                // `<lang>.manifest` with just this call's own file list -- discarding whatever fuller
                // manifest `alef generate`/`alef all` had folded in from later phases (public_api,
                // stubs, service API) via `write_lang_manifest`. For a backend whose core bindings
                // step emits only its Rust glue crate (python/node/ruby/elixir/php/wasm), every `alef
                // diff` run silently regressed `<lang>.manifest` back down to that one file. ~keep
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, true, config_path, false)?;
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let scaffold = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                all_diffs.extend(pipeline::diff_files(&bindings, &base_dir)?);
                all_diffs.extend(pipeline::diff_files(&stubs, &base_dir)?);
                all_diffs.extend(pipeline::diff_files(
                    &[(crate::core::config::Language::Rust, scaffold)],
                    &base_dir,
                )?);
                // `alef diff` is documented as a preview of what `alef generate` would do, and a
                // real generate also sweeps orphans (`pipeline::generate_sweep_roots`,
                // `src/cli/pipeline/generate/orphans.rs`) -- a file the current run's backends
                // would no longer produce. Before this, `alef diff` had no way to preview that
                // impending removal at all: it only ever unioned `pipeline::diff_files` over
                // bindings/stubs/scaffold, never the orphan sweep `alef verify` already runs. This
                // reuses `find_missing_and_frozen_generated_files` purely for its `managed_paths`
                // side effect -- the same full-surface regeneration `Commands::Verify` above pays
                // for the identical reason -- and reports through
                // `verify_orphans::find_orphaned_generated_files`, never a second orphan-finding
                // implementation. ~keep
                let found =
                    find_missing_and_frozen_generated_files(&languages, &api, resolved_cfg, config_path, &base_dir)?;
                all_managed_paths.extend(found.managed_paths);
            }
            let orphan_generated_files = verify_orphans::find_orphaned_generated_files(&base_dir, &all_managed_paths);

            if all_diffs.is_empty() && orphan_generated_files.is_empty() {
                crate::bin_cli::output::line("No changes detected.");
            } else {
                if !all_diffs.is_empty() {
                    crate::bin_cli::output::line("Files that would change:");
                    for diff in &all_diffs {
                        crate::bin_cli::output::line(format_args!("  {diff}"));
                    }
                }
                if !orphan_generated_files.is_empty() {
                    crate::bin_cli::output::line(
                        "Files that would be removed (orphaned generated files a regeneration would sweep -- \
                         alef never deletes automatically; review each and delete by hand if genuinely stale):",
                    );
                    for path in &orphan_generated_files {
                        crate::bin_cli::output::line(format_args!("  {path}"));
                    }
                }
                if exit_code {
                    process::exit(1);
                }
            }
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

/// Fail `alef verify` when a record alef requires to be committed exists on disk but git
/// does not track it.
///
/// Kept separate from [`super::verify_outcome::ensure_success`] because the remedy is
/// different in kind: nothing is stale, nothing regenerates it, a human has to `git add`
/// the file -- so folding it into "generated bindings, versions, or snippet coverage are
/// out of date" would name the wrong fix. The message therefore lists every offending
/// record and the exact command, because the notice this replaces was ignored precisely
/// for being unspecific and unactionable.
///
/// `report_only` short-circuits after the caller has already printed the records, matching
/// how every other verify failure downgrades to a report. ~keep
pub(super) fn ensure_required_records_tracked(untracked: &[&'static str], report_only: bool) -> Result<()> {
    if report_only || untracked.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "required alef records exist but git does not track them: {names}. Fix: `git add {names_spaced}` \
         and commit -- untracked, they exist only on this machine, so a fresh clone or CI has neither the \
         scaffold protection nor a correct orphan picture",
        names = untracked.join(", "),
        names_spaced = untracked.join(" "),
    )
}

/// Fail `alef verify` when a crate's last generation run started but never finished --
/// `cache::generation_record::mark_generation_in_progress` is written before the first
/// mutation of a run and only cleared on success, so a marker still present here means the
/// process that wrote it died mid-flight (alef#268). Kept as a distinct gate, with its own
/// message, for the same reason [`ensure_required_records_tracked`] is: this is not staleness
/// and `alef generate` is not automatically the fix a reader would infer from "out of date" --
/// rerunning is correct, but the diagnosis has to say why, or it reads exactly like the
/// missing-file staleness report this gate exists to distinguish from. `report_only`
/// downgrades to a report, matching every other verify failure. ~keep
pub(super) fn ensure_generation_completed(incomplete_crates: &[String], report_only: bool) -> Result<()> {
    if report_only || incomplete_crates.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "the last generation run did not complete for: {names}. Fix: rerun `alef all`/`alef generate` \
         for those crate(s). This is not staleness -- missing/frozen findings already reported for them \
         may be artifacts of the unfinished run",
        names = incomplete_crates.join(", "),
    )
}

#[cfg(test)]
mod format_scope_tests;
#[cfg(test)]
mod generate_summary_tests;
#[cfg(test)]
mod post_build_failure_stamp_tests;
#[cfg(test)]
mod post_build_format_order_tests;
#[cfg(test)]
mod strict_formatting_tests;
#[cfg(test)]
mod tests;
