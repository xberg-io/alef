//! `alef verify` -- the read-only staleness, completeness, and drift report.
//!
//! Split out of `core_commands.rs` rather than added to it: that file is over the
//! 1,000-line cap this repository sets for backend/codegen/CLI sources, and `verify` is a
//! self-contained concern (it shares nothing with the other command arms but the helper
//! module every arm uses). ~keep

use anyhow::Result;

use crate::cli::{cache, dispatch, pipeline};

use super::super::args::Commands;
use super::super::dispatch::DispatchContext;
use super::super::helpers::*;
use super::super::verify_orphans;

/// Run `alef verify`.
///
/// # Errors
///
/// Returns an error when configuration or extraction fails, and -- unless `report_only` --
/// when the verification itself finds drift.
pub(super) fn run(context: &DispatchContext, report_only: bool) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    let (_workspace, resolved) = load_config(config_path)?;
    let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
    // Two separate checks, not one: the embedded per-file `alef:hash:` is a pure content
    // hash (catches a hand-edit or reverted generated output), and the crate-scoped
    // `stale_crates` check below catches generation inputs (sources/alef.toml) moving since
    // the last generate. See `core::hash`'s module doc for why they were split. ~keep
    tracing::info!("Verifying alef-generated files (per-file content hash + crate-scoped inputs hash)");
    let base_dir = std::env::current_dir()?;

    let missing_snippet_roots: Vec<String> = crates_to_process
        .iter()
        .flat_map(|resolved_cfg| missing_snippet_directories(resolved_cfg, &base_dir))
        .collect();
    let has_missing_snippet_roots = !missing_snippet_roots.is_empty();

    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

    // One walk, two answers. `stale` is the verdict; `scan_coverage` is how much of the tree
    // that verdict is about. Deriving the second from a second walk would let the report
    // describe a file set the verdict never saw -- see `verify_coverage`'s module doc. ~keep
    let (scanned, scan_coverage) = super::super::verify_scan::scan(&base_dir);
    // A path `[workspace.ownership] user_owned` declares is dropped from the walk's RESULT, not
    // from the walk: `scan_coverage` still counts the file as opened, because it was. What is
    // removed is the verdict -- a declared path alef never rewrites cannot be held to a hash,
    // and a consumer's first hand-edit to one it stamped under an earlier release would
    // otherwise report it permanently stale with no reachable remedy. That is the same stable
    // bad state the declaration exists to end, arriving through the other door. ~keep
    let declared = pipeline::declared_user_owned(&base_dir)?;
    let scanned: Vec<_> = scanned
        .into_iter()
        .filter(|(path, _, _)| !declared.matches(&base_dir, path))
        .collect();
    let marked_paths: std::collections::HashSet<std::path::PathBuf> =
        scanned.iter().map(|(path, _, _)| path.clone()).collect();
    // Pure per-file content check -- see `crate::core::hash::compute_file_hash`'s doc for why
    // it takes no generation-inputs argument. This catches a hand-edit to any generated file;
    // it does NOT by itself catch a crate whose sources/alef.toml changed but whose generated
    // bytes happen to be unaffected -- that coarser question is `stale_crates`, below. ~keep
    let stale = stale_among(&scanned);

    // Crate-scoped generation-inputs staleness -- the replacement for what the old per-file
    // hash used to (over-)report: whether each crate's Rust sources + alef.toml still match
    // what `cache::generation_record` recorded at its last successful `alef generate`/`alef
    // all` run. A crate with no recorded baseline yet (every crate in every consumer repo,
    // immediately after upgrading to this version, before the next generate) is silently
    // skipped rather than reported -- see `cache::generation_record`'s module doc. ~keep
    let current_inputs_hashes: Vec<(String, String)> = crates_to_process
        .iter()
        .filter_map(|c| {
            let sources_hash = cache::sources_hash(&c.sources).ok()?;
            let inputs_hash = crate::core::hash::compute_inputs_hash(&sources_hash, &alef_toml_bytes);
            Some((c.name.clone(), inputs_hash))
        })
        .collect();
    // Crates whose last `alef generate`/`alef all` run was interrupted before it finished --
    // see `cache::generation_record`'s in-progress marker doc (alef#268) for why this is the
    // right question to ask BEFORE reading `stale_crates`/`missing_generated_files` below: an
    // interrupted `--clean` run can leave a crate with fewer files than it started with, and
    // that absence must be diagnosed as "the run didn't finish", never as ordinary staleness a
    // rerun coincidentally happens to also fix. ~keep
    let incomplete_crates: Vec<String> =
        cache::generation_record::incomplete_crate_names(&base_dir, crates_to_process.iter().map(|c| c.name.as_str()));
    let has_incomplete_crates = !incomplete_crates.is_empty();

    let stale_crates: Vec<String> = cache::stale_crate_names(
        &base_dir,
        current_inputs_hashes
            .iter()
            .map(|(name, hash)| (name.as_str(), hash.as_str())),
    )
    .into_iter()
    // An incomplete crate is already reported under its own heading below; folding it into
    // "stale" too would tell the operator to run the same remedy twice under two different
    // diagnoses for the same crate. ~keep
    .filter(|name| !incomplete_crates.contains(name))
    .collect();
    let has_stale_crates = !stale_crates.is_empty();

    let mut snippet_coverage_issues = Vec::new();
    // `verify_walk_multi` only sees files that already exist on disk; a file
    // generation would now produce but that was never written (a backend
    // that emits one file per public type, an item added since the last
    // regen) is invisible to it. Closing that requires knowing what
    // generation would produce, so every crate pays a regeneration pass
    // here (mirrors `alef diff`) to find files entirely absent from disk, and
    // -- in the same pass -- files that exist but were never marked and so
    // can never be written by a plain `alef generate` (frozen; see
    // `FrozenFile`). ~keep
    let mut missing_generated_files: Vec<String> = Vec::new();
    // Absent AND gitignored: `alef generate` cannot close this gap the way it closes a
    // plain `missing_generated_files` entry -- see `MissingAndFrozenFiles::missing_gitignored`. ~keep
    let mut missing_gitignored_generated_files: Vec<String> = Vec::new();
    let mut frozen_generated_files: Vec<FrozenFile> = Vec::new();
    // Unioned across every crate before the orphan diff runs below: a file legitimately
    // owned by crate B must never look orphaned merely because crate A's own managed
    // surface doesn't mention it. See `verify_orphans::find_orphaned_generated_files`. ~keep
    let mut all_managed_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    // Debt `collect_managed_surface` tolerated while still building the rest of
    // the surface (currently only the e2e stages' deferred strict-assertion
    // failure). `alef verify` is read-only and has no target to excuse a stage
    // failure the way `alef adopt` can, so every one of these is collected and
    // reported below rather than silently absorbed into a clean-looking zero --
    // see `collect_managed_surface`'s doc for why dropping this list is exactly
    // the bug this return shape exists to prevent. ~keep
    let mut stage_failures: Vec<String> = Vec::new();
    // Informational only -- see `pipeline::generate::scaffold_drift`'s module doc for what
    // this checks, its stated false-positive/false-negative shape, and why it never
    // contributes to `has_stage_failures` or any other hard-fail condition below. ~keep
    let mut create_once_template_drift: Vec<String> = Vec::new();
    // Paths `[crates.verify].ignore_ephemeral` matched and dropped from `missing`/
    // `missing_gitignored` below, one count per crate. Reported unconditionally alongside
    // `alef verify`'s other coverage facts -- see the `VerifyCoverage::measure` call further
    // down and `VerifyConfig`'s module doc for why a run that narrowed its own scope must say
    // so rather than silently passing. ~keep
    let mut ephemeral_excluded_count = 0usize;
    for resolved_cfg in &crates_to_process {
        let languages = resolve_languages(resolved_cfg, None)?;
        let api = pipeline::extract(resolved_cfg, config_path, false)?;
        let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
        create_once_template_drift.extend(
            pipeline::find_create_once_template_drift(&scaffold_files, &base_dir)
                .into_iter()
                .map(|path| format!("[{}] {}", resolved_cfg.name, path.display())),
        );
        let found = find_missing_and_frozen_generated_files(&languages, &api, resolved_cfg, config_path, &base_dir)?;
        // `managed_paths` is what generation would produce for this crate's CURRENT sources
        // and config -- computed fresh in memory, independent of whatever the last run left on
        // disk -- so it stays correct and worth unioning into the orphan diff even for a crate
        // whose last run was interrupted. `missing`/`missing_gitignored`/`frozen`/
        // `stage_failures` below are all disk-vs-surface comparisons, which is exactly what an
        // interrupted run corrupts; those are skipped for an incomplete crate and reported
        // under their own "did not complete" heading instead. ~keep
        all_managed_paths.extend(found.managed_paths);
        if incomplete_crates.contains(&resolved_cfg.name) {
            tracing::debug!(
                crate_name = %resolved_cfg.name,
                "skipping missing/frozen/stage-failure reporting for this crate: its last \
                 generation run did not complete"
            );
        } else {
            // `ignore_ephemeral` only ever narrows `missing`/`missing_gitignored` -- both are
            // exclusively about paths ABSENT from disk. `frozen` (which feeds the orphan diff)
            // is untouched: that check is about the correctness of bytes that already exist,
            // which "this output is ephemeral" says nothing about. ~keep
            let (missing, missing_excluded) = resolved_cfg.verify.partition_ephemeral(found.missing, &base_dir);
            let (missing_gitignored, gitignored_excluded) = resolved_cfg
                .verify
                .partition_ephemeral(found.missing_gitignored, &base_dir);
            ephemeral_excluded_count += missing_excluded + gitignored_excluded;
            missing_generated_files.extend(missing);
            missing_gitignored_generated_files.extend(missing_gitignored);
            frozen_generated_files.extend(found.frozen);
            stage_failures.extend(
                found
                    .stage_failures
                    .into_iter()
                    .map(|failure| format!("[{}] {failure}", resolved_cfg.name)),
            );
        }

        let Some(e2e_config) = &resolved_cfg.e2e else {
            continue;
        };
        if let Err(error) = crate::e2e::verify_fresh_snippet_coverage(
            &base_dir,
            resolved_cfg,
            e2e_config,
            &api.types,
            &api.enums,
            &api.functions,
        ) {
            snippet_coverage_issues.push(format!("[{}] {error:#}", resolved_cfg.name));
        }
    }
    missing_generated_files.sort();
    missing_generated_files.dedup();
    missing_gitignored_generated_files.sort();
    missing_gitignored_generated_files.dedup();
    frozen_generated_files.sort_by(|a, b| a.path.cmp(&b.path));
    frozen_generated_files.dedup_by(|a, b| a.path == b.path);
    stage_failures.sort();
    stage_failures.dedup();
    create_once_template_drift.sort();
    create_once_template_drift.dedup();
    let has_stage_failures = !stage_failures.is_empty();
    let has_missing_files = !missing_generated_files.is_empty();
    let has_missing_gitignored_files = !missing_gitignored_generated_files.is_empty();
    // Only a frozen file that `alef adopt --write` will actually ACCEPT may gate the exit code.
    // A create-once seed's missing marker is deliberate, not drift: the write guard refuses it by
    // design, a plain `alef generate` leaves it untouched, and adopting it needs the deliberate
    // `--clobber-create-once-seeds`. Counting those made `alef verify` unable to reach exit 0 on
    // repos carrying legacy pre-marker files -- no amount of regeneration cleared them -- which
    // turned the release gate into something operators had to route around with a dangerous flag.
    // `create_once_template_drift` is already informational-only for the same reason. ~keep
    let has_adoptable_frozen_files =
        crate::bin_cli::helpers::frozen::has_adoptable_frozen_files(&frozen_generated_files);
    // Report-only: see `verify_orphans`'s module doc for why this never deletes.
    log_managed_surface(&all_managed_paths);
    let orphan_generated_files = verify_orphans::find_orphaned_generated_files(&base_dir, &all_managed_paths);
    let has_orphan_files = !orphan_generated_files.is_empty();

    // Catches the cross-artifact ABI straddle a per-file hash check cannot
    // see: an FFI header and a binding backend's opaque-handle file each
    // individually fresh against current inputs, but stamped by two
    // different handle-ABI generations because only one side was
    // regenerated. See `crate::core::hash::HANDLE_ABI_STAMP_KEY` and
    // `find_stamp_disagreement` for why 0/1 distinct values is silently
    // fine and only 2+ is reported. ~keep
    let abi_disagreement = find_stamp_disagreement(&base_dir, crate::core::hash::HANDLE_ABI_STAMP_KEY);
    let has_abi_disagreement = abi_disagreement.is_some();
    if let Some(disagreement) = &abi_disagreement {
        crate::bin_cli::output::line(format_args!(
            "ABI generation disagreement detected for `{}`:",
            disagreement.key
        ));
        for (path, value) in &disagreement.examples {
            crate::bin_cli::output::line(format_args!("  {path} -> {value}"));
        }
    }

    // Dart's FRB `frb_generated.rs` is written by `PostBuildStep::RunCommand` (an external
    // `flutter_rust_bridge_codegen` invocation) and then rewritten in place by
    // `PostBuildStep::CarryFrbCfgGates` -- neither goes through the guarded
    // `write_files_report` path that stamps alef's own embedded `alef:hash:` marker, so the
    // per-file `stale` walk above never even looks at this file: it is structurally invisible
    // to it, not merely skipped. alef #179 was exactly this -- the file could silently drift
    // (an unformatted `alef build` regeneration disagreeing with a formatted `alef generate`
    // one, or `lib.rs`'s `#[cfg(...)]` gates falling out of sync) and `alef verify` reported
    // nothing. This recomputes the same canonical form `CarryFrbCfgGates` itself would write --
    // see `pipeline::canonical_frb_generated` -- and flags a difference as drift. It cannot
    // prove the wire dispatch bodies still match the current API surface (that needs the real
    // `flutter_rust_bridge_codegen` tool, which a fast read-only `verify` must not shell out
    // to); it only proves the file is not a fixed point of the transform that already governs
    // it. ~keep
    let mut all_frb_generated_drift: Vec<String> = Vec::new();
    for resolved_cfg in &crates_to_process {
        all_frb_generated_drift.extend(frb_generated_drift(resolved_cfg, &base_dir));
    }
    let has_frb_generated_drift = !all_frb_generated_drift.is_empty();
    if has_frb_generated_drift {
        crate::bin_cli::output::line(
            "Dart FRB bridge drift detected (frb_generated.rs is not the canonical form \
             `CarryFrbCfgGates` would write for the current lib.rs -- run `alef generate` to \
             regenerate and reformat it):",
        );
        for path in &all_frb_generated_drift {
            crate::bin_cli::output::line(format_args!("  {path}"));
        }
    }

    let mut all_version_mismatches: Vec<String> = Vec::new();
    for resolved_cfg in &crates_to_process {
        let mismatches = pipeline::verify_versions(resolved_cfg)?;
        all_version_mismatches.extend(mismatches);
    }
    let has_version_issues = !all_version_mismatches.is_empty();
    if has_version_issues {
        crate::bin_cli::output::line("Version mismatches detected:");
        for mismatch in &all_version_mismatches {
            crate::bin_cli::output::line(format_args!("  {mismatch}"));
        }
    }
    // The consumer's vendored copy of alef's own `alef.toml` JSON Schema, if they keep one. It
    // is not a generated binding and nothing here writes it -- see `verify_schema`'s module doc
    // for why this reports only, why it speaks only about a file that already exists at the path
    // `alef schema` defaults to, and why only a difference in the described config surface (never
    // a version stamp alone) is allowed to gate the exit code below. ~keep
    let vendored_schema =
        super::super::verify_schema::find_stale_vendored_schema(&base_dir, crate::cli::version_pin::cli_version());
    let has_schema_surface_drift = vendored_schema
        .as_ref()
        .is_some_and(|finding| finding.describes_a_different_surface());
    if let Some(finding) = &vendored_schema {
        for line in finding.report_lines() {
            crate::bin_cli::output::line(line);
        }
    }
    if !snippet_coverage_issues.is_empty() {
        crate::bin_cli::output::line("Snippet coverage issues detected:");
        for issue in &snippet_coverage_issues {
            crate::bin_cli::output::line(format_args!("  {issue}"));
        }
    }
    // Deliberately not folded into `snippet_coverage_issues` above: that list is about
    // generated-snippet coverage ledgers being fresh, an entirely different question from
    // whether the roots naming them exist at all. ~keep
    if has_missing_snippet_roots {
        crate::bin_cli::output::line(
            "Configured docs.snippets roots that do not exist (every snippet check that walks \
             these passes having examined nothing -- fix the dirs/inline_dirs entry or create \
             the directory):",
        );
        for directory in &missing_snippet_roots {
            crate::bin_cli::output::line(format_args!("  {directory}"));
        }
    }

    // Informational only, printed unconditionally and never folded into the "up to
    // date"/failure gates below: see `pipeline::generate::scaffold_drift`'s module doc.
    // A create-once file differing from its template is the expected steady state for a
    // hand-maintained file -- this only fires when the file's own git history rules out a
    // consumer edit as the explanation, and even then there is no rerun that fixes it;
    // only a human reviewing the current template can decide what to do. ~keep
    if !create_once_template_drift.is_empty() {
        crate::bin_cli::output::line(
            "Create-once scaffold files that predate a template fix (informational -- these are \
             user-owned after their first write, so alef never rewrites them; review the current \
             template and hand-port the fix if it applies to your copy):",
        );
        for path in &create_once_template_drift {
            crate::bin_cli::output::line(format_args!("  {path}"));
        }
    }

    // The `verify` half of the escalation `cache::untracked_required_records`
    // documents: write commands warn and keep going, verification must refuse. The
    // query is already silent outside a git work tree and for a record that does not
    // exist yet, so this never fires where "untracked" is unanswerable, nor on the
    // run that legitimately creates the record. ~keep
    let untracked_records = cache::untracked_required_records(&base_dir);
    if !untracked_records.is_empty() {
        crate::bin_cli::output::line(
            "Required alef records are not tracked by git (alef writes these and depends on them \
         being committed):",
        );
        for record in &untracked_records {
            crate::bin_cli::output::line(format_args!("  {record} -- fix with: git add {record}"));
        }
    }

    // Printed unconditionally, before the verdict and on every run including a clean one.
    // Every finding above is a NEGATIVE claim, and a report made only of negative claims is
    // indistinguishable from one that examined nothing -- which is exactly how consumer CI
    // came to read a green `alef verify` as a whole-tree freshness gate when it is a claim
    // about marker-carrying files only. See `verify_coverage`'s module doc. ~keep
    let unmarked_seeds = crate::bin_cli::helpers::frozen::unmarked_create_once_seeds(&frozen_generated_files);
    let drifted_seeds = crate::bin_cli::helpers::frozen::drifted_frozen_seeds(&frozen_generated_files);
    // Printed unconditionally, ABOVE the verdict, and deliberately outside the exit-code gate
    // below.
    //
    // Not fatal, and the cost of that choice is stated rather than hidden: a consumer whose
    // frozen file is benign today would start failing CI on the upgrade that added this check,
    // for a condition that predates the release and that no rerun of `alef generate` clears --
    // which is precisely the trap `has_adoptable_frozen_files` was narrowed to escape when
    // create-once seeds made verify unable to reach exit 0 at all. Buying strictness by
    // re-breaking every consumer's gate is how a check gets routed around, and a routed-around
    // check reports nothing.
    //
    // What replaces the exit code is that this can no longer be quiet. It prints on every run
    // including a clean one, it names each file rather than counting it, and its count is
    // restated in the coverage block, so it is visible to a reader of a PASSING run -- unlike
    // the write-time refusal tally, which only ever appeared in a generate/build log nobody
    // reads after the fact. A consumer who wants it fatal has a precise, local gate: declare
    // the path `user_owned` (which removes it) or grep this heading in CI. ~keep
    for line in crate::bin_cli::helpers::frozen::drifted_seed_report_lines(&frozen_generated_files) {
        crate::bin_cli::output::line(line);
    }
    // Measured from the SAME managed surface the rest of this report is measured from, and with
    // the same matcher the write guards use, so the number cannot describe a different file set
    // than the one alef actually exempted. ~keep
    let declared_user_owned_paths: Vec<&std::path::PathBuf> = all_managed_paths
        .iter()
        .filter(|path| declared.matches(&base_dir, path))
        .collect();
    let declared_user_owned_count = declared_user_owned_paths.len();
    for path in &declared_user_owned_paths {
        tracing::debug!(
            "declared user-owned by [workspace.ownership] user_owned (contents not verified): {}",
            path.display()
        );
    }
    // Paths at debug level, count in the report: one line per seed is 72 lines on a measured
    // consumer tree, and there is no action any of them prompts -- but the count must never be
    // invisible, which is what reporting them only inside the failure block amounted to. ~keep
    for path in &unmarked_seeds {
        tracing::debug!("unmarked create-once seed (contents not verified): {path}");
    }
    for line in super::super::verify_coverage::VerifyCoverage::measure(
        &all_managed_paths,
        &marked_paths,
        scan_coverage,
        unmarked_seeds.len(),
        drifted_seeds.len(),
        ephemeral_excluded_count,
        declared_user_owned_count,
    )
    .report_lines()
    {
        crate::bin_cli::output::line(line);
    }

    if stale.is_empty()
        && !has_stale_crates
        && !has_missing_files
        && !has_missing_gitignored_files
        && !has_adoptable_frozen_files
        && !has_orphan_files
        && !has_abi_disagreement
        && !has_frb_generated_drift
        && !has_version_issues
        && snippet_coverage_issues.is_empty()
        && untracked_records.is_empty()
        && !has_stage_failures
        && !has_missing_snippet_roots
        && !has_incomplete_crates
        && !has_schema_surface_drift
    {
        // Derived FROM `drifted_seeds` -- the same finding set `drifted_seed_report_lines`
        // (printed above, unconditionally) reports -- not a second, independently-maintained
        // condition. See `frozen::report_sign_off_line`'s doc: this used to be a
        // bare literal here, so a run that had just printed the DRIFTED block a few lines above
        // went on to assert the unqualified opposite three lines later and exited 0. ~keep
        crate::bin_cli::output::line(crate::bin_cli::helpers::frozen::report_sign_off_line(
            drifted_seeds.len(),
        ));
    } else {
        // Printed first and named distinctly from every finding below: for these crates, the
        // remaining sections describe an interrupted run's expected shape (files a `--clean`
        // pass removed but never got to rewrite), not staleness a later source/config change
        // caused. Reporting them as ordinary "missing"/"stale" would send an operator hunting
        // for what changed upstream when the real cause is simpler -- the run never finished.
        // See `cache::generation_record`'s in-progress marker doc (alef#268). ~keep
        if has_incomplete_crates {
            crate::bin_cli::output::line(
                "The last generation run did not complete for the following crate(s) -- rerun \
                 `alef all`/`alef generate` for them. This is not staleness; other findings below \
                 for these crates may be artifacts of the unfinished run:",
            );
            for name in &incomplete_crates {
                crate::bin_cli::output::line(format_args!("  {name}"));
            }
        }
        if !stale.is_empty() {
            crate::bin_cli::output::line("Stale bindings detected:");
            for s in &stale {
                crate::bin_cli::output::line(format_args!("  {}", s.path));
                if tracing::enabled!(tracing::Level::DEBUG) {
                    crate::bin_cli::output::line(format_args!("    embedded:  {}", s.embedded));
                    crate::bin_cli::output::line(format_args!("    computed:  {}", s.computed));
                }
            }
        }
        // Distinct from `stale` above: a hand-edit to a file's own bytes versus a crate whose
        // generation inputs no longer match what it was last generated against. Named
        // separately because the remedy differs from ordinary stale bindings only in scope --
        // rerun `alef generate`/`alef all` for the crate and let it discover whether any
        // output byte actually changed. ~keep
        if has_stale_crates {
            crate::bin_cli::output::line(
                "Crates whose generation inputs (Rust sources or alef.toml) changed since their \
                 last successful `alef generate`/`alef all` run (rerun to see which files, if any, \
                 actually changed):",
            );
            for name in &stale_crates {
                crate::bin_cli::output::line(format_args!("  {name}"));
            }
        }
        if has_missing_files {
            crate::bin_cli::output::line("Missing generated files detected:");
            for path in &missing_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Distinct from `has_missing_files` above on purpose: `alef generate` is not a
        // remedy here, it is the failure mode -- the file gets written, then discarded by
        // the ignore rule before it can ever be committed, and the next `alef verify` finds
        // it "missing" again. Naming the correct fix (narrow the .gitignore rule, then
        // commit) is the entire point of splitting this out from plain "missing" instead of
        // folding it into the same heading with the same generate-and-rerun remedy. ~keep
        if has_missing_gitignored_files {
            crate::bin_cli::output::line(
                "Missing generated files that are also gitignored detected (running `alef generate` \
             cannot fix these -- the file would be written, then discarded by the matching \
             .gitignore rule before it can be committed; narrow the ignore rule for each path \
             below, then commit the file):",
            );
            for path in &missing_gitignored_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Reported separately from stale/missing, never folded into either
        // count: the remedy is different (a human must review and adopt or
        // delete the file -- `alef generate` alone cannot fix it) and folding
        // it in would make a frozen file look like ordinary drift. ~keep
        //
        // Create-once seeds are deliberately absent from this heading -- their count is stated
        // in the coverage report above instead, every run. See `frozen::report_lines`. ~keep
        for line in crate::bin_cli::helpers::frozen::report_lines(&frozen_generated_files) {
            crate::bin_cli::output::line(line);
        }
        // Report-only, never auto-deleted: see `verify_orphans`'s module doc for the
        // asymmetry between a missed report (status quo) and a wrong deletion
        // (unrecoverable). Folded into the hard-fail exit code below anyway, same as
        // frozen files, so CI actually surfaces a dropped emit instead of staying green
        // forever -- which is the exact failure mode that let Java's visitor files sit
        // as invisible orphans across releases. ~keep
        if has_orphan_files {
            crate::bin_cli::output::line(
                "Orphaned generated files detected (alef's marker is present but this run's backends \
             would not produce these paths: a dropped emit, a generation-config change, or a \
             create-once seed alef writes only when absent). Review each and delete by hand if \
             genuinely stale; alef never deletes automatically:",
            );
            for path in &orphan_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Not folded into missing/frozen: this is debt `collect_managed_surface`
        // hit while building the surface those two lists come from, not a
        // conclusion drawn *from* the surface. Naming it separately is what makes
        // a report that hit this debt distinguishable from one that genuinely
        // found nothing wrong -- a missing section here would look identical to
        // a clean run. ~keep
        if has_stage_failures {
            crate::bin_cli::output::line(
                "Generation debt detected while collecting the managed surface (missing/frozen \
             files above are still accurate; this is additional, separate debt):",
            );
            for failure in &stage_failures {
                crate::bin_cli::output::line(format_args!("  {failure}"));
            }
        }
    }
    super::super::verify_outcome::ensure_success(
        !stale.is_empty()
            || has_stale_crates
            || has_missing_files
            || has_missing_gitignored_files
            || has_adoptable_frozen_files
            || has_orphan_files
            || has_abi_disagreement
            || has_frb_generated_drift
            || has_stage_failures
            || has_schema_surface_drift,
        has_version_issues,
        snippet_coverage_issues.len(),
        report_only,
    )?;
    super::ensure_required_records_tracked(&untracked_records, report_only)?;
    super::ensure_generation_completed(&incomplete_crates, report_only)?;
    ensure_configured_snippet_directories_exist(&missing_snippet_roots, report_only)?;
    Ok(None)
}

/// Dump, at debug level, the exact path set the orphan report is diffed against.
///
/// An orphan finding is a *difference* between two sets, and only one of them is printed: the
/// report names the files on disk and says nothing about the surface they were missing from.
/// That makes the two most common explanations indistinguishable from the output alone -- a
/// genuinely dropped emit, versus a managed surface that came back short because a stage failed
/// or because the run was language-filtered -- and it is why an orphan report and `alef generate`'s
/// own "unrecorded alef-marked file" warning can name different files without either being wrong:
/// they are diffs against different sets (`collect_managed_surface` over every configured
/// language here, versus this run's recorded output under one sweep root there, git-tracked-only
/// and manifest-gated).
///
/// Printing the surface is what makes that difference checkable instead of arguable: run
/// `alef verify -vv`, diff this list against the paths `alef generate` reports, and the gap names
/// itself. Debug level, not info: it is one line per managed file on a consumer tree. ~keep
fn log_managed_surface(managed_paths: &std::collections::HashSet<std::path::PathBuf>) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    let mut paths: Vec<&std::path::PathBuf> = managed_paths.iter().collect();
    paths.sort();
    tracing::debug!(
        "managed surface the orphan report is diffed against: {} path(s)",
        paths.len()
    );
    for path in paths {
        tracing::debug!("  managed: {}", path.display());
    }
}

/// `config`'s Dart FRB `frb_generated.rs`, as a one-line drift report (`[<crate>] <path>`), if
/// its on-disk content is not the canonical form `PostBuildStep::CarryFrbCfgGates` would write
/// for the current `lib.rs`. Empty when the crate does not target Dart's FRB style, or when
/// either file is missing (nothing generated yet is not drift -- `missing_generated_files`
/// covers files a plain `alef generate` can create; this file cannot be, since alef only ever
/// rewrites one flutter_rust_bridge_codegen already produced). See [`run`]'s call site. ~keep
fn frb_generated_drift(config: &crate::core::config::ResolvedCrateConfig, base_dir: &std::path::Path) -> Vec<String> {
    let Some((lib_rs_path, frb_generated_path)) = crate::backends::dart::frb_rust_facade_paths(config) else {
        return Vec::new();
    };
    let lib_rs_path = base_dir.join(lib_rs_path);
    let frb_generated_path = base_dir.join(frb_generated_path);
    let (Ok(lib_rs), Ok(frb_generated)) = (
        std::fs::read_to_string(&lib_rs_path),
        std::fs::read_to_string(&frb_generated_path),
    ) else {
        return Vec::new();
    };

    let canonical = pipeline::canonical_frb_generated(&lib_rs, &frb_generated, &frb_generated_path);
    if canonical == frb_generated {
        return Vec::new();
    }
    vec![format!("[{}] {}", config.name, frb_generated_path.display())]
}

/// The crate's configured `docs.snippets` roots that are not on disk, as
/// `<crate>: <configured entry> (resolved to <absolute path>)`.
///
/// `alef verify` had no opinion on this at all: it checks generated-file hashes and
/// generated-snippet coverage-ledger freshness, neither of which asks whether the roots those
/// snippets are configured to live in exist. A `dirs`/`inline_dirs` entry pointing at a path
/// that was renamed or never created is real config drift, and every snippet check that walks
/// it reports a clean run having examined nothing.
///
/// `alef all` already refuses the same condition (`docs::build_snippet_context`), but only as a
/// docs-stage error -- and `verify` reaches that same stage through
/// `find_missing_and_frozen_generated_files`, where a stage error is deliberately downgraded to
/// a debug log so an unrelated docs failure cannot fail an ownership question. So the condition
/// was already being detected during `verify` and then discarded. This asks the question
/// directly instead of trying to recover it from a swallowed stage error.
///
/// `exclude` is deliberately not applied, matching `build_snippet_context`: a root that is
/// excluded from discovery is still a root the configuration claims exists. ~keep
fn missing_snippet_directories(
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<String> {
    let Some(snippets) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Vec::new();
    };
    snippets
        .dirs
        .iter()
        .chain(&snippets.inline_dirs)
        .filter_map(|dir| {
            let resolved = base_dir.join(dir);
            (!resolved.exists()).then(|| {
                format!(
                    "[{}] {} (resolved to {})",
                    config.name,
                    dir.display(),
                    resolved.display()
                )
            })
        })
        .collect()
}

/// Fail `alef verify` when a configured snippet root does not exist.
///
/// Kept out of [`super::super::verify_outcome::ensure_success`] for the same reason
/// [`super::ensure_required_records_tracked`] is: nothing here is stale and nothing regenerates
/// it, so "generated bindings, versions, or snippet coverage are out of date" would name the
/// wrong cause and the wrong fix. `report_only` short-circuits after the caller has already
/// printed the roots, matching how every other verify failure downgrades to a report. ~keep
fn ensure_configured_snippet_directories_exist(missing: &[String], report_only: bool) -> Result<()> {
    if report_only || missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "configured docs.snippets roots do not exist: {}. Fix: correct the dirs/inline_dirs \
         entries in alef.toml, or create the directories -- until then every snippet check that \
         walks them reports a clean run having examined nothing",
        missing.join(", ")
    )
}

#[cfg(test)]
mod frb_generated_drift_tests {
    use super::frb_generated_drift;

    /// A minimal single-crate, Dart-targeting config, resolved the same way
    /// `backends::order_invariance_tests::every_language_config` builds its fixture.
    fn dart_config(name: &str) -> crate::core::config::ResolvedCrateConfig {
        let toml_text = format!(
            "[workspace]\nlanguages = [\"dart\"]\n\n[[crates]]\nname = \"{name}\"\nsources = [\"src/lib.rs\"]\n"
        );
        let cfg: crate::core::config::new_config::NewAlefConfig =
            toml::from_str(&toml_text).expect("test config must parse");
        cfg.resolve().expect("test config must resolve").remove(0)
    }

    /// Writes `lib_rs`/`frb_generated_rs` at the exact paths `frb_rust_facade_paths` computes for
    /// `config`, rooted under `base_dir`. Returns the `frb_generated.rs` path actually written, so
    /// callers can build a canonical form against it with `pipeline::canonical_frb_generated`.
    fn write_frb_fixture(
        config: &crate::core::config::ResolvedCrateConfig,
        base_dir: &std::path::Path,
        lib_rs: &str,
        frb_generated_rs: &str,
    ) -> std::path::PathBuf {
        let (lib_rs_path, frb_generated_path) =
            crate::backends::dart::frb_rust_facade_paths(config).expect("dart FRB style always has facade paths");
        let lib_rs_path = base_dir.join(lib_rs_path);
        let frb_generated_path = base_dir.join(frb_generated_path);
        std::fs::create_dir_all(lib_rs_path.parent().expect("facade path has a parent dir"))
            .expect("create facade src dir");
        std::fs::write(&lib_rs_path, lib_rs).expect("write facade lib.rs");
        std::fs::write(&frb_generated_path, frb_generated_rs).expect("write frb_generated.rs");
        frb_generated_path
    }

    const LIB_RS: &str = "pub fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n";
    const RAW_FROM_TOOL: &str = "use flutter_rust_bridge::for_generated::{transform_result_dco, Lifetimeable, \
                                  Lockable};\nfn wire__crate__add_impl() {}\n";

    /// alef #179: before `frb_generated_drift` existed, `alef verify` had no opinion at all on a
    /// `frb_generated.rs` left in the tool's raw, unformatted form by `alef build` -- the file
    /// carries no alef marker, so the per-file `stale` walk never even looks at it. This proves
    /// the new check actually catches that case instead of silently agreeing with it.
    #[test]
    fn flags_unformatted_frb_generated_as_drift() {
        // `frb_generated_drift` reaches `format_rust_content` (via `canonical_frb_generated`),
        // which resolves rustfmt's `--config-path` against `std::env::current_dir()` --
        // process-global state shared across every test thread in this binary. Without this
        // lock, a sibling test that legitimately chdirs mid-run (`test_support::CwdGuard`) can
        // point rustfmt at a directory that no longer exists, making rustfmt fail and silently
        // fall back to unformatted output -- turning this test's real assertion into a false
        // failure unrelated to `frb_generated_drift` itself. See `test_support` module docs. ~keep
        let _cwd_lock = crate::test_support::CWD_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        if !crate::cli::pipeline::is_tool_available("rustfmt") {
            return;
        }
        let config = dart_config("sample-lib");
        let base_dir = tempfile::tempdir().expect("tempdir");
        write_frb_fixture(&config, base_dir.path(), LIB_RS, RAW_FROM_TOOL);

        let drift = frb_generated_drift(&config, base_dir.path());
        assert_eq!(
            drift.len(),
            1,
            "unformatted frb_generated.rs must be reported as drift: {drift:?}"
        );
    }

    /// The converse of the above: once the file already holds the canonical form
    /// `CarryFrbCfgGates` would write, the check must be silent -- otherwise every `alef verify`
    /// on an up-to-date tree would falsely report drift forever.
    #[test]
    fn is_silent_once_frb_generated_is_canonical() {
        // Same cwd-race guard as `flags_unformatted_frb_generated_as_drift` above -- see that
        // test's comment. ~keep
        let _cwd_lock = crate::test_support::CWD_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        if !crate::cli::pipeline::is_tool_available("rustfmt") {
            return;
        }
        let config = dart_config("sample-lib");
        let base_dir = tempfile::tempdir().expect("tempdir");
        let frb_generated_path = write_frb_fixture(&config, base_dir.path(), LIB_RS, RAW_FROM_TOOL);
        let canonical = crate::cli::pipeline::canonical_frb_generated(LIB_RS, RAW_FROM_TOOL, &frb_generated_path);
        std::fs::write(&frb_generated_path, &canonical).expect("write canonical frb_generated.rs");

        let drift = frb_generated_drift(&config, base_dir.path());
        assert!(
            drift.is_empty(),
            "already-canonical frb_generated.rs must not be reported as drift: {drift:?}"
        );
    }
}
