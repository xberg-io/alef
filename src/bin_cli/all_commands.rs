use anyhow::Result;
use std::path::PathBuf;

use crate::cli::{cache, dispatch, pipeline};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;
#[path = "all_commands_run_setup.rs"]
mod all_commands_run_setup;
mod docs_stage;
mod e2e_stage;
mod preflight;
// `pub(crate)`, not private: `bin_cli::core_commands::generate` reuses `StageFailures` for the
// identical hazard `alef generate` had -- a per-crate post-build failure must not skip the
// terminal `finalize_hashes` call and must not deny every later crate its own regeneration. See
// that module's doc and `core_commands/generate.rs`'s use of it. ~keep
pub(crate) mod stage_failures;
use all_commands_run_setup::{
    create_once_overwrite, refused_snippet_dir_paths, report_deferred_formatting, sync_registry_versions_before_all,
};
use stage_failures::StageFailures;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::All {
            clean,
            clobber_create_once_seeds,
            skip_frb,
            strict,
            skip_snippet_validation,
            skip_compile,
        } => {
            let compile_policy = if skip_compile {
                crate::core::backend::CompilePolicy::Skipped
            } else {
                crate::core::backend::CompilePolicy::Allowed
            };
            if skip_frb {
                let existing = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
                let updated = if existing.is_empty() {
                    "flutter_rust_bridge_codegen".to_string()
                } else {
                    format!("{existing},flutter_rust_bridge_codegen")
                };
                // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", updated) };
            }
            let _ = skip_frb;
            let overwrite_create_once = create_once_overwrite(clean, clobber_create_once_seeds);
            let (mut workspace, mut resolved) = load_config(config_path)?;
            crate::bin_cli::version_pin_sync::sync_alef_version_pin(
                &workspace,
                config_path,
                crate::bin_cli::build_info::running_build_is_clean(),
            )?;
            let registry_versions_changed = {
                let selected = dispatch::select_crates(&resolved, &context.crate_filter)?;
                sync_registry_versions_before_all(config_path, &selected)?
            };
            if registry_versions_changed {
                (workspace, resolved) = load_config(config_path)?;
                crate::bin_cli::version_pin_sync::sync_alef_version_pin(
                    &workspace,
                    config_path,
                    crate::bin_cli::build_info::running_build_is_clean(),
                )?;
            }
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            // Deferred across the whole run, not just this check -- see `StageFailures`'s doc
            // comment. A rejection here must still fail `alef all`, but it must not fail it
            // before the main loop below has had a chance to write bindings, e2e suites and
            // docs for every crate. ~keep
            let mut stage_failures = StageFailures::new();
            preflight::run_snippet_coverage_preflight(&crates_to_process, config_path, &mut stage_failures);

            let config_toml = std::fs::read_to_string(config_path)?;
            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let mut grand_binding_count: usize = 0;
            let mut grand_stub_count: usize = 0;
            let mut grand_api_count: usize = 0;
            let mut grand_scaffold_count: usize = 0;
            let mut grand_readme_count: usize = 0;
            let mut grand_e2e_count: usize = 0;
            let mut grand_doc_count: usize = 0;
            // A refusal is a run-level fact addressed to an operator, so it is accumulated across
            // every writing phase and reported once at the end. Reporting per phase is how this
            // command came to surface only the scaffold phase's refusals while omitting every
            // binding-phase one from the summary entirely. ~keep
            let mut refusals = pipeline::WriteReport::default();
            // A per-crate docs/snippet validation failure must not short-circuit formatting, orphan
            // sweeping, hash finalisation, deferred-formatting reporting or hook installation -- for
            // this crate or for any crate later in this loop -- because the bindings those steps act
            // on are already written to disk by the time the docs stage runs. Returning early there
            // left them unformatted and unstamped, and an unstamped file has no provenance marker for
            // the ownership guard to recognise next run, which manufactures fresh refusals from a
            // failure that had nothing to do with writing. The first failure is what this function's
            // `Result` reports; later ones are only `tracing::error!`-ed so a second crate's distinct
            // docs failure in the same run is never silently dropped. ~keep
            let mut docs_stage_error: Option<anyhow::Error> = None;
            // A generator failure inside either e2e stage below (`crate::e2e::generate_e2e`) must be
            // deferred the same way, and for a sharper reason than the docs case: the two lines right
            // after each write -- `sweep_manifest_orphans` and `cache::write_stage_hash` -- are
            // actively unsafe to run when a backend's codegen failed. `write_stage_hash` would record
            // this IR+config+fixture hash as satisfied, so the *next* run reads it back as cached,
            // never calls `generate_e2e` again, and exits 0 with the failing backend's suite
            // permanently missing. `sweep_manifest_orphans` compares this run's (incomplete) path set
            // against the last good run's (complete) one, so the previously-working backend's own
            // output -- present in the old set, absent from this one -- reads as orphaned and gets
            // deleted. Both call sites below gate on this being `None` before either line runs; write,
            // format and `finalize_hashes` still run unconditionally, so a partially generated suite
            // still ships settled, stamped bytes. (An earlier revision claimed the reason was that
            // "unstamped output has no provenance marker for the ownership guard": false -- provenance
            // is the prose header marker `write_files_report` writes, not the `alef:hash:` line, and
            // that false claim is what justified the per-phase checkpoints removed from this loop.) ~keep
            let mut e2e_stage_error: Option<anyhow::Error> = None;
            // Bindings, stubs, public API and scaffold output are written early in this loop but
            // are stamped exactly once, at the very end, by `finalize_hashes_sweeping` -- see that
            // call's doc comment. Every stage between "written" and that final stamp (e2e/test-apps
            // codegen above, and README generation below) therefore sits inside the stamp's blast
            // radius: a bare `?`/`return Err` in any of them used to abort the whole crate before
            // `finalize_hashes_sweeping` ever ran, leaving already-written, already-formatted
            // binding/scaffold output on disk with no `alef:hash:` line at all -- invisible to
            // `alef verify`, and indistinguishable from hand-authored content. README failures are
            // deferred the same way `docs_stage_error` already defers docs failures, just above. ~keep
            let mut readme_stage_error: Option<anyhow::Error> = None;

            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                // ~keep `alef all` is the chain the repos actually run, and it was the one command
                // resolving `languages` without the hard toolchain gate the other five got: a
                // missing `uv`/`pnpm`/`cargo-upgrade` reached the per-language steps and each one
                // skipped itself, so the whole run reported success having built nothing for that
                // language. `warn_missing_formatters` below is deliberately NOT that check -- it
                // warns about optional formatters, which a skipped step can tolerate.
                pipeline::enforce_required_toolchains(&languages, &resolved_cfg.tools)?;
                pipeline::warn_missing_formatters(&languages);
                if multi {
                    tracing::info!(
                        "[{}] Running all for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Running all for: {}", format_languages(&languages));
                }

                // Recorded before `extract`/`generate` make their first mutation -- see
                // `cache::generation_record`'s in-progress marker doc for why this survives the
                // process being killed outright (a file write, not a `Drop` guard) and why it
                // lives in the gitignored `.alef/` cache rather than a committed record
                // (alef#268). `--clean` is exactly the case this protects: it removes before it
                // writes, so an interruption between the two can leave the tree with LESS than
                // it started with, and this marker is what lets a later `alef verify` tell that
                // apart from ordinary staleness. A marker already present here means the
                // PREVIOUS run for this crate was interrupted before it finished; this run
                // overwrites it with its own fresh start and, on success, clears it below. ~keep
                if cache::generation_record::generation_in_progress(&base_dir, &resolved_cfg.name) {
                    tracing::warn!(
                        crate_name = %resolved_cfg.name,
                        "previous generation run for this crate was interrupted; regenerating it fully"
                    );
                }
                cache::generation_record::mark_generation_in_progress(&base_dir, &resolved_cfg.name)?;

                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                // The fingerprint every stamped stage output (e2e, test-apps) was stamped under --
                // `is_stage_cached` needs it to tell a hand-edited stage output from an untouched
                // one, not just whether the file still exists. ~keep
                let inputs_hash = crate::core::hash::compute_inputs_hash(&sources_hash, &alef_toml_bytes);

                // Accumulated across every phase below and stamped exactly ONCE, by the
                // `finalize_hashes_sweeping` call after the format pass at the end of this loop
                // body. Every phase used to stamp its own output as soon as it was written -- ten
                // `finalize_hashes(&current_gen_paths, ..)` checkpoints, all ahead of the only
                // formatting pass this command runs -- and a stamped file is one poly refuses to
                // format, so those checkpoints made the format pass a no-op for everything `alef
                // all` emitted (21 of 93 files on a measured eight-language fixture). See
                // `pipeline::format::stamp_gate`'s module doc for the mechanism and the measurements.
                // Losing the checkpoints costs the ownership guard nothing: provenance is the prose
                // header marker `write_files_report` writes, not the `alef:hash:` line. ~keep
                let mut current_gen_paths = std::collections::HashSet::new();
                // Whether formatting is needed this run -- covers every write phase (bindings,
                // service API, stubs, public API, scaffold, e2e/test-apps, README, docs), not just
                // bindings/service-API/stubs. A single `HashSet<Language>` populated from only those
                // three phases (`changed_languages`, pre-fix) under-triggered `format_generated`:
                // a run that only rewrote e.g. scaffold or README output left that phase's own
                // `report.changed_count() > 0` unread by the gate, so the whole-tree converging pass
                // never ran and the newly written file stayed unformatted with a stale hash (alef
                // #119). Seeded from `languages_have_post_build_steps` because a post-build step
                // (e.g. Dart's `flutter_rust_bridge_codegen`) runs unconditionally every pass and
                // writes straight to disk with no `WriteReport` at all -- see that function's doc
                // comment for why its mere presence must count as "may have changed". ~keep
                let mut any_output_changed = languages_have_post_build_steps(&languages, resolved_cfg);
                // Registry-mode dependency resolution that had to wait for a publish.
                // Collected rather than raised so finalisation, the orphan sweep and
                // docs all still run; reported once the pipeline has completed. ~keep
                let mut deferred_formatting: Vec<crate::e2e::format::DeferredFormatting> = Vec::new();

                // The binding-orphan sweep below needs last run's per-language output list as its
                // baseline. It cannot read that from `<lang>.manifest` (`cache::read_lang_manifest`):
                // `pipeline::generate` unconditionally overwrites that same file, for every language it
                // regenerates, via `write_lang_hash` a few lines down -- so by the time the sweep ran, a
                // "previous" read of `<lang>.manifest` was actually reading THIS run's own freshly
                // written output for any language that regenerated, and a path this run stopped emitting
                // could never appear there to be swept. `all-bindings-{lang}-ownership` is a dedicated
                // stage manifest `pipeline::generate` never touches, so reading it here -- before
                // `pipeline::generate` runs -- returns last run's binding list untouched. See
                // `binding_ownership`'s write-back below the sweep for the other half. ~keep
                let previous_binding_ownership: std::collections::HashMap<crate::core::config::Language, Vec<PathBuf>> =
                    languages
                        .iter()
                        .map(|language| {
                            (
                                *language,
                                cache::read_stage_paths(
                                    &resolved_cfg.name,
                                    &format!("all-bindings-{language}-ownership"),
                                ),
                            )
                        })
                        .collect();

                tracing::info!("Generating bindings...");
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path, true)?;
                // `<lang>.manifest` (`cache::write_lang_manifest`) must hold the union of every
                // phase's output, not just this one: `pipeline::generate`'s own `write_lang_hash`
                // call already stamped it with only `bindings`, and that stays uncorrected for a
                // language `pipeline::generate` skips as lang-hash-cached (absent from `bindings`
                // entirely) unless seeded here from last run's own manifest -- mirrors
                // `alef generate`'s `language_output_paths` seeding in `core_commands.rs`. ~keep
                let regenerated_languages: std::collections::HashSet<_> =
                    bindings.iter().map(|(language, _)| *language).collect();
                let mut language_output_paths: std::collections::HashMap<
                    crate::core::config::Language,
                    std::collections::HashSet<PathBuf>,
                > = std::collections::HashMap::new();
                for language in languages
                    .iter()
                    .filter(|language| !regenerated_languages.contains(language))
                {
                    language_output_paths
                        .entry(*language)
                        .or_default()
                        .extend(cache::read_lang_manifest(&resolved_cfg.name, &language.to_string()));
                }
                // This run's per-language binding ownership: the exact file list `pipeline::generate`
                // just produced for every language it regenerated, plus -- unchanged -- last run's
                // recorded list for any language `pipeline::generate` skipped as cached (it is present
                // as a key here iff `pipeline::generate` regenerated it, even if that produced zero
                // files). A cache hit must not be read as "this language emitted nothing", or the sweep
                // below would delete every file a cached, unregenerated language still legitimately
                // owns. ~keep
                let mut binding_ownership: std::collections::HashMap<crate::core::config::Language, Vec<PathBuf>> =
                    bindings
                        .iter()
                        .map(|(language, generated)| {
                            (
                                *language,
                                generated.iter().map(|file| base_dir.join(&file.path)).collect(),
                            )
                        })
                        .collect();
                for language in languages.iter() {
                    if binding_ownership.contains_key(language) {
                        continue;
                    }
                    binding_ownership.insert(
                        *language,
                        previous_binding_ownership.get(language).cloned().unwrap_or_default(),
                    );
                }
                // `binding_ownership`'s cache-hit entries just above exist to answer exactly the
                // question the sweep below asks, but the sweep's `keep` set is `current_gen_paths`,
                // not `binding_ownership` -- and `current_gen_paths` is populated only from what
                // `bindings`/`stubs`/`public_api_files` actually returned this run, so a language
                // `pipeline::generate` skipped as cache-hit counted as having emitted nothing. The
                // manifest-based route in `sweep_manifest_orphans` (unlike its disk-scan route) has
                // no per-root "nothing recorded here" guard, so it deleted that language's still-valid
                // binding output on every cache hit -- `<lang>.manifest` stayed intact and non-empty,
                // but a file it named was gone, so the next run's `outputs_exist` read that as a miss,
                // regenerated, and the following hit deleted it again: an unbroken hit/miss/hit/miss
                // cycle (alef-tasks#303). Folding `binding_ownership` in here closes that gap. ~keep
                current_gen_paths.extend(binding_ownership.values().flatten().cloned());

                let mut binding_count: usize = 0;
                for (lang, lang_files) in &bindings {
                    let lang_str = lang.to_string();

                    current_gen_paths.extend(pipeline::stampable_output_paths(lang_files, &base_dir));
                    language_output_paths
                        .entry(*lang)
                        .or_default()
                        .extend(pipeline::managed_output_paths(lang_files, &base_dir));

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                        .collect();

                    let cache_key = format!("{}.{lang_str}", resolved_cfg.name);
                    let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                    let cache_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if cache_match && !clean && generated_files_match_disk(lang_files, &base_dir) {
                        tracing::info!("  [{lang_str}] up to date (skipping)");
                        continue;
                    }

                    let single = vec![(*lang, lang_files.clone())];
                    let report = pipeline::write_files_report(&single, &base_dir)?;
                    refusals.absorb_unwritten(&report);
                    binding_count += report.changed_count();
                    if report.changed_count() > 0 {
                        any_output_changed = true;
                    }
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
                }

                if !api.services.is_empty() {
                    let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
                    if !svc_files.is_empty() {
                        for (lang, files) in &svc_files {
                            current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                            language_output_paths
                                .entry(*lang)
                                .or_default()
                                .extend(pipeline::managed_output_paths(files, &base_dir));
                            // `binding_ownership` seeded above from `bindings` alone: for a backend
                            // whose `generate_bindings()` writes only a Rust glue crate (`crates/{name}-py/src`,
                            // never `packages/python`), the service-API output landing under the
                            // language's actual package root is the ONLY evidence this stage's manifest
                            // (`all-bindings-{language}-ownership`) will ever record for that root --
                            // see the matching fold-in for stubs/public API below and
                            // `core_commands/generate.rs`'s `generation_owned_paths`, which already does
                            // this for `alef generate`. ~keep
                            binding_ownership
                                .entry(*lang)
                                .or_default()
                                .extend(files.iter().map(|file| base_dir.join(&file.path)));
                        }
                        let report = pipeline::write_files_report(&svc_files, &base_dir)?;
                        refusals.absorb_unwritten(&report);
                        let svc_count = report.changed_count();
                        tracing::info!("Generated {svc_count} service API files");
                        if svc_count > 0 {
                            any_output_changed = true;
                        }
                    }
                }

                tracing::info!("Generating scaffolding...");
                // `alef all` always resolves the crate's full configured language set (there is
                // no `--lang` filter on this command), so the crate-wide scaffold manifest below
                // is always written from a complete file list and never clobbers another
                // language's recorded paths. See `write_scaffold_manifest`'s doc for why a
                // `--lang`-filtered caller must not call it. ~keep
                let previous_scaffold_paths = cache::read_scaffold_manifest(&resolved_cfg.name);
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                // The stage that actually holds create-once seeds: `packages/php/composer.json`,
                // `crates/*-node/package.json`, `packages/java/pom.xml`, `packages/zig/build.zig`
                // and every placeholder test file are emitted `generated_header: false`, so
                // `can_skip` is the only thing between a hand-grown one and this run's
                // placeholder. See `create_once_overwrite` for why `clean` no longer answers
                // that question. ~keep
                let scaffold_report =
                    pipeline::write_scaffold_files_report(&scaffold_files, &base_dir, overwrite_create_once)?;
                refusals.absorb_unwritten(&scaffold_report);
                let scaffold_count = scaffold_report.changed_count();
                if scaffold_count > 0 {
                    any_output_changed = true;
                }
                let scaffold_output_paths: Vec<PathBuf> =
                    scaffold_files.iter().map(|file| base_dir.join(&file.path)).collect();
                current_gen_paths.extend(pipeline::stampable_output_paths(&scaffold_files, &base_dir));
                let scaffold_keep: std::collections::HashSet<PathBuf> = scaffold_output_paths.iter().cloned().collect();
                let scaffold_sweep_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                pipeline::sweep_manifest_orphans(&previous_scaffold_paths, &scaffold_keep, &scaffold_sweep_roots, &[])?;
                cache::write_scaffold_manifest(&resolved_cfg.name, &scaffold_output_paths)?;

                tracing::info!("Running post-build processing...");
                // A bare `?`/`return Err` here used to hard-stop the entire run: not just the
                // rest of THIS crate's stages (stubs, scaffold, e2e, docs never ran), but every
                // crate listed after this one in `crates_to_process` too -- one backend's
                // post-build failure (e.g. a Dart `flutter_rust_bridge_codegen` break) denied a
                // multi-crate workspace any regeneration at all (task #186). Deferred into
                // `stage_failures` instead: this crate's remaining stages, and every later
                // crate, still run. `refusals` is still reported -- once, at the very end of
                // this function, covering this failure alongside every other write refusal --
                // which is what turns "install/enable flutter_rust_bridge_codegen" (misleading;
                // the tool is present) into "run `alef adopt <path>`" (the actual fix). ~keep
                if let Err(error) = complete_generated_artifacts(&languages, resolved_cfg, &base_dir, compile_policy) {
                    stage_failures.record(&format!("[{}] post-build processing", resolved_cfg.name), error);
                }

                // Fold in every path a post-build step writes unguarded -- mirrors
                // `core_commands/generate.rs`'s identical fold-in for `alef generate` (see
                // `PostBuildStep::owned_paths`'s doc for why this can't be left to the generator's
                // own `GeneratedFile` output). Before this, `alef all` never called `owned_paths`
                // at all, so `MaterializeSwiftBridge`'s real, unguarded trio
                // (`RustBridgeC.h`'s populated form, `SwiftBridgeCore.swift`, `{binding_crate}.swift`)
                // was never claimed in `binding_ownership`/`current_gen_paths` here -- unlike
                // `alef generate`, which has claimed it since the alef #B fix. The very next
                // `alef all` run's orphan sweep then read this crate's swift root as having
                // recorded nothing worth comparing against on the previous run (`previous_paths`
                // empty under `packages/swift`) even though this run's `keep` plainly claims files
                // under it -- exactly the "orphan-reclaim bookkeeping gap" diagnostic describes
                // (alef-task #557). ~keep
                for &language in &languages {
                    let Some(backend) = crate::cli::registry::try_get_backend(language) else {
                        continue;
                    };
                    let Some(build_config) = backend.build_config_with_config(resolved_cfg) else {
                        continue;
                    };
                    let owned: Vec<_> = build_config
                        .post_build
                        .iter()
                        .flat_map(|step| step.owned_paths(&base_dir))
                        .collect();
                    if owned.is_empty() {
                        continue;
                    }
                    binding_ownership
                        .entry(language)
                        .or_default()
                        .extend(owned.iter().cloned());
                    current_gen_paths.extend(owned);
                }

                tracing::info!("Generating type stubs...");
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;

                let stub_hashes: Vec<(String, String)> = stubs
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
                let stubs_cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored_stubs = cache::read_generation_hashes(&stubs_cache_key).unwrap_or_default();
                let stubs_match =
                    !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

                let stub_count = if !stubs_match || clean {
                    let report = pipeline::write_files_report(&stubs, &base_dir)?;
                    refusals.absorb_unwritten(&report);
                    let count = report.changed_count();
                    let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);
                    if count > 0 {
                        any_output_changed = true;
                    }
                    count
                } else {
                    tracing::info!("  [stubs] up to date (skipping)");
                    0
                };

                for (lang, files) in &stubs {
                    current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                    language_output_paths
                        .entry(*lang)
                        .or_default()
                        .extend(pipeline::managed_output_paths(files, &base_dir));
                    // See the matching comment on the service-API fold-in above: type stubs are
                    // where Python's `packages/python` output actually lives (`generate_bindings()`
                    // only ever writes the pyo3 glue crate under `crates/{name}-py/src`), so without
                    // this the `all-bindings-python-ownership` stage manifest recorded zero entries
                    // under `packages/python` on every run and orphan reclaim was permanently
                    // disabled for that root. ~keep
                    binding_ownership
                        .entry(*lang)
                        .or_default()
                        .extend(files.iter().map(|file| base_dir.join(&file.path)));
                }

                let mut api_count = 0;
                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages, config_path)?;
                    if !public_api_files.is_empty() {
                        let api_hashes: Vec<(String, String)> = public_api_files
                            .iter()
                            .flat_map(|(_, fs)| {
                                fs.iter().map(|f| {
                                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                                    (
                                        base_dir.join(&f.path).display().to_string(),
                                        cache::hash_content(&normalized),
                                    )
                                })
                            })
                            .collect();
                        let api_cache_key = format!("{}.public_api", resolved_cfg.name);
                        let stored_api = cache::read_generation_hashes(&api_cache_key).unwrap_or_default();
                        let api_match =
                            !api_hashes.is_empty() && api_hashes.iter().all(|(p, h)| stored_api.get(p) == Some(h));

                        for (lang, files) in &public_api_files {
                            current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                            language_output_paths
                                .entry(*lang)
                                .or_default()
                                .extend(pipeline::managed_output_paths(files, &base_dir));
                            // Same fold-in as service API and stubs above. ~keep
                            binding_ownership
                                .entry(*lang)
                                .or_default()
                                .extend(files.iter().map(|file| base_dir.join(&file.path)));
                        }

                        if !api_match || clean {
                            let report = pipeline::write_files_report(&public_api_files, &base_dir)?;
                            refusals.absorb_unwritten(&report);
                            api_count = report.changed_count();
                            tracing::info!("Generated {api_count} public API files");
                            if api_count > 0 {
                                any_output_changed = true;
                            }
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                        } else {
                            tracing::info!("  [public_api] up to date (skipping)");
                        }
                    }
                }

                if !api.version.is_empty() {
                    let pkg = base_dir.join("Package.swift");
                    if let Ok(content) = std::fs::read_to_string(&pkg) {
                        let updated = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{}", api.version));
                        if updated != content {
                            std::fs::write(&pkg, updated)?;
                        }
                    }
                }

                // See `e2e_stage::E2eStageOutcome::error`'s doc comment: every fatal error from
                // either sub-stage is already deferred by the time it reaches here, so this is
                // the same fold-in the run-wide `e2e_stage_error`/`docs_stage_error` pattern
                // just above uses. ~keep
                let e2e_outcome = e2e_stage::run(
                    resolved_cfg,
                    &api,
                    &base_dir,
                    &config_toml,
                    &sources_hash,
                    &alef_toml_bytes,
                    clean,
                    strict,
                    &mut refusals,
                    &mut current_gen_paths,
                    &mut deferred_formatting,
                );
                let e2e_count = e2e_outcome.e2e_count;
                if e2e_outcome.any_output_changed {
                    any_output_changed = true;
                }
                if let Some(error) = e2e_outcome.error {
                    if e2e_stage_error.is_some() {
                        tracing::error!("[{}] e2e codegen failed: {error:#}", resolved_cfg.name);
                    }
                    e2e_stage_error.get_or_insert(error);
                }

                tracing::info!("Generating READMEs...");
                // Deferred into `readme_stage_error` rather than `?`, matching `docs_stage_error`
                // just below: a README rendering or write failure must not skip this crate's
                // terminal format+stamp pass and leave its already-written bindings unstamped --
                // see `readme_stage_error`'s doc comment above the loop. ~keep
                let readme_stage: anyhow::Result<usize> = (|| {
                    let readme_languages = crate::readme::expand_configured_readme_languages(resolved_cfg, &languages);
                    let readme_files = pipeline::readme(&api, resolved_cfg, &readme_languages)?;
                    let readme_report = pipeline::write_scaffold_files_report(&readme_files, &base_dir, true)?;
                    refusals.absorb_unwritten(&readme_report);
                    current_gen_paths.extend(pipeline::stampable_output_paths(&readme_files, &base_dir));
                    Ok(readme_report.changed_count())
                })();
                let readme_count = match readme_stage {
                    Ok(count) => count,
                    Err(err) => {
                        if readme_stage_error.is_some() {
                            tracing::error!("[{}] README generation failed: {err:#}", resolved_cfg.name);
                        }
                        readme_stage_error.get_or_insert(err);
                        0
                    }
                };
                if readme_count > 0 {
                    any_output_changed = true;
                }

                tracing::info!("Generating docs...");
                // No pre-flight "this needs a build" warning here (task #542): a static check
                // keyed on `docs.snippets.validation_level` alone cannot know whether a prior
                // `alef build` already satisfied it, so it fired on every run regardless of
                // outcome -- exactly the "which command am I" anti-pattern the deleted
                // `build_dependency::enforce_build_dependency` gate already illustrated once (see
                // `docs::tests::snippet_build_dependency_removed`). `docs::enforce_snippet_summary`
                // reports the real, evidence-based signal after validation actually runs instead. ~keep
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                // `generate_docs_stage` hands back every page it rendered even when a later step
                // (snippet validation, CLI/MCP adoption, llms/skills) fails, specifically so a
                // strict-mode bail never discards already-rendered API reference pages. Write and
                // hash `doc_files` before propagating `doc_result`, not after. ~keep
                let (doc_files, doc_result) = docs_stage::generate(
                    skip_snippet_validation,
                    &docs_api,
                    resolved_cfg,
                    &doc_languages,
                    &base_dir,
                );
                // Inert today and kept honest on purpose: `docs::generate_docs_stage` forces
                // `generated_header = true` on every reference page and every extra
                // (`cli.md`, `mcp.md`, `llms.txt`, `SKILL.md`) it emits, so `can_skip` cannot
                // fire here whatever this argument says -- threading `clean` in was never
                // buying the docs stage anything. Passing the same decision as the scaffold
                // stage rather than a bare `true` means the day a docs page is emitted as a
                // seed, it is protected by default instead of silently clobbered. ~keep
                let doc_report = pipeline::write_scaffold_files_report(&doc_files, &base_dir, overwrite_create_once)?;
                refusals.absorb_unwritten(&doc_report);
                let doc_count = doc_report.changed_count();
                if doc_count > 0 {
                    any_output_changed = true;
                }
                current_gen_paths.extend(pipeline::stampable_output_paths(&doc_files, &base_dir));
                // Snippet/doc validation (`docs::generate_docs_stage`'s later sub-steps) reads its
                // input from disk, not from `doc_files` in memory. When the ownership guard refuses
                // a write earlier in this same run -- e.g. a pre-marker-fix snippet with no durable
                // ownership record -- the file on disk stays exactly as stale as it was before this
                // run started, and a validation failure against it reads as a defect in freshly
                // generated content when it is actually a defect in content this run never touched.
                // Naming the refusal count on the error, right where it surfaces, is what makes that
                // distinguishable without cross-referencing a warning log emitted stages earlier.
                //
                // A failure here is deferred, not returned: the formatting/sweep/hash-finalisation/
                // hook-installation steps below must still run for this crate (and this loop must
                // still reach every later crate) even though its docs stage failed -- see
                // `docs_stage_error`'s doc comment above the loop for why. `doc_result` is matched by
                // value instead of re-testing `.is_err()` after this point, because the `Ok` arm right
                // below still needs the snippet-refusal warning to run exactly once. ~keep
                match doc_result {
                    Ok(()) => {
                        // An `Ok` snippet-validation verdict is not proof the validated content came
                        // from this run. Same disk-read hazard as the `Err` arm above, just silent
                        // instead of loud: a refused write inside `docs.snippets.dirs`/`inline_dirs`
                        // leaves pre-run bytes in place for `discover_snippets` to grade, and a
                        // validator that happens to accept those stale bytes reports success with no
                        // trace that this run never produced what it graded. That is the "refused
                        // 2,897 writes, reported success, validated two-day-old content" failure mode
                        // -- attribute it here the same way the `Err` arm does. ~keep
                        let snippet_refusals =
                            refused_snippet_dir_paths(&refusals.refused_paths, resolved_cfg, &base_dir);
                        if !snippet_refusals.is_empty() {
                            tracing::warn!(
                                "[{}] docs/snippet validation passed, but {} write(s) inside its \
                                 docs.snippets root(s) were refused by the ownership guard -- validation \
                                 graded pre-run content at those paths, not this run's output: {}",
                                resolved_cfg.name,
                                snippet_refusals.len(),
                                snippet_refusals
                                    .iter()
                                    .map(|path| path.display().to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                    }
                    Err(error) => {
                        // Gated on `refused_snippet_dir_paths` (refusals inside `docs.snippets`
                        // roots), not `refusals.refused_count()` (every refusal anywhere in the
                        // run): a refusal to an unrelated scaffold or README file must not attach
                        // an "ownership guard" excuse to a validation failure it had nothing to do
                        // with -- that wrong attribution is what previously sent investigators
                        // chasing the ownership guard for a plain checkstyle/compiler defect in
                        // freshly generated content. Mirrors the `Ok` arm above: both arms consult
                        // the same scoped set so a validation failure and a validation pass
                        // attribute refusals identically. ~keep
                        let snippet_refusals =
                            refused_snippet_dir_paths(&refusals.refused_paths, resolved_cfg, &base_dir);
                        let error = if !snippet_refusals.is_empty() {
                            pipeline::report_refused_writes(&refusals);
                            pipeline::report_user_owned_skips(&refusals);
                            error.context(format!(
                                "{} file write(s) inside this crate's docs.snippets root(s) were refused by \
                                 the ownership guard (see the refusal report above), so validation graded \
                                 pre-run content at those paths. Fix: check whether the failing path is \
                                 among the refused writes, then run `alef adopt <path>`.",
                                snippet_refusals.len()
                            ))
                        } else {
                            error
                        };
                        if docs_stage_error.is_some() {
                            // A second (or later) crate's docs failure in the same multi-crate run.
                            // Only one error becomes this function's `Result`; without this, every
                            // failure past the first would vanish with no trace at all. ~keep
                            tracing::error!("[{}] docs/snippet validation failed: {error:#}", resolved_cfg.name);
                        }
                        docs_stage_error.get_or_insert(error);
                    }
                }

                let cleanup_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                // `previous_binding_ownership` (read above, before `pipeline::generate` could ever
                // overwrite `<lang>.manifest`) is the correct baseline -- see its doc comment for why
                // `read_lang_manifest` cannot be used here. `binding_ownership` is written back as the
                // new baseline only now, after the sweep has consumed the old one, mirroring
                // `generate-{language}-ownership`'s read-before / write-after ordering in
                // `alef generate` (`bin_cli/core_commands.rs`). Kept as its own dedicated stage rather
                // than sharing that exact stage name: `generate-{language}-ownership` also folds in
                // service-API, stub and public-API paths that `alef all` tracks and sweeps separately
                // (or not at all here), so writing this narrower, bindings-only list under the shared
                // name would let `alef all` silently truncate the broader baseline `alef generate`
                // relies on the next time the two commands are run back to back. ~keep
                let previous_paths: Vec<_> = previous_binding_ownership.into_values().flatten().collect();
                // `cleanup_roots` doubles as the disk-scan candidate list -- see the matching
                // comment at the `alef generate` call site (`core_commands.rs`) for why this is
                // safe: `sweep_manifest_orphans` only scans a root once it independently confirms
                // both `previous_paths` and `current_gen_paths` carry an entry under it. That
                // per-root check is what keeps a language `pipeline::generate`'s per-language cache
                // skipped this run -- which leaves `current_gen_paths` with zero entries under that
                // language's root, not merely a stale one -- from being scanned at all. ~keep
                pipeline::sweep_manifest_orphans(&previous_paths, &current_gen_paths, &cleanup_roots, &cleanup_roots)?;
                for (language, paths) in &binding_ownership {
                    cache::write_stage_hash(
                        &resolved_cfg.name,
                        &format!("all-bindings-{language}-ownership"),
                        &sources_hash,
                        paths,
                    )?;
                }
                // Replaces the bindings-only manifest `pipeline::generate`'s own `write_lang_hash`
                // call left behind, now that every phase (bindings, service API, stubs, public API)
                // has contributed its `carries_alef_marker()` paths to `language_output_paths` above.
                // Writing this after the sweep (not before) matches the ownership write-back just
                // above: both are end-of-loop bookkeeping the sweep must not observe as this run's
                // "previous" state. ~keep
                for (language, paths) in &language_output_paths {
                    let paths: Vec<_> = paths.iter().cloned().collect();
                    cache::write_lang_manifest(&resolved_cfg.name, &language.to_string(), &paths)?;
                }

                // `any_output_changed` alone is the wrong gate for a tree an EARLIER alef stamped
                // without ever formatting: nothing was written this run, and the tree is still
                // non-canonical. `generated_tree_needs_formatting` answers "no" on a settled tree,
                // so the fast path is unchanged -- see `pipeline::format::stamp_gate`. ~keep
                if any_output_changed || pipeline::generated_tree_needs_formatting(&base_dir) {
                    tracing::info!("Formatting generated files...");
                    // Scope-symmetric by construction: `finalize_hashes_sweeping` below re-stamps
                    // `current_gen_paths` plus everything under `cleanup_roots`, a superset of what
                    // is stripped here, so no file can be left carrying no hash line at all. ~keep
                    let unstamped = pipeline::unstamp_before_formatting(&current_gen_paths);
                    tracing::debug!("unstamped {unstamped} generated file(s) so the formatter can see them");
                    // `None` selects the converging whole-tree pass, which is what a full regen needs
                    // and what `converge_full_regen_formatting` documents itself as serving. Passing
                    // `Some(&only_languages_that_wrote_bindings)` would take the single-pass branch
                    // instead, so the loop that exists precisely because poly's .cs/.java/.json engines
                    // are not single-pass idempotent would never run on the one command that regenerates
                    // everything: `alef all` would leave drift that a second `alef all` would silently
                    // settle, and stamp hashes over it. The language filter is also wrong for the
                    // workspace-wide `cargo sort -n -w` folded into that loop, which must cover crates
                    // this run did not generate. ~keep
                    // `strict` must reach this pass too: until it did, `--strict` guarded the e2e
                    // tree while `packages/<lang>` -- the shipped bindings -- went unguarded. ~keep
                    let skipped = pipeline::format_generated_reporting(resolved_cfg, &base_dir, None, strict)?;
                    deferred_formatting.extend(skipped);
                }

                tracing::info!("Finalising hashes...");
                // Sweeping (not the plain path-tracked `finalize_hashes` used by the
                // earlier per-stage checkpoints above) so that a language dropped from
                // `bindings` by the per-language cache in `pipeline::generate` -- and
                // therefore never added to `current_gen_paths` -- still gets its
                // on-disk output re-stamped from `cleanup_roots`. Safe to run after
                // `sweep_manifest_orphans` above: it clones `current_gen_paths` rather
                // than mutating it, so the orphan sweep already saw the untouched,
                // precisely-tracked set.
                pipeline::finalize_hashes_sweeping(
                    &current_gen_paths,
                    &cleanup_roots,
                    &sources_hash,
                    &alef_toml_bytes,
                )?;
                // Records this crate's generation-inputs fingerprint centrally, once, now that
                // generation for it has completed successfully -- the replacement for folding
                // `inputs_hash` into every file's own stamp. See `core::hash`'s module doc and
                // `cache::generation_record`. Reuses the `inputs_hash` already computed above
                // for the stage cache rather than re-deriving it. ~keep
                cache::record_inputs_hash(&base_dir, &resolved_cfg.name, &inputs_hash)?;
                // This crate's run reached the point `record_inputs_hash` just marked as its
                // successful baseline -- clear the in-progress marker set above so it is
                // indistinguishable from a crate that was never interrupted at all. ~keep
                cache::generation_record::clear_generation_in_progress(&base_dir, &resolved_cfg.name)?;

                // Reported only now, after finalisation, the orphan sweep and docs have
                // all run. Raising at the point of failure is what made the release
                // unreachable: these steps resolve the very version the run produces. ~keep
                report_deferred_formatting(&resolved_cfg.name, &deferred_formatting);

                // Generation for this crate is complete and every manifest it owns is on disk in
                // its final form, so this is the first point at which "does the committed lock
                // beside a manifest alef vouches for still resolve" is a well-formed question.
                // Recorded rather than returned: the answer is about a file alef does not author,
                // so it must not deny the remaining crates their regeneration -- the run simply
                // must not keep exiting 0 over a lock cargo would reject. See
                // `cli::pipeline::lock_freshness` for why the pre-existing relock hook cannot
                // observe this. Tolerating-variant, not the plain check: a `test_apps`/e2e
                // manifest requiring this crate's own not-yet-published version cannot resolve
                // until release and must warn, not fail -- see that function's doc. ~keep
                if let Some(error) = pipeline::check_generated_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    resolved_cfg.resolved_version().as_deref(),
                ) {
                    stage_failures.record(
                        &format!("[{}] generated Cargo.lock freshness", resolved_cfg.name),
                        error,
                    );
                }
                // Same reasoning as the Cargo.lock check immediately above, for the Node
                // ecosystem's equivalent: e2e/test-app generation can leave a `package.json`
                // whose specifiers a committed `pnpm-lock.yaml` no longer matches, which fails
                // `pnpm install` under the default frozen lockfile in CI. See
                // `cli::pipeline::lock_freshness::check_generated_node_lock_freshness`'s doc
                // comment for why this is a sibling check rather than a shared one.
                // Tolerating-variant, same rationale as the Cargo.lock check above. ~keep
                if let Some(error) = pipeline::check_generated_node_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(
                        &format!("[{}] generated pnpm-lock.yaml freshness", resolved_cfg.name),
                        error,
                    );
                }
                // Same reasoning as the two lock checks immediately above, for the Python/uv
                // ecosystem's equivalent: e2e/test-app generation can leave a `pyproject.toml`
                // whose dependency specifiers a committed `uv.lock` no longer records, which fails
                // `uv sync --locked` under CI's default frozen lockfile. See
                // `cli::pipeline::lock_freshness::check_generated_uv_lock_freshness`'s doc comment
                // for why this is a sibling check rather than a shared one. Tolerating-variant,
                // same rationale as the Cargo.lock check above. ~keep
                if let Some(error) = pipeline::check_generated_uv_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(&format!("[{}] generated uv.lock freshness", resolved_cfg.name), error);
                }
                // Same reasoning as the checks immediately above, for the Composer/PHP
                // ecosystem's equivalent: e2e/test-app generation can leave a `composer.json`
                // whose constraints a committed `composer.lock` no longer resolves, which fails
                // `composer install` in CI. Tolerating-variant, same rationale as the Cargo.lock
                // check above. ~keep
                if let Some(error) = pipeline::check_generated_composer_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(
                        &format!("[{}] generated composer.lock freshness", resolved_cfg.name),
                        error,
                    );
                }
                // Same reasoning as the checks immediately above, for the RubyGems ecosystem's
                // equivalent: e2e/test-app generation can leave a `Gemfile` whose constraint a
                // committed `Gemfile.lock` no longer resolves, which fails `bundle install
                // --deployment` in CI. Tolerating-variant, same rationale as the Cargo.lock
                // check above. ~keep
                if let Some(error) = pipeline::check_generated_gemfile_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(
                        &format!("[{}] generated Gemfile.lock freshness", resolved_cfg.name),
                        error,
                    );
                }
                // Same reasoning as the checks immediately above, for the Go ecosystem's
                // equivalent: e2e/test-app generation can leave a `go.mod` requiring an exact
                // version the committed `go.sum` checksum ledger has no entry for, which fails
                // `go build -mod=readonly`/`go test -mod=readonly` (the CI default).
                // Tolerating-variant, same rationale as the Cargo.lock check above. ~keep
                if let Some(error) = pipeline::check_generated_go_sum_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(&format!("[{}] generated go.sum freshness", resolved_cfg.name), error);
                }
                // Same reasoning as the checks immediately above, for the Dart ecosystem's
                // equivalent: e2e/test-app generation can leave a `pubspec.yaml` whose constraint
                // a committed `pubspec.lock` no longer resolves, which fails `dart pub get
                // --enforce-lockfile` in CI. Unlike the internal `stale_dart_pins` helper in
                // `version_lockfiles` (used only to decide whether to re-run `dart pub get`),
                // this surfaces the same class of drift as an actual stage failure.
                // Tolerating-variant, same rationale as the Cargo.lock check above. ~keep
                if let Some(error) = pipeline::check_generated_dart_lock_freshness_tolerating_pending_publish(
                    &current_gen_paths,
                    &base_dir,
                    Some(resolved_cfg),
                ) {
                    stage_failures.record(
                        &format!("[{}] generated pubspec.lock freshness", resolved_cfg.name),
                        error,
                    );
                }

                grand_binding_count += binding_count;
                grand_stub_count += stub_count;
                grand_api_count += api_count;
                grand_scaffold_count += scaffold_count;
                grand_readme_count += readme_count;
                grand_e2e_count += e2e_count;
                grand_doc_count += doc_count;
            }

            pipeline::install_poly_hooks(&base_dir);

            pipeline::report_refused_writes(&refusals);
            pipeline::report_user_owned_skips(&refusals);
            tracing::info!(
                "Done: {grand_binding_count} binding files, {grand_stub_count} stub files, {grand_api_count} API files, {grand_scaffold_count} scaffold files, {grand_readme_count} readme files, {grand_e2e_count} e2e files, {grand_doc_count} doc files"
            );
            // Folded last, after every crate has been through formatting, orphan sweeping, hash
            // finalisation and hook installation -- see `docs_stage_error`'s doc comment for why a
            // docs/snippet validation failure must not reach this point any earlier than this. The
            // run still exits non-zero, and the error's own context (including the refusal-count
            // wrapping above) is untouched; only the timing of the `return` moved. Both go into
            // `stage_failures` rather than an early `return Err`, alongside any pre-flight or
            // post-build failure recorded earlier: a run that hit more than one of these categories
            // used to report only `e2e_stage_error` and silently drop `docs_stage_error` entirely --
            // this is the "accurate summary naming everything that went wrong" task #186 asks for,
            // not just the first thing. ~keep
            if let Some(error) = e2e_stage_error {
                stage_failures.record("e2e codegen", error);
            }
            if let Some(error) = readme_stage_error {
                stage_failures.record("README generation", error);
            }
            if let Some(error) = docs_stage_error {
                stage_failures.record("docs/snippet validation", error);
            }
            stage_failures.into_result()?;
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

#[cfg(test)]
#[path = "all_commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "all_commands_refusal_tests.rs"]
mod refusal_tests;

#[cfg(test)]
#[path = "all_commands_defer_tests.rs"]
mod defer_tests;

#[cfg(test)]
#[path = "pyrefly_generated_package_tests.rs"]
mod pyrefly_generated_package_tests;
