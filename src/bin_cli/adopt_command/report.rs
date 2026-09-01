//! What one `alef adopt` invocation prints, merged across its targets and bounded.
//!
//! Every section here used to be emitted once **per target**, so a 48-target recovery sweep
//! repeated the same create-once-seed paragraph 48 times, re-printed the same converged
//! tally 48 times, and printed one full diff body per drifted match with no ceiling at all.
//! The result was ~11,500 lines for a command whose entire job is to make a broken repo
//! recoverable — a recovery path nobody can read is not one.
//!
//! Two rules govern what is allowed to shrink:
//!
//! * **Every path that carries a decision is still named, once.** The create-once-seed list
//!   and the unreadable list are the command's *result* for those paths, not progress
//!   chatter; they are deduplicated across targets and printed in full, never counted.
//! * **Nothing is elided silently.** [`render`] only ever drops diff *bodies*, only when
//!   `--converged-only` makes those diffs informational (a drifted file cannot be adopted
//!   under that flag), and it names every elided path alongside an exact count of how many
//!   bodies it withheld. A short output and a truncated one must not render identically. ~keep

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::cli::commands::adopt::{AdoptBatchOutcome, AdoptReport};

/// How many full drifted diff bodies `--converged-only` prints before it switches to naming
/// the remainder by path.
///
/// Deliberately generous rather than minimal: under `--converged-only` these diffs are the
/// operator's shortlist of what to look at next, so the ceiling exists to stop the
/// pathological case (thousands of frozen snippets) from burying the run, not to ration
/// normal use. A sweep with fewer drifted matches than this prints exactly what it always
/// did. ~keep
pub(crate) const CONVERGED_ONLY_DIFF_BODY_LIMIT: usize = 20;

/// One piece of `alef adopt`'s stdout result.
///
/// Rendering returns these instead of writing as it goes so the report is assertable: a test
/// reads the exact lines, including the elision count, without capturing stdout — and a
/// truncation that failed to state its count fails the test rather than looking like a short
/// run. ~keep
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReportChunk {
    Line(String),
    Blank,
    /// Output whose pieces already carry their own line terminators (a unified diff).
    Fragment(String),
}

/// Every decision one invocation reached, merged across its targets.
///
/// Sets rather than vectors: two targets that both select `packages/ruby/foo.gemspec` reached
/// the same verdict about it, and printing that verdict twice tells the reader nothing except
/// how the globs happened to overlap. Ordering is path order, which is also the order each
/// individual target reported in. ~keep
#[derive(Debug, Default)]
pub(crate) struct AdoptSummary {
    pub(crate) unreadable: BTreeSet<PathBuf>,
    pub(crate) skipped_create_once: BTreeSet<PathBuf>,
    pub(crate) converged: BTreeSet<PathBuf>,
    pub(crate) already_owned: BTreeSet<PathBuf>,
    pub(crate) adopted: BTreeSet<PathBuf>,
    pub(crate) recorded_unstampable: BTreeSet<PathBuf>,
    pub(crate) skipped_drifted: BTreeSet<PathBuf>,
    pub(crate) diffs: BTreeMap<PathBuf, String>,
    pub(crate) preview: bool,
}

impl AdoptSummary {
    /// `preview` comes from the invocation's own `--write` flag rather than from the
    /// per-target reports: with every target failing there are no reports to read it off,
    /// and the summary would then claim a write happened. ~keep
    pub(crate) fn merge(outcome: &AdoptBatchOutcome, preview: bool) -> Self {
        let mut summary = Self {
            preview,
            ..Self::default()
        };
        for report in outcome.reports() {
            summary.absorb(report);
        }
        summary
    }

    fn absorb(&mut self, report: &AdoptReport) {
        self.unreadable.extend(report.unreadable.iter().cloned());
        self.skipped_create_once
            .extend(report.skipped_create_once.iter().cloned());
        self.converged.extend(report.converged.iter().cloned());
        self.already_owned.extend(report.already_owned.iter().cloned());
        self.adopted.extend(report.adopted.iter().cloned());
        self.recorded_unstampable
            .extend(report.recorded_unstampable.iter().cloned());
        self.skipped_drifted.extend(report.skipped_drifted.iter().cloned());
        for diff in &report.diffs {
            self.diffs.insert(diff.relative.clone(), diff.body.clone());
        }
    }

    /// Whether no target reached a verdict about any path.
    pub(crate) fn decided_nothing(&self) -> bool {
        self.unreadable.is_empty()
            && self.skipped_create_once.is_empty()
            && self.converged.is_empty()
            && self.already_owned.is_empty()
            && self.adopted.is_empty()
            && self.skipped_drifted.is_empty()
            && self.diffs.is_empty()
    }
}

/// How many diff bodies [`render`] prints and how many it withholds.
///
/// Split out as a value so the ceiling is testable directly and so the printed message can
/// never disagree with what was actually printed — both read the same numbers. ~keep
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DiffBudget {
    pub(crate) printed: usize,
    pub(crate) elided: usize,
}

impl DiffBudget {
    /// A drifted diff is only ever withheld when `--converged-only` has already made it
    /// impossible for this run to adopt the file it describes. Without that flag the diff is
    /// the consent document for a write this very command may perform, and the module header
    /// of `cli::commands::adopt` makes the full, untruncated diff a hard invariant of that
    /// case — so the budget is unbounded there. ~keep
    pub(crate) fn decide(total: usize, converged_only: bool) -> Self {
        if !converged_only {
            return Self {
                printed: total,
                elided: 0,
            };
        }
        let printed = total.min(CONVERGED_ONLY_DIFF_BODY_LIMIT);
        Self {
            printed,
            elided: total - printed,
        }
    }
}

pub(crate) fn render(summary: &AdoptSummary, converged_only: bool) -> Vec<ReportChunk> {
    if summary.decided_nothing() {
        // Every target failed, so there is no result to print. The "nothing was written,
        // re-run with --write" hint would be actively wrong here: `--write` was not the
        // problem, and the failures the caller is about to report are. ~keep
        return Vec::new();
    }
    let mut chunks = Vec::new();
    render_unreadable(summary, &mut chunks);
    render_create_once_seeds(summary, &mut chunks);
    render_diffs(summary, converged_only, &mut chunks);
    render_tallies(summary, &mut chunks);
    chunks
}

fn render_unreadable(summary: &AdoptSummary, chunks: &mut Vec<ReportChunk>) {
    if summary.unreadable.is_empty() {
        return;
    }
    // Named, not counted, and on stdout beside the diffs: this list is the whole
    // result for these paths, and it reports alef being unable to say anything --
    // not the operator having a decision left to make. A binary alef *does* emit
    // (a `.jar`) is no longer here: it is diffed by size and digest and adopted
    // through the ownership record like any other unstampable format. ~keep
    chunks.push(ReportChunk::Blank);
    chunks.push(ReportChunk::Line(
        "NOT ADOPTED -- alef could not read these matches: their bytes are neither valid \
         UTF-8 nor one of alef's own base64-encoded binary outputs, so alef leaves them alone:"
            .to_owned(),
    ));
    for path in &summary.unreadable {
        chunks.push(ReportChunk::Line(format!("  {}", path.display())));
    }
    chunks.push(ReportChunk::Blank);
}

fn render_create_once_seeds(summary: &AdoptSummary, chunks: &mut Vec<ReportChunk>) {
    if summary.skipped_create_once.is_empty() {
        return;
    }
    // Every path, never a count, and on stdout with the drifted diffs rather
    // than through `tracing`: this list is the command's result for these
    // paths, and `-q` must not be able to hide the one output that says work
    // is about to be destroyed. The consequence is spelled out because
    // "skipped" alone reads as "nothing happened", when the fact the operator
    // needs is what adopting them *would* have cost. ~keep
    chunks.push(ReportChunk::Blank);
    chunks.push(ReportChunk::Line(
        "NOT ADOPTED -- create-once seeds. alef writes each only when the path is absent, so \
         the copy on disk has almost certainly grown past alef's placeholder. Adopting one \
         consents to alef REPLACING its contents with that placeholder on the next OVERWRITING \
         regen -- an `alef version` sync or `alef all --clobber-create-once-seeds`, not a plain \
         `alef generate`:"
            .to_owned(),
    ));
    for path in &summary.skipped_create_once {
        chunks.push(ReportChunk::Line(format!("  {}", path.display())));
    }
    chunks.push(ReportChunk::Line(
        "Fix: confirm each holds nothing you wrote, then re-run with \
         --clobber-create-once-seeds to adopt them anyway."
            .to_owned(),
    ));
    chunks.push(ReportChunk::Blank);
}

fn render_diffs(summary: &AdoptSummary, converged_only: bool, chunks: &mut Vec<ReportChunk>) {
    let total = summary.diffs.len();
    let budget = DiffBudget::decide(total, converged_only);
    for (_, body) in summary.diffs.iter().take(budget.printed) {
        chunks.push(ReportChunk::Fragment(body.clone()));
        chunks.push(ReportChunk::Blank);
    }
    if budget.elided == 0 {
        return;
    }
    tracing::warn!(
        printed = budget.printed,
        withheld = budget.elided,
        total,
        "printed a bounded number of drifted diff bodies; every withheld path is named on stdout"
    );
    chunks.push(ReportChunk::Line(format!(
        "{} of {total} drifted diff(s) are named below WITHOUT their bodies -- only the first {} \
         were printed in full above. --converged-only cannot adopt a drifted file. Run \
         `alef adopt <path>` on one to read its complete diff:",
        budget.elided, budget.printed,
    )));
    for path in summary.diffs.keys().skip(budget.printed) {
        chunks.push(ReportChunk::Line(format!("  {}", path.display())));
    }
    chunks.push(ReportChunk::Blank);
}

fn render_tallies(summary: &AdoptSummary, chunks: &mut Vec<ReportChunk>) {
    if !summary.converged.is_empty() {
        // Summarised, never diffed. See `cli::commands::adopt`'s header: a
        // converged file's diff is the file itself echoed back, and printing
        // 12,000 of them buries the drifted diffs printed just above -- the
        // only ones with content to read. ~keep
        chunks.push(ReportChunk::Line(format!(
            "{} file(s) already match generated output byte-for-byte apart from the marker; \
             adopting them changes no content.",
            summary.converged.len()
        )));
    }
    if summary.preview {
        if !summary.diffs.is_empty() && !summary.converged.is_empty() {
            chunks.push(ReportChunk::Line(format!(
                "Re-run with --converged-only --write to adopt the {} converged file(s) alone, \
                 then review the {} drifted diff(s) above before adopting those.",
                summary.converged.len(),
                summary.diffs.len()
            )));
        }
        chunks.push(ReportChunk::Line(
            "Nothing was written. Re-run with --write to stamp these files so alef can regenerate them.".to_owned(),
        ));
        return;
    }
    if !summary.skipped_drifted.is_empty() {
        chunks.push(ReportChunk::Line(format!(
            "Left {} drifted file(s) untouched (--converged-only). Read each diff above, then \
             adopt it with an explicit target.",
            summary.skipped_drifted.len()
        )));
    }
}

/// Write a rendered report to the sanctioned stdout result surface.
pub(crate) fn emit(chunks: &[ReportChunk]) {
    for chunk in chunks {
        match chunk {
            ReportChunk::Line(text) => crate::bin_cli::output::line(text),
            ReportChunk::Blank => crate::bin_cli::output::blank(),
            ReportChunk::Fragment(text) => crate::bin_cli::output::fragment(text),
        }
    }
}

/// The diagnostics that belong on the `tracing` channel rather than in the result.
///
/// `already_owned` is per-path at DEBUG and a single count at INFO. It was one INFO line per
/// path, which on the bulk-migration case this command exists for is five figures of INFO
/// naming files where *nothing happened* — the exact inverse of the level contract, and the
/// same reasoning that already puts converged adoptions at DEBUG in `adopt::batch`. ~keep
pub(crate) fn log_diagnostics(summary: &AdoptSummary) {
    if summary.decided_nothing() {
        return;
    }
    for path in &summary.already_owned {
        tracing::debug!(path = %path.display(), "already alef-owned, nothing to adopt");
    }
    if !summary.already_owned.is_empty() {
        tracing::info!(
            "{} file(s) already alef-owned, nothing to adopt",
            summary.already_owned.len()
        );
    }
    if summary.preview {
        return;
    }
    tracing::info!("Adopted {} file(s)", summary.adopted.len());
    if !summary.recorded_unstampable.is_empty() {
        // These adoptions changed no bytes in the files themselves — the whole
        // consent lives in `.alef-ownership.toml`. Leave it uncommitted and the
        // human read the diff for nothing: every other checkout, CI included,
        // still refuses these paths. Naming the file is the difference between
        // an adoption and an adoption that took effect. ~keep
        tracing::info!(
            "{} of these carry no marker syntax; their ownership is recorded in \
             .alef-ownership.toml. Commit that file or the adoption applies only to \
             this working copy.",
            summary.recorded_unstampable.len()
        );
    }
}
