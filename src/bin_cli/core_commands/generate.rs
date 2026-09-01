//! `alef generate`'s command-arm body, split out of `core_commands.rs` for the
//! file-modularization cap. Originally a pure extraction of the `Commands::Generate` match arm
//! with no behaviour change; since then it has picked up its own fixes, notably deferring a
//! per-crate post-build failure into `StageFailures` instead of hard-returning before the
//! terminal stamp call (task #546) -- see `stage_failures`'s doc comment above the crate loop.

use anyhow::Result;

use crate::cli::{cache, dispatch, pipeline};

use crate::bin_cli::all_commands::stage_failures::StageFailures;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::bin_cli::helpers::*;

pub(crate) fn handle_generate(
    lang: Option<Vec<String>>,
    clean: bool,
    skip_frb: bool,
    strict: bool,
    skip_compile: bool,
    config_path: &std::path::Path,
    context: &DispatchContext,
) -> Result<Option<Commands>> {
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
    let (workspace, resolved) = load_config(config_path)?;
    crate::bin_cli::version_pin_sync::sync_alef_version_pin(
        &workspace,
        config_path,
        crate::bin_cli::build_info::running_build_is_clean(),
    )?;
    let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
    let multi = dispatch::is_multi_crate(&crates_to_process);
    let base_dir = std::env::current_dir()?;

    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

    // Accumulated across every writing phase and reported once: a refusal is a
    // run-level fact for an operator, and a per-phase summary silently omits every
    // other phase's frozen files. ~keep
    let mut refusals = pipeline::WriteReport::default();
    let mut grand_total_generated: usize = 0;
    // Per-category grand totals, aggregated purely for the always-printed summary below -- see
    // its doc comment for why a single flat total made a 98-file per-language run
    // indistinguishable from a ~278-file full regen and why zeros are printed rather than
    // omitted. ~keep
    let mut grand_binding_count: usize = 0;
    let mut grand_service_api_count: usize = 0;
    let mut grand_public_api_count: usize = 0;
    let mut grand_stub_count: usize = 0;
    let mut grand_scaffold_count: usize = 0;
    let mut crates_with_e2e: usize = 0;
    // A post-build failure (below) must not skip this crate's terminal `finalize_hashes` call,
    // and must not deny every later crate its own regeneration -- see the doc on the
    // `complete_generated_artifacts` call site for the exact hazard this closes. ~keep
    let mut stage_failures = StageFailures::new();
    for resolved_cfg in &crates_to_process {
        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
        pipeline::warn_missing_formatters(&languages);
        if multi {
            tracing::info!(
                "[{}] Generating bindings for: {}",
                resolved_cfg.name,
                format_languages(&languages)
            );
        } else {
            tracing::info!("Generating bindings for: {}", format_languages(&languages));
        }
        // Recorded before `extract`/`generate` make their first mutation -- see
        // `cache::generation_record`'s in-progress marker doc for why this survives the
        // process being killed outright (a file write, not a `Drop` guard) and why it lives
        // in the gitignored `.alef/` cache rather than a committed record (alef#268). A
        // marker already present here means the PREVIOUS run for this crate was interrupted
        // before it finished; this run overwrites it with its own fresh start and, on
        // success, clears it below. ~keep
        if cache::generation_record::generation_in_progress(&base_dir, &resolved_cfg.name) {
            tracing::warn!(
                crate_name = %resolved_cfg.name,
                "previous generation run for this crate was interrupted; regenerating it fully"
            );
        }
        cache::generation_record::mark_generation_in_progress(&base_dir, &resolved_cfg.name)?;
        let api = pipeline::extract(resolved_cfg, config_path, clean)?;
        let files = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path, true)?;
        let regenerated_languages: std::collections::HashSet<_> = files.iter().map(|(language, _)| *language).collect();
        let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

        // Accumulated across every phase below and stamped exactly ONCE, by the
        // `finalize_hashes` call after the format pass at the end of this loop body.
        // Every phase used to stamp its own output as soon as it was written -- five
        // `finalize_hashes(&current_gen_paths, ..)` checkpoints, all ahead of the only
        // formatting pass this command runs -- and a stamped file is one poly refuses to
        // format, so those checkpoints made the format pass a no-op for everything `alef
        // generate` emitted. Exactly the shape `alef all` had (see
        // `pipeline::format::stamp_gate`'s module doc for the mechanism and the
        // measurements) before it was fixed in `all_commands.rs`; this mirrors that fix
        // here. ~keep
        let mut current_gen_paths = std::collections::HashSet::new();
        let mut language_output_paths: std::collections::HashMap<_, std::collections::HashSet<_>> = files
            .iter()
            .map(|(language, generated)| {
                (
                    *language,
                    generated
                        .iter()
                        .filter(|file| file.carries_alef_marker())
                        .map(|file| base_dir.join(&file.path))
                        .collect(),
                )
            })
            .collect();
        let mut generation_owned_paths: std::collections::HashMap<_, std::collections::HashSet<_>> = files
            .iter()
            .map(|(language, generated)| {
                (
                    *language,
                    generated.iter().map(|file| base_dir.join(&file.path)).collect(),
                )
            })
            .collect();
        for language in languages
            .iter()
            .filter(|language| !regenerated_languages.contains(language))
        {
            let cached_paths = cache::read_lang_manifest(&resolved_cfg.name, &language.to_string());
            current_gen_paths.extend(cached_paths.iter().cloned());
            language_output_paths
                .entry(*language)
                .or_default()
                .extend(cached_paths.iter().cloned());
            generation_owned_paths
                .entry(*language)
                .or_default()
                .extend(cached_paths);
        }
        let mut changed_languages: std::collections::HashSet<crate::core::config::Language> =
            std::collections::HashSet::new();

        // The grand total this loop reports (`grand_total_generated`) counts actual
        // writes only, matching every per-phase "Generated N ... files" line below --
        // it must never be the size of a candidate set the generator merely computed in
        // memory. A file that was cache-skipped, refused by the ownership guard, or
        // matched what was already on disk was not generated this run in any sense a
        // reader of that line would expect, so it must not inflate the count. ~keep
        let mut written_count: usize = 0;
        let mut binding_count: usize = 0;
        let mut any_written = false;
        for (lang, lang_files) in &files {
            let lang_str = lang.to_string();
            current_gen_paths.extend(pipeline::stampable_output_paths(lang_files, &base_dir));

            let hashes: Vec<(String, String)> = lang_files
                .iter()
                .map(|f| {
                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                    (
                        base_dir.join(&f.path).display().to_string(),
                        cache::hash_content(&normalized),
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
            written_count += report.changed_count();
            binding_count += report.changed_count();
            if report.changed_count() > 0 {
                any_written = true;
                changed_languages.insert(*lang);
            }
            let _ = cache::write_generation_hashes(&cache_key, &hashes);
        }

        if !api.services.is_empty() {
            let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
            if !svc_files.is_empty() {
                for (_, files) in &svc_files {
                    current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                }
                for (language, generated) in &svc_files {
                    generation_owned_paths
                        .entry(*language)
                        .or_default()
                        .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                    language_output_paths.entry(*language).or_default().extend(
                        generated
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .map(|file| base_dir.join(&file.path)),
                    );
                }
                let report = pipeline::write_files_report(&svc_files, &base_dir)?;
                refusals.absorb_unwritten(&report);
                let svc_count = report.changed_count();
                written_count += svc_count;
                grand_service_api_count += svc_count;
                tracing::info!("Generated {svc_count} service API files");
                if svc_count > 0 {
                    any_written = true;
                    for (lang, generated) in &svc_files {
                        if generated
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            changed_languages.insert(*lang);
                        }
                    }
                }
            }
        }

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
                let api_match = !api_hashes.is_empty() && api_hashes.iter().all(|(p, h)| stored_api.get(p) == Some(h));

                for (_, files) in &public_api_files {
                    current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                }
                for (language, generated) in &public_api_files {
                    generation_owned_paths
                        .entry(*language)
                        .or_default()
                        .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                    language_output_paths.entry(*language).or_default().extend(
                        generated
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .map(|file| base_dir.join(&file.path)),
                    );
                }

                if !api_match || clean {
                    let report = pipeline::write_files_report(&public_api_files, &base_dir)?;
                    refusals.absorb_unwritten(&report);
                    let api_count = report.changed_count();
                    written_count += api_count;
                    grand_public_api_count += api_count;
                    tracing::info!("Generated {api_count} public API files");
                    any_written |= api_count > 0;
                    let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                    for (lang, generated) in &public_api_files {
                        if generated
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            changed_languages.insert(*lang);
                        }
                    }
                } else {
                    tracing::info!("  [public_api] up to date (skipping)");
                }
            }
        }

        let stub_files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
        if !stub_files.is_empty() {
            let stub_hashes: Vec<(String, String)> = stub_files
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

            for (_, files) in &stub_files {
                current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
            }
            for (language, generated) in &stub_files {
                generation_owned_paths
                    .entry(*language)
                    .or_default()
                    .extend(generated.iter().map(|file| base_dir.join(&file.path)));
                language_output_paths.entry(*language).or_default().extend(
                    generated
                        .iter()
                        .filter(|file| file.carries_alef_marker())
                        .map(|file| base_dir.join(&file.path)),
                );
            }

            if !stubs_match || clean {
                let report = pipeline::write_files_report(&stub_files, &base_dir)?;
                refusals.absorb_unwritten(&report);
                let stub_count = report.changed_count();
                written_count += stub_count;
                grand_stub_count += stub_count;
                tracing::info!("Generated {stub_count} type stub files");
                any_written |= stub_count > 0;
                let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);

                for (lang, generated) in &stub_files {
                    if generated
                        .iter()
                        .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                    {
                        changed_languages.insert(*lang);
                    }
                }
            } else {
                tracing::info!("  [stubs] up to date (skipping)");
            }
        }

        let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
        let report = pipeline::reconcile_managed_scaffold_manifests(&scaffold_files, &base_dir)?;
        let scaffold_count = report.changed_count();
        grand_scaffold_count += scaffold_count;
        if scaffold_count > 0 {
            any_written = true;
            // A scaffold-managed manifest (`packages/java/pom.xml`,
            // `crates/<name>-ffi/cmake/*.cmake`, `packages/python/pyproject.toml`) can change with
            // no corresponding bindings/service-api/public-api/stubs write -- e.g. a
            // `package_metadata.license` edit. Those phases are the only other place this loop
            // inserts into `changed_languages`, so without this a scaffold-only write left its
            // language out of `format_scope` below and the freshly written manifest shipped
            // unformatted. `alef all` never showed the defect because its whole-tree pass
            // reformats everything regardless of which phase wrote it. ~keep
            changed_languages.extend(pipeline::languages_owning_changed_paths(
                resolved_cfg,
                &base_dir,
                &languages,
                &report.changed_paths,
            ));
        }
        // `reconcile_managed_scaffold_manifests` silently drops a manifest it cannot
        // prove alef owns; this repair runs regardless, since a missing forwarded feature
        // is additive-only and safe even without that proof (see `scaffold::repair`). ~keep
        crate::scaffold::repair_missing_cfg_binding_features(&api, resolved_cfg, &languages);
        current_gen_paths.extend(pipeline::stampable_output_paths(&scaffold_files, &base_dir));

        tracing::info!("Running post-build processing...");
        // Post-build MUST run before the format pass below, not after: several
        // post-build steps (Swift's `MaterializeSwiftBridge`, Dart's
        // `flutter_rust_bridge_codegen`) write straight to disk, unguarded by
        // `write_files_report`, and a format pass that already ran before they wrote
        // never sees their output at all -- the file this run ships is whatever the
        // post-build tool produced, untouched by `poly fmt`. Stamping that output
        // afterward (as this function used to) embeds `alef:hash:` over UNFORMATTED
        // bytes: the moment anything reformats the file later -- this repo's own
        // whole-tree `converge_full_regen` on a later `alef all` run, or a standalone
        // `poly fmt --fix .` a consumer runs before committing -- the body no longer
        // matches its own embedded hash and `alef verify` reports it stale, even though
        // nothing about the *generation inputs* changed. `all_commands.rs`'s "All" arm
        // already runs post-build before its own format pass for exactly this reason;
        // this mirrors that ordering here. Surface refusals before a post-build error,
        // not after -- see the identical guard (and its full rationale) on
        // `all_commands.rs`'s "All" arm. ~keep
        // Deferred into `stage_failures` rather than `?`/`return Err`: a bare early return here
        // used to abort not just the rest of THIS crate's stages but every later crate in
        // `crates_to_process` too, and -- the defect this task exists to close -- it skipped the
        // single terminal `finalize_hashes(&current_gen_paths, ..)` call below entirely. Every
        // language's binding/service-API/public-API/stub output for THIS crate had already been
        // written with its header (no hash line yet, by the two-pass design `finalize_hashes`'s
        // own doc describes) before this post-build step ever runs, so skipping the stamp left
        // that output on disk permanently unstamped: a later run whose content matches (cache-hit)
        // never rewrites it and so never gets another chance to stamp it either. One backend's
        // post-build failure (a missing toolchain, a bad `flutter_rust_bridge_codegen` run) must
        // not orphan every other language's freshly-written output this way. Mirrors the identical
        // fix already shipped for `alef all` (`all_commands.rs`'s `stage_failures.record(...)` at
        // its own `complete_generated_artifacts` call site) -- see `StageFailures`'s module doc for
        // the general hazard and task #186 for the multi-crate half of it. ~keep
        if let Err(error) = complete_generated_artifacts(&languages, resolved_cfg, &base_dir, compile_policy) {
            stage_failures.record(&format!("[{}] post-build processing", resolved_cfg.name), error);
        }

        // Fold in every path a post-build step writes unguarded (see
        // `PostBuildStep::owned_paths`'s doc for why this can't be left to the
        // generator's own `GeneratedFile` output). Claimed on every run the step is
        // configured for, independent of whether the generator found fresh content
        // to emit for the same path this time -- that independence is the fix for the
        // alef #B incident: without it, a run where the generator legitimately emits
        // nothing for a path a post-build step still writes reads as "no longer
        // generated" to the orphan sweep below.
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
            generation_owned_paths
                .entry(language)
                .or_default()
                .extend(owned.iter().cloned());
            current_gen_paths.extend(owned);
        }
        // Deliberately NOT `finalize_hashes` here, unlike every other phase above: a
        // post-build-owned path (`RustBridgeC.h`) must stay UNSTAMPED until it has been
        // through the format pass below. Stamping it now -- over content the format pass
        // has not touched yet -- would embed `alef:hash:` while the body is still
        // whatever the post-build tool produced. `poly`'s built-in "hash-stamped
        // generated file" skip then protects that stamp from ever being reformatted
        // again (verified: once a file carries a well-formed `alef:hash:` line, `poly
        // fmt --fix` leaves it untouched), so the file would ship non-canonical forever
        // and never get a second chance to be formatted. The single `finalize_hashes`
        // call after the format pass below is what actually stamps these paths. ~keep

        // A post-build-owning language must be formatted even when nothing in
        // `files`/`stub_files` changed this run: Swift's `RustBridgeC.h` and Dart's FRB
        // bridge are written unguarded by post-build (see above), invisible to
        // `any_written`/`changed_languages`, and would otherwise never reach
        // `poly_paths`'s per-language directory scoping below. Mirrors
        // `any_output_changed`'s `languages_have_post_build_steps` seed in
        // `all_commands.rs`. ~keep
        let post_build_languages = languages_with_post_build_steps(&languages, resolved_cfg);
        if !post_build_languages.is_empty() {
            any_written = true;
            changed_languages.extend(post_build_languages);
        }

        let any_output_changed = any_written && !changed_languages.is_empty();
        // `any_output_changed` alone is the wrong gate for a tree an EARLIER run (a
        // pre-fix `alef generate`, or a standalone `alef scaffold`/`alef stubs` that
        // stamps its own output) left stamped and never formatted: nothing was written
        // THIS run, and the tree is still non-canonical. `generated_tree_needs_formatting`
        // answers "no" on a settled tree, so the fast path (skip formatting entirely) is
        // unchanged -- see `pipeline::format::stamp_gate`. ~keep
        if any_output_changed || pipeline::generated_tree_needs_formatting(&base_dir) {
            tracing::info!("Formatting generated files...");
            // Load-bearing, not defence in depth: removing the per-phase
            // `finalize_hashes` checkpoints above stops a FRESH run from stamping too
            // early, but a tree an EARLIER, pre-fix run already stamped-and-never-formatted
            // needs this too -- `write_files_report` compares hash-stripped bodies, so an
            // unchanged file is never rewritten and keeps the stamp it was given,
            // forever. Scope-symmetric by construction: the final `finalize_hashes`
            // call below re-stamps `current_gen_paths`, a superset of what is stripped
            // here, so no file is left carrying no hash line at all. ~keep
            let unstamped = pipeline::unstamp_before_formatting(&current_gen_paths);
            tracing::debug!("unstamped {unstamped} generated file(s) so the formatter can see them");
            // `changed_languages` is the right scope when something in THIS run's own
            // output actually changed -- narrower is faster and correct. It is the WRONG
            // scope when the gate fired only because `generated_tree_needs_formatting`
            // found a stale tree with nothing changed this run: `changed_languages` is
            // then empty, and an empty language set makes `format_generated_reporting` a
            // no-op (see `run_format_pass`), silently defeating the very pass this branch
            // exists to run. Fall back to every language this invocation resolved so the
            // healing case actually reformats something. ~keep
            let format_scope: std::collections::HashSet<_> = if any_output_changed {
                changed_languages.clone()
            } else {
                languages.iter().copied().collect()
            };
            // `strict`, not `false`: this pass formats `packages/<lang>` -- the SHIPPED
            // bindings -- and used to swallow every missing-formatter skip in a `warn!`
            // the caller never saw, so `--strict` guarded only the e2e formatter while
            // the more important surface went unguarded. ~keep
            pipeline::format_generated_reporting(resolved_cfg, &base_dir, Some(&format_scope), strict)?;
        }
        // Final stamp, after post-build AND formatting have both settled every byte this
        // run will ship -- see the ordering comment above `complete_generated_artifacts`
        // for why an intermediate stamp before formatting is not enough.
        //
        // Sweeping (not the plain path-tracked `finalize_hashes`), matching `all_commands.rs`'s
        // terminal stamp: a language `pipeline::generate` dropped as a per-language cache hit
        // contributes no files to `current_gen_paths` beyond whatever `read_lang_manifest`
        // recorded, and a run interrupted between an earlier write and this stamp (machine sleep,
        // a killed process) can leave a marker-carrying file on disk that no in-memory list this
        // run computed ever names. `generate_sweep_roots` -- computed here instead of at its
        // original call site further down, which needed it only for the orphan sweep -- rederives
        // every such file's hash straight from its own on-disk content, so neither gap can leave a
        // file permanently unstamped. ~keep
        let cleanup_roots = pipeline::generate_sweep_roots(&languages, lang.is_some(), resolved_cfg, &base_dir);
        pipeline::finalize_hashes_sweeping(&current_gen_paths, &cleanup_roots, &sources_hash, &alef_toml_bytes)?;
        // Records this crate's generation-inputs fingerprint centrally, once, now that
        // generation for it has completed successfully -- the replacement for folding
        // `inputs_hash` into every file's own stamp. See `core::hash`'s module doc and
        // `cache::generation_record`. ~keep
        cache::record_inputs_hash(
            &base_dir,
            &resolved_cfg.name,
            &crate::core::hash::compute_inputs_hash(&sources_hash, &alef_toml_bytes),
        )?;
        // This crate's run reached the point `record_inputs_hash` just marked as its
        // successful baseline -- clear the in-progress marker set above so it is
        // indistinguishable from a crate that was never interrupted at all. ~keep
        cache::generation_record::clear_generation_in_progress(&base_dir, &resolved_cfg.name)?;

        // Same check, same shared function and same deferral rationale as `all_commands.rs`'s
        // call site: `alef generate` writes the nested native-extension manifests too, so it can
        // leave exactly the same committed lock unresolvable and must stop exiting 0 over it.
        // Calling the one function rather than restating the rule is deliberate -- two sites
        // deriving "is this lock stale" independently is how the relock hook and the validator
        // came to disagree in the first place. Tolerating-variant, not the plain check: a
        // `test_apps`/e2e manifest requiring this crate's own not-yet-published version cannot
        // resolve until release and must warn, not fail -- see that function's doc. ~keep
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
        // Same check, same shared function and same deferral rationale as `all_commands.rs`'s
        // call site, for the Node ecosystem's equivalent: a generated `package.json` whose
        // specifiers a committed `pnpm-lock.yaml` no longer matches fails `pnpm install` under
        // the default frozen lockfile in CI just as surely as a stale `Cargo.lock` fails `cargo
        // build --locked`. Tolerating-variant, same rationale as the Cargo.lock check above. ~keep
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
        // Same check, same shared function and same deferral rationale as `all_commands.rs`'s
        // call site, for the Python/uv ecosystem's equivalent: a generated `pyproject.toml` whose
        // dependency specifiers a committed `uv.lock` no longer records fails `uv sync --locked`
        // under CI's default frozen lockfile just as surely as a stale `Cargo.lock` or
        // `pnpm-lock.yaml` fails their own frozen-install commands. Tolerating-variant, same
        // rationale as the Cargo.lock check above. ~keep
        if let Some(error) = pipeline::check_generated_uv_lock_freshness_tolerating_pending_publish(
            &current_gen_paths,
            Some(resolved_cfg),
        ) {
            stage_failures.record(&format!("[{}] generated uv.lock freshness", resolved_cfg.name), error);
        }

        let previous_generation_owned: std::collections::HashMap<_, _> = languages
            .iter()
            .map(|language| {
                (
                    *language,
                    cache::read_stage_paths(&resolved_cfg.name, &format!("generate-{language}-ownership")),
                )
            })
            .collect();
        for (language, previous_paths) in &previous_generation_owned {
            if !regenerated_languages.contains(language) {
                generation_owned_paths
                    .entry(*language)
                    .or_default()
                    .extend(previous_paths.iter().cloned());
            }
        }
        let cleanup_keep_paths: std::collections::HashSet<_> = generation_owned_paths
            .values()
            .flat_map(|paths| paths.iter().cloned())
            .collect();
        // `cleanup_roots` was already computed above, ahead of the terminal
        // `finalize_hashes_sweeping` call, which needed it first -- `generate_sweep_roots` is a
        // pure function of `languages`/`lang`/`resolved_cfg`/`base_dir`, none of which change
        // between there and here, so reusing the one value is correct, not merely convenient.
        let previous_paths: Vec<_> = previous_generation_owned.into_values().flatten().collect();
        // `cleanup_roots` doubles as the disk-scan candidate list: `sweep_manifest_orphans`
        // only actually scans a root once it has independently verified both `previous_paths`
        // and `cleanup_keep_paths` carry at least one entry under it (plus git-tracked-ness),
        // so a language this run skipped or whose bookkeeping is broken is refused, not
        // scanned -- see that function's doc for the measured evidence behind the gate. ~keep
        pipeline::sweep_manifest_orphans(&previous_paths, &cleanup_keep_paths, &cleanup_roots, &cleanup_roots)?;
        for (language, paths) in &generation_owned_paths {
            let paths: Vec<_> = paths.iter().cloned().collect();
            cache::write_stage_hash(
                &resolved_cfg.name,
                &format!("generate-{language}-ownership"),
                &sources_hash,
                &paths,
            )?;
        }
        for (language, paths) in language_output_paths {
            let paths: Vec<_> = paths.into_iter().collect();
            cache::write_lang_manifest(&resolved_cfg.name, &language.to_string(), &paths)?;
        }

        if let Err(e) = pipeline::sync_versions(resolved_cfg, config_path, None, true, true, None) {
            tracing::warn!("version sync failed: {e}");
        }

        if resolved_cfg.e2e.is_some() {
            // An [e2e] block is a correct, intentional configuration; this is advice on
            // the next command to run, not a problem with the current one. ~keep
            tracing::info!("[e2e] block detected — run 'alef e2e generate' to regenerate e2e test suites");
            crates_with_e2e += 1;
        }

        grand_total_generated += written_count;
        grand_binding_count += binding_count;
    }
    pipeline::report_refused_writes(&refusals);
    pipeline::report_user_owned_skips(&refusals);
    // Structured and always printed, zeros included, deliberately not folded into one flat
    // number: a per-language `alef generate --lang X` run touching a handful of files used to
    // print the exact same "Generated N files. exit 0" shape as a full regeneration, with no
    // signal that docs-site and e2e output -- both real categories a full `alef all` regen
    // produces -- were never in scope here at all. That gap caused a real misdiagnosis: a
    // 98-file per-language result was read as a full regen for a repo whose real one is ~278
    // files including `docs-site/src` and `e2e/`, both silently zero, because the exit code and
    // the one-line summary carried no information to contradict that reading. Printing every
    // category's count -- including zero -- every time, plus naming the categories this command
    // structurally cannot touch, is the "most honest thing" this run can do: a genuine no-op
    // regen (everything already current) legitimately reports zeros here too, so this is not a
    // pass/fail guard, it is the evidence a human or CI needs to tell the two apart themselves.
    // ~keep
    tracing::info!(
        binding_files = grand_binding_count,
        service_api_files = grand_service_api_count,
        public_api_files = grand_public_api_count,
        stub_files = grand_stub_count,
        scaffold_files = grand_scaffold_count,
        total_files = grand_total_generated,
        crates_processed = crates_to_process.len(),
        crates_with_e2e_configured = crates_with_e2e,
        "Generate summary: {grand_binding_count} binding, {grand_service_api_count} service-api, \
         {grand_public_api_count} public-api, {grand_stub_count} stub, {grand_scaffold_count} scaffold files \
         ({grand_total_generated} total). `alef generate` never generates docs or e2e/test-app output ({} of {} \
         processed crate(s) here have an [e2e] block configured) -- run `alef docs`, `alef e2e generate`, \
         or `alef all` for those.",
        crates_with_e2e,
        crates_to_process.len(),
    );
    // Every crate's write, format and finalize-hash phases already ran and already wrote their
    // output by the time we reach here, deferred post-build failures included -- see
    // `stage_failures`'s doc comment above the loop. Surfacing them now (instead of at the point
    // of failure) is what lets the run still exit non-zero and name everything that went wrong,
    // without withholding any of the regeneration this invocation could still do. ~keep
    stage_failures.into_result()?;
    Ok(None)
}
