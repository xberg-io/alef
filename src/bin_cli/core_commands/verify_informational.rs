//! The `alef verify` report sections that are printed for the operator rather than for the
//! verdict.
//!
//! ~keep Own module rather than blocks inline in `verify.rs`: that file sits at the repo's
//! 1,000-line cap (`file-modularization` in CLAUDE.md), mirroring `verify_flags.rs`'s and
//! `verify_dart_bridge.rs`'s identical reason for existing as their own files.
//!
//! ~keep [`report_untracked_records`] is the one partial exception to "informational": its
//! printing is unconditional and gate-free like everything else here, but the findings it
//! returns still gate `alef verify`'s exit code back in [`super::verify`] -- see
//! `super::ensure_required_records_tracked`.

use crate::bin_cli::helpers::FrozenFile;
use crate::cli::cache;

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
pub(super) fn log_managed_surface(managed_paths: &std::collections::HashSet<std::path::PathBuf>) {
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

/// Report create-once scaffold files whose template has since been fixed.
pub(super) fn report_create_once_template_drift(create_once_template_drift: &[String]) {
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
        for path in create_once_template_drift {
            crate::bin_cli::output::line(format_args!("  {path}"));
        }
    }
}

/// Report, and return, the required alef records git does not track.
pub(super) fn report_untracked_records(base_dir: &std::path::Path) -> Vec<&'static str> {
    // The `verify` half of the escalation `cache::untracked_required_records`
    // documents: write commands warn and keep going, verification must refuse. The
    // query is already silent outside a git work tree and for a record that does not
    // exist yet, so this never fires where "untracked" is unanswerable, nor on the
    // run that legitimately creates the record. ~keep
    let untracked_records = cache::untracked_required_records(base_dir);
    if !untracked_records.is_empty() {
        crate::bin_cli::output::line(
            "Required alef records are not tracked by git (alef writes these and depends on them \
         being committed):",
        );
        for record in &untracked_records {
            crate::bin_cli::output::line(format_args!("  {record} -- fix with: git add {record}"));
        }
    }
    untracked_records
}

/// Report the drifted create-once seeds, returning `(unmarked seeds, drifted seeds)`.
pub(super) fn report_drifted_seeds(frozen_generated_files: &[FrozenFile]) -> (Vec<&str>, Vec<&str>) {
    // Printed unconditionally, before the verdict and on every run including a clean one.
    // Every finding above is a NEGATIVE claim, and a report made only of negative claims is
    // indistinguishable from one that examined nothing -- which is exactly how consumer CI
    // came to read a green `alef verify` as a whole-tree freshness gate when it is a claim
    // about marker-carrying files only. See `verify_coverage`'s module doc. ~keep
    let unmarked_seeds = crate::bin_cli::helpers::frozen::unmarked_create_once_seeds(frozen_generated_files);
    let drifted_seeds = crate::bin_cli::helpers::frozen::drifted_frozen_seeds(frozen_generated_files);
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
    for line in crate::bin_cli::helpers::frozen::drifted_seed_report_lines(frozen_generated_files) {
        crate::bin_cli::output::line(line);
    }
    (unmarked_seeds, drifted_seeds)
}

/// The independently-measured inputs [`report_coverage`] states, each produced at a different
/// point in [`super::verify`]'s run.
pub(super) struct CoverageParams<'a> {
    pub(super) all_managed_paths: &'a std::collections::HashSet<std::path::PathBuf>,
    pub(super) marked_paths: &'a std::collections::HashSet<std::path::PathBuf>,
    pub(super) scan_coverage: super::super::verify_scan::ScanCoverage,
    pub(super) unmarked_seeds: &'a [&'a str],
    pub(super) drifted_seed_count: usize,
    pub(super) ephemeral_excluded_count: usize,
    pub(super) declared_user_owned_count: usize,
}

/// Print the coverage block that states how much of the tree this run's verdict is about.
pub(super) fn report_coverage(params: CoverageParams<'_>) {
    let CoverageParams {
        all_managed_paths,
        marked_paths,
        scan_coverage,
        unmarked_seeds,
        drifted_seed_count,
        ephemeral_excluded_count,
        declared_user_owned_count,
    } = params;
    // Paths at debug level, count in the report: one line per seed is 72 lines on a measured
    // consumer tree, and there is no action any of them prompts -- but the count must never be
    // invisible, which is what reporting them only inside the failure block amounted to. ~keep
    for path in unmarked_seeds {
        tracing::debug!("unmarked create-once seed (contents not verified): {path}");
    }
    for line in super::super::verify_coverage::VerifyCoverage::measure(
        all_managed_paths,
        marked_paths,
        scan_coverage,
        super::super::verify_coverage::VerifyCoverageCounts {
            create_once_unmarked: unmarked_seeds.len(),
            create_once_drifted: drifted_seed_count,
            ephemeral_excluded: ephemeral_excluded_count,
            declared_user_owned: declared_user_owned_count,
        },
    )
    .report_lines()
    {
        crate::bin_cli::output::line(line);
    }
}
