//! Frozen managed files -- pre-existing alef-owned paths that carry no provenance marker,
//! and so are deadlocked out of the write guard forever (see [`FrozenFile`]'s doc).
//!
//! Split out of `helpers.rs` rather than added to it: that file sits at this repository's
//! 1,000-line cap, and this concern -- deciding which pre-existing files are frozen, and
//! whether each one is a create-once seed or a genuinely adoptable frozen file -- is
//! self-contained enough to own its own module. ~keep

#[cfg(test)]
mod tests;

// The write guards' own convergence predicate, reached through the pipeline rather than
// restated here. This report and the guards must answer "did the refused write have different
// content to deliver" identically, or the report describes a refusal that did not happen --
// the two-derivations-of-one-fact shape behind this repository's last several ownership
// defects. It lives beside the guards, in `cli::pipeline::generate::write::report`. ~keep
use crate::cli::pipeline::matches_alef_output;

/// A generated file alef would own and mark, that already exists on disk but
/// carries no provenance marker at all.
///
/// This is a different, unrecoverable condition from a stale [`super::StaleMismatch`]
/// or a [`super::missing_managed_paths`] entry: the write guard in
/// `crate::cli::pipeline::generate::write::write_files_report` and
/// `crate::cli::pipeline::generate::scaffold::write_scaffold_files_report`
/// refuses to touch a pre-existing file that carries no marker (it cannot tell
/// a hand-written file from an alef output that predates the marker system),
/// and the marker can only ever be added *by* a write the guard has already
/// authorised — so an unmarked pre-existing file is frozen forever. Running
/// `alef generate` again does nothing; a human must read the file, then either
/// adopt it (paste `remedy` in and rerun `alef generate`) or delete it so
/// generation can write it cleanly. ~keep
pub(crate) struct FrozenFile {
    pub(crate) path: String,
    /// The literal marker line `alef adopt` would add to the top of the file, or `None` when
    /// the format has no comment syntax to carry one (`.json`, lockfiles).
    ///
    /// Informational only -- [`report_lines`] never instructs a reader to paste this in by
    /// hand. `crate::cli::pipeline::generate::write::report_refused_writes` (the write guard's
    /// own refusal message) is explicit that hand-adding the marker is unsafe: "a refusal can
    /// be protecting a deliberate hand-edit, and stamping it blind re-enables exactly the
    /// clobbering the guard exists to prevent." `alef verify`'s report used to say the opposite
    /// -- "add the marker shown" -- which pointed a reader at the one workflow alef's own write
    /// guard warns against, instead of at `alef adopt`, the reviewed, diffed path that exists
    /// for exactly this. ~keep
    pub(crate) remedy: Option<String>,
    /// A leading line in the existing file that looks like a failed attempt at a marker --
    /// see [`crate::core::hash::near_miss_marker`] -- so the report can point at what's already
    /// there instead of only showing what should be there. `None` when the file's leading lines
    /// don't mention alef and generation at all (a plain hand-written file). ~keep
    pub(crate) near_miss: Option<String>,
    /// Whether this path is a create-once seed under
    /// [`crate::cli::commands::adopt::is_create_once_seed`] -- the exact predicate `alef
    /// adopt` gates `--clobber-create-once-seeds` on, called here rather than
    /// re-derived, so the report and the command it points at can never disagree about
    /// which paths are seeds.
    ///
    /// THE DEFECT this closes: before this field existed, every frozen path was reported
    /// under one heading with one remedy ("run `alef adopt <path>`"), regardless of
    /// whether the path was a create-once seed. `alef adopt --write` then refused every
    /// seed outright -- measured at 85 of 85 in one consumer repo and 99 of 99 (all of
    /// them seeds) in another -- naming a flag (`--clobber-create-once-seeds`) the
    /// report never mentioned. A human followed the printed remedy and hit a wall every
    /// single time.
    ///
    /// Splitting the heading was not enough, and [`report_lines`] finished the job: a
    /// create-once seed is no longer reported as frozen at all, because the only remedy alef
    /// has for one is the flag its own output calls DANGEROUS, for a file alef never rewrites.
    /// The field survives because the count still has to be stated -- as coverage, in
    /// `bin_cli::verify_coverage`. ~keep
    pub(crate) create_once: bool,
    /// Whether the bytes on disk differ from the bytes alef would write to this path, once
    /// the provenance header alef itself would prepend is discounted
    /// ([`matches_alef_output`]).
    ///
    /// THE DEFECT this closes: every report about a frozen file described the file's
    /// OWNERSHIP ("carries no marker") and nothing about its CONTENT, so a frozen file that
    /// is byte-for-byte what alef would generate and a frozen file whose withheld content
    /// has since gone stale read identically -- as a count of files not written. Measured:
    /// a generated PHP test-app installer bakes the release version into its own bytes, the
    /// guard refused the rewrite on every run, and three separate consumer repositories
    /// shipped an installer pinned to a stale release for weeks. From outside, a refusal on
    /// version-derived content is indistinguishable from a file that is simply up to date --
    /// unless the report says which one it is. alef has both sides in hand at the moment it
    /// decides (the disk bytes it just read, and the rendered `GeneratedFile` it was about
    /// to write), so the comparison costs one string compare and no extra I/O. ~keep
    pub(crate) drifted: bool,
    /// Whether alef re-attempts this write on every run, rather than emitting the path only
    /// when it is absent.
    ///
    /// This is the antecedent of [`FrozenFile`]'s own definition -- "alef would write this
    /// path and the guard refuses it forever" -- and it is not uniform across the tree.
    /// `write_scaffold_files_report`'s `can_skip` short-circuits a `generated_header: false`
    /// path that already exists BEFORE any ownership or content check, so under an
    /// `overwrite = false` writer (`packages/**`) nothing is attempted and nothing is
    /// withheld: a seed a human has grown past alef's placeholder is the documented steady
    /// state, and reporting its drift would be noise on the exact list whose value is that
    /// every line on it is real. The e2e and test-app stages write with `overwrite = true`
    /// (`bin_cli::all_commands::e2e_stage`), so `can_skip` never fires there and the guard
    /// refuses a real, differing write on every single run. Only that second case is a
    /// defect a consumer cannot see. ~keep
    pub(crate) rewritten_every_run: bool,
}

/// [`FrozenFile`] entries for every alef-owned file in `files` that already
/// exists on disk but carries no marker.
///
/// Uses the same ownership predicate as [`super::missing_managed_paths`] — a
/// scaffold-once file alef never marks is excluded here exactly as it is from
/// the missing-file check, so a hand-edited `Cargo.toml`/`package.json`
/// template is never mistaken for a frozen generated file.
///
/// For a format [`crate::cli::pipeline::marker_comment_style`] has no comment syntax for
/// (`.json`, `DESCRIPTION`, a pre-widening `.clang-format`), a missing marker is not by
/// itself evidence of foreign authorship — [`crate::cli::pipeline::is_owned_by_ownership_record`]
/// is consulted exactly as `write_files_report`'s and `write_scaffold_files_report`'s write
/// guards consult it, so this report agrees with what those guards would actually accept.
/// Before this fell back to the marker check alone, a file the write guard would happily
/// (re)write on the strength of its committed `.alef-ownership.toml` record — including one
/// `alef adopt` or a delete-and-regenerate had just recorded — stayed reported "frozen"
/// forever, because this function never looked at the record the guard relies on. ~keep
///
/// The remedy text is read straight from the in-memory `GeneratedFile::content`
/// first, because a self-marking backend (custom Swift/Kotlin/Dart/Gleam/Zig
/// headers, `docs::render`'s HTML-commented `.md` pages) already bakes its
/// literal header into `content` regardless of `generated_header`. Only when
/// that content carries no marker yet — the common case, where the header is
/// added later by `write_files_report`'s `ensure_generated_header` pass — does
/// this fall back to reconstructing it from the path via
/// [`crate::cli::pipeline::provenance_header_for_path`]. ~keep
///
/// Runs over two candidate sets, not only [`crate::cli::pipeline::managed_generated_files`]'s
/// marker-carrying subset: `carries_alef_marker()` is `generated_header ||
/// content_has_alef_marker`, so a file emitted with `generated_header: false` whose content
/// embeds no marker at all — the PHP backend's `config.m4`
/// (`backends::php::gen_bindings::rust_items::generate_config_m4`) is the shipped case —
/// never reaches the ownership-record fallback a few lines below, even though
/// `write_files_report`'s guard already refuses to overwrite that exact path once it exists
/// without a committed `.alef-ownership.toml` record. [`unmarkable_unclaimed_files`] recovers
/// that second set: it is deliberately narrower than "every `generated_header: false` file" —
/// see its own doc for why only the genuinely unmarkable ones qualify.
///
/// `create_once` is computed from the *original* [`crate::core::backend::GeneratedFile`]
/// via [`crate::cli::commands::adopt::is_create_once_seed`] before it is consumed to build
/// `remedy`/`near_miss` below — the same predicate answers correctly for both candidate
/// sets without branching: every entry from `managed_generated_files` carries a marker
/// (`carries_alef_marker() == true`), which `is_create_once_seed` always answers `false`
/// for, so only entries recovered from the two unmarked candidate sets below can ever be
/// seeds. ~keep
///
/// `rewritten_roots` are the absolute output roots whose writer runs with `overwrite = true`
/// -- see [`FrozenFile::rewritten_every_run`] for why that distinction decides whether a
/// missing marker is a withheld write or a documented steady state, and
/// [`super::rewritten_output_roots`] for where the roots come from. Passing an empty slice
/// keeps the pre-existing candidate sets exactly as they were.
pub(super) fn frozen_managed_paths(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
    rewritten_roots: &[std::path::PathBuf],
) -> Vec<FrozenFile> {
    // `[workspace.ownership] user_owned` is consulted here, not just in the writers, because
    // "frozen" means "alef would write this path and the guard refuses it forever" -- and for a
    // declared path the antecedent is false by consumer declaration: no write is attempted, so
    // no write is refused, and the absent marker is the documented steady state rather than a
    // deadlock. Reporting one here would put it back under a heading whose only remedy (`alef
    // adopt --write`) grants exactly the ownership the declaration disclaims. Deriving the
    // declaration from `base_dir` is the same call the write guards make, so this report and
    // those guards cannot disagree about which paths are declared. ~keep
    //
    // A config that declares ownership and fails to parse is fatal in `declared_user_owned`,
    // but `alef verify` has already loaded and resolved the same file before reaching here, so
    // that state is unreachable on this path; falling back to "declares nothing" is the same
    // answer verify would give if the option were absent. ~keep
    let declared = crate::cli::pipeline::declared_user_owned(base_dir)
        .unwrap_or_else(|_| crate::core::config::UserOwnedPaths::none());
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .chain(unmarkable_unclaimed_files(files, base_dir))
        .chain(unmarked_files_under_rewritten_roots(files, base_dir, rewritten_roots))
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            if declared.matches(base_dir, &full_path) {
                return None;
            }
            let existing = std::fs::read_to_string(&full_path).ok()?;
            if crate::core::hash::content_has_alef_marker(&existing) {
                return None;
            }
            let is_markable = crate::cli::pipeline::marker_comment_style(&full_path).is_some();
            if !is_markable && crate::cli::pipeline::is_owned_by_ownership_record(base_dir, &full_path) {
                return None;
            }
            let create_once = crate::cli::commands::adopt::is_create_once_seed(&file);
            // The bytes the writer would actually place, obtained from the writer's own
            // `normalize_content` + `ensure_generated_header` pair via `adopt::managed_outputs`
            // rather than re-derived here -- a comparison against content that is merely close
            // to what alef would write reports drift that no regeneration would ever resolve.
            //
            // COST, stated because it is a real one: `normalize_content` shells out to rustfmt
            // for a `.rs` path, so this spawns one process per frozen Rust file. Bounded by the
            // frozen candidate set, which is empty on a healthy tree and is dominated in the
            // measured bad cases by `.md` and `.toml` (no subprocess). Approximating the
            // comparison to dodge that would answer a different question than the guard asked,
            // and the whole point here is that the two agree. ~keep
            let drifted = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&file), base_dir)
                .first()
                .is_none_or(|output| !matches_alef_output(&full_path, &existing, &output.content));
            let remedy = super::marker_line(&file.content).map(str::to_owned).or_else(|| {
                let header = crate::cli::pipeline::provenance_header_for_path(&file.path)?;
                super::marker_line(&header).map(str::to_owned)
            });
            let near_miss = crate::core::hash::near_miss_marker(&existing).map(str::to_owned);
            Some(FrozenFile {
                path: full_path.display().to_string(),
                remedy,
                near_miss,
                create_once,
                drifted,
                rewritten_every_run: is_under_any_root(&full_path, rewritten_roots),
            })
        })
        .collect()
}

/// Every file in `files` that both existing candidate sets miss: no marker of any kind
/// (`carries_alef_marker()` is false, nothing in `content`), a MARKABLE extension, and a path
/// under one of `rewritten_roots`.
///
/// [`unmarkable_unclaimed_files`] is scoped to the unmarkable subset for a reason its own doc
/// gives -- an ownership record may only ever clear a path the write guard would accept on a
/// record, and a markable path with no marker is refused regardless of any record. That
/// reasoning bounds where the RECORD may be consulted; it does not bound which files can be
/// frozen, and this set falls on the correct side of it either way, because
/// [`frozen_managed_paths`]'s own record fallback is already gated on `!is_markable`.
///
/// THE HOLE this closes: a `generated_header: false` file on a markable extension carries no
/// marker in memory and is therefore in neither existing set, so `alef verify` could not see
/// it at all -- not as a finding, not as a create-once seed, not in any count. The measured
/// instance is a generated PHP test-app installer (`.sh`, `generated_header: false`) whose
/// baked-in release version had drifted in three consumer repositories while `alef verify`
/// stayed green. The e2e writer passes `overwrite = true`, so alef was attempting and being
/// refused that exact write on every run, and the only place it was ever mentioned was the
/// write-time refusal tally.
///
/// Scoped to `rewritten_roots` rather than applied tree-wide: under an `overwrite = false`
/// writer the identical file shape (`packages/dart/test/*_test.dart`, `build.zig`) is skipped
/// by `can_skip` before any check runs, so nothing is withheld and there is nothing to report.
/// See [`FrozenFile::rewritten_every_run`]. ~keep
fn unmarked_files_under_rewritten_roots(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
    rewritten_roots: &[std::path::PathBuf],
) -> Vec<crate::core::backend::GeneratedFile> {
    if rewritten_roots.is_empty() {
        return Vec::new();
    }
    files
        .iter()
        .filter(|file| {
            let full_path = base_dir.join(&file.path);
            !file.carries_alef_marker()
                && crate::cli::pipeline::marker_comment_style(&full_path).is_some()
                && is_under_any_root(&full_path, rewritten_roots)
        })
        .cloned()
        .collect()
}

fn is_under_any_root(path: &std::path::Path, roots: &[std::path::PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Every file in `files` that [`crate::cli::pipeline::managed_generated_files`] excludes
/// (`carries_alef_marker()` is false — no `generated_header: true` claim, no marker baked
/// into `content`) but that is genuinely incapable of ever carrying one
/// ([`crate::cli::pipeline::marker_comment_style`] answers `None` for its path).
///
/// Scoped this narrowly on purpose: widening it to every `generated_header: false` file
/// would also pull in a markable file a backend simply forgot to self-mark, which
/// `write_files_report`'s guard treats differently — a markable path with no marker is
/// refused regardless of any ownership record (see that function's `owned` computation),
/// so folding it into this ownership-record-checked set would wrongly clear it once a
/// record existed. Only the genuinely unmarkable subset is where alef's write guard has
/// ever accepted an ownership record as proof, and this mirrors exactly that. ~keep
fn unmarkable_unclaimed_files(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> Vec<crate::core::backend::GeneratedFile> {
    files
        .iter()
        .filter(|file| {
            !file.carries_alef_marker()
                && crate::cli::pipeline::marker_comment_style(&base_dir.join(&file.path)).is_none()
        })
        .cloned()
        .collect()
}

/// Whether any frozen file is one `alef adopt --write` will actually ACCEPT.
///
/// A create-once seed is excluded on purpose. Its missing marker is deliberate, not drift: the
/// write guard refuses it by design, a plain `alef generate` leaves it untouched, and adopting it
/// requires the explicit `--clobber-create-once-seeds`. Gating `alef verify`'s exit code on the
/// whole frozen list therefore made verify unable to reach exit 0 on any repo carrying legacy
/// pre-marker files -- no amount of regeneration cleared them -- so the release gate could only be
/// satisfied by reaching for a destructive flag. `create_once` comes from
/// [`crate::cli::commands::adopt::is_create_once_seed`], the identical predicate `alef adopt` gates
/// that flag on, so this and that refusal cannot drift apart. ~keep
pub(crate) fn has_adoptable_frozen_files(frozen: &[FrozenFile]) -> bool {
    frozen.iter().any(|file| !file.create_once)
}

/// The paths of every create-once seed on disk that carries no provenance marker.
///
/// Named as a *coverage* fact, not a finding: `alef verify` proves nothing about these files'
/// contents, and there is no action that changes that. See [`report_lines`] for why they are
/// no longer reported as frozen. ~keep
pub(crate) fn unmarked_create_once_seeds(frozen: &[FrozenFile]) -> Vec<&str> {
    frozen
        .iter()
        .filter(|file| file.create_once)
        .map(|file| file.path.as_str())
        .collect()
}

/// Every create-once seed that alef re-attempts on every run AND whose on-disk content
/// differs from what it would write -- the withheld-and-stale set.
///
/// The one subset of the frozen report where a count alone is a false statement. For every
/// other seed the coverage line's wording is true as written: nothing is attempted, so the
/// missing marker is a steady state and the file's contents are simply unproven. For these,
/// alef renders different bytes, tries to write them, is refused, and says only that N files
/// were not written -- which is exactly what let a version-bearing installer sit stale in three
/// consumer repositories for weeks. Naming the file is the whole fix: the reader cannot infer
/// it from any number, because a refusal on up-to-date content and a refusal on stale content
/// produce the same number. ~keep
pub(crate) fn drifted_frozen_seeds(frozen: &[FrozenFile]) -> Vec<&str> {
    frozen
        .iter()
        .filter(|file| file.create_once && file.drifted && file.rewritten_every_run)
        .map(|file| file.path.as_str())
        .collect()
}

/// The report for [`drifted_frozen_seeds`], or no lines at all when there are none.
///
/// Separate from [`report_lines`] because the remedy is different, not because the wording is:
/// these paths are refused by `alef adopt --write` as create-once seeds, so pointing at the
/// same remedy would repeat the measured failure where a human followed the printed advice and
/// hit a wall every time. The two remedies that do work are named instead. ~keep
pub(crate) fn drifted_seed_report_lines(frozen: &[FrozenFile]) -> Vec<String> {
    let drifted = drifted_frozen_seeds(frozen);
    if drifted.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        format!(
            "{} frozen path(s) DRIFTED: alef re-renders these every run and would write different \
             bytes, but the file carries no provenance marker.",
            drifted.len()
        ),
        "  Preview: alef adopt <path>".to_owned(),
        "  Fix: delete the file, or declare it in `[workspace.ownership] user_owned`".to_owned(),
    ];
    for path in drifted {
        lines.push(format!("    {path}"));
    }
    lines
}

/// `alef verify`'s closing sign-off line, derived FROM [`drifted_frozen_seeds`] rather than
/// computed independently of it.
///
/// The sign-off used to be a bare literal at the call site, printed whenever every
/// *other* finding was empty -- with no regard for whether [`drifted_seed_report_lines`] had
/// just printed the "frozen path(s) DRIFTED" block a few lines above.
/// A run against 21 version-bearing manifests (`package.json`, `go.mod`, `pom.xml`, ...) named
/// every one of them as drifted and then, three lines later, asserted the unqualified opposite
/// and exited 0 -- the two halves of the report disagreeing about the same run. Passing the
/// finding count in, instead of re-testing a matching condition at the call site, is what keeps
/// them from drifting apart again: there is only one place that decides whether the report was
/// clean, and it is the place that already counted the findings.
///
/// This does not change what `--exit-code` gates -- see [`drifted_frozen_seeds`]'s doc for why
/// a drifted create-once seed stays non-fatal there. It only stops the sign-off from lying about
/// a finding the same run just printed. ~keep
pub(crate) fn report_sign_off_line(drifted_seed_count: usize) -> String {
    if drifted_seed_count == 0 {
        return "All bindings and versions are up to date.".to_owned();
    }
    format!(
        "{drifted_seed_count} frozen path(s) drifted and not enforced (see the DRIFTED block \
         above); everything else is up to date."
    )
}

/// `alef verify`'s frozen-file report -- the ADOPTABLE entries only, one line each plus its
/// remedy.
///
/// A create-once seed is not reported here at all, and that is the fix rather than an
/// omission. [`FrozenFile`] means "alef would write this path and the guard refuses it
/// forever", and for a create-once seed the antecedent is false: alef emits the path only when
/// it is absent, so on an existing file there is no write to refuse and nothing is lost by the
/// missing marker. Reporting it as frozen described the file as a problem and then offered the
/// only escape alef has -- `alef adopt --write --clobber-create-once-seeds`, whose own output
/// calls it DANGEROUS -- for a file this repository's own documentation calls user-owned after
/// scaffold (`generated_header: false`). Measured in a consumer repo: `alef adopt
/// --converged-only` adopted 0 of 102 reported paths, 72 of them refused by alef itself as
/// seeds, including 13 LICENSE files and several `.gitkeep`s. A file cannot be both user-owned
/// and a verify finding.
///
/// The alternative -- recording ownership of a seed without touching its body -- was
/// considered and rejected: it buys no verification (alef still never rewrites the body, and
/// the stamp covers generation inputs rather than the seed's hand-grown contents) while handing
/// the write guard a licence it deliberately withholds, which is the exact protection
/// `--clobber-create-once-seeds` exists to gate.
///
/// The count does not disappear with the heading. `alef verify` states it in its coverage
/// report on every run, including a clean one, so these files move from "reported as a problem
/// only when something else already failed" to "always visible as unchecked" -- see
/// [`unmarked_create_once_seeds`] and `bin_cli::verify_coverage`. ~keep
pub(crate) fn report_lines(frozen: &[FrozenFile]) -> Vec<String> {
    let adoptable: Vec<&FrozenFile> = frozen.iter().filter(|file| !file.create_once).collect();
    if adoptable.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        format!(
            "Frozen generated files detected ({} path(s)): alef owns these paths but the files \
             carry no provenance marker, so writes are refused.",
            adoptable.len()
        ),
        "  Fix: run `alef adopt <path> --write`, or delete the file so generation rewrites it.".to_owned(),
        "  Do not hand-add the marker line -- the write guard reads it as consent to clobber.".to_owned(),
    ];
    for file in adoptable {
        lines.push(format!("  {}", file.path));
        // Stated per file, above the remedy, because it decides how the remedy must be read.
        // Adopting a converged file changes no byte of it; adopting a drifted one consents to
        // the next `alef generate` replacing content that is genuinely different -- and a
        // reader who cannot tell the two apart has to open every file to find out which of
        // them was the one silently holding stale output. ~keep
        lines.push(if file.drifted {
            "    content DIFFERS from what alef would generate: stale while it stays frozen".to_owned()
        } else {
            "    content already matches what alef would generate: adoption changes no bytes".to_owned()
        });
        if let Some(near_miss) = &file.near_miss {
            lines.push(format!(
                "    close but not recognized: {near_miss:?} (alef accepts \"generated by alef\" \
                 case-insensitively)"
            ));
        }
        lines.push(match &file.remedy {
            Some(remedy) => format!("    marker `alef adopt` would write: {remedy}"),
            None => "    no comment syntax for a marker: ownership lives in .alef-ownership.toml \
                     -- run `alef adopt <path> --write` to record it, or delete the file"
                .to_owned(),
        });
    }
    lines
}
