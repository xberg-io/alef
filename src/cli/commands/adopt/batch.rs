//! Resolving every target of one `alef adopt` invocation against one managed surface.
//!
//! The surface is already built once per invocation (`bin_cli::adopt_command` hoists the
//! extract/render sweep above the target loop), but everything downstream of it used to be
//! repeated per target: each target recompiled its glob once **per candidate path** while
//! scanning the whole managed set, re-read and re-classified every file it selected,
//! re-rendered every drifted diff, and closed with its own read-modify-write of
//! `.alef-ownership.toml`. On a 48-target sweep of a consumer tree whose surface holds five
//! figures of paths that is a million glob compilations and 48 full manifest parses to
//! answer questions the first pass already had the data for.
//!
//! [`AdoptSession`] owns the four things worth sharing across an invocation: one compiled
//! [`TargetMatcher`] per target, a classification cache keyed on the repo-relative path, a
//! rendered-diff cache, and a single accumulated ownership-record write.
//!
//! The per-target **decision** is deliberately unchanged and single-sourced. `alef adopt`
//! with one target runs the identical [`AdoptSession::adopt_target`] body through
//! [`run_single`], so there is no second copy of the partition, the two `bail!`s, or the
//! `--converged-only` filter that could drift from the batch path. ~keep

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{
    AdoptCandidate, AdoptDiff, AdoptOptions, AdoptReport, AdoptionState, ManagedOutput, apply, classify,
    classify_binary, render_diff,
};

/// One adopt target with its glob compiled once for the whole invocation.
///
/// Semantics are exactly [`super::matches_target`]'s — that function is now a thin wrapper
/// over this type, so the "could this stage's output ever satisfy one of these targets"
/// predicate `bin_cli::helpers::collect_managed_surface` shares with adopt cannot fall out
/// of step with the predicate adopt's own selection uses. ~keep
pub(crate) struct TargetMatcher {
    literal: String,
    pattern: Option<glob::Pattern>,
}

impl TargetMatcher {
    pub(crate) fn new(target: &str) -> Self {
        let literal = target.trim_start_matches("./").to_owned();
        let pattern = glob::Pattern::new(&literal).ok();
        Self { literal, pattern }
    }

    pub(crate) fn matches(&self, relative: &Path) -> bool {
        let spelled = relative.to_string_lossy().replace('\\', "/");
        if spelled == self.literal {
            return true;
        }
        self.pattern.as_ref().is_some_and(|pattern| pattern.matches(&spelled))
    }
}

/// What one managed output turned out to be, memoized for the whole invocation.
#[derive(Clone)]
enum Classification {
    /// Not on disk. Nothing to consent to: the guard never engages on a path with no
    /// pre-existing content, so an ordinary generate already writes this. ~keep
    Absent,
    /// Bytes that are neither valid UTF-8 on a text path nor decodable on a binary one.
    Unreadable,
    Ready(Rc<AdoptCandidate>),
}

/// The classifiable matches a target selected, plus the matches alef could neither read as
/// text nor decode as one of its own binary outputs.
struct SelectedCandidates {
    candidates: Vec<Rc<AdoptCandidate>>,
    unreadable: Vec<PathBuf>,
}

/// Everything one `alef adopt` invocation shares across its targets.
pub struct AdoptBatchOptions {
    pub base_dir: PathBuf,
    pub write: bool,
    pub converged_only: bool,
    pub clobber_create_once_seeds: bool,
}

impl AdoptBatchOptions {
    fn for_target(&self, target: &str) -> AdoptOptions {
        AdoptOptions {
            target: target.to_owned(),
            base_dir: self.base_dir.clone(),
            write: self.write,
            converged_only: self.converged_only,
            clobber_create_once_seeds: self.clobber_create_once_seeds,
        }
    }
}

/// What one invocation of `alef adopt` decided, target by target, in the order the operator
/// spelled them.
///
/// Kept per-target rather than merged because a target that resolves to nothing adoptable is
/// a failure of *that* target only and the exit status has to name which ones failed —
/// merging first would lose the association. The caller merges for display; see
/// `bin_cli::adopt_command`. ~keep
pub struct AdoptBatchOutcome {
    pub results: Vec<(String, Result<AdoptReport>)>,
    /// How many managed outputs this invocation actually read and classified.
    ///
    /// Exposed so the regression test can *count* the passes rather than infer them from
    /// timing: a re-entrant implementation shows this climbing with the target count, while
    /// one classification per distinct selected path is flat. ~keep
    pub classification_passes: usize,
}

impl AdoptBatchOutcome {
    /// Every target that could not be adopted, paired with why.
    pub fn failures(&self) -> impl Iterator<Item = (&str, &anyhow::Error)> {
        self.results.iter().filter_map(|(target, result)| match result {
            Ok(_) => None,
            Err(error) => Some((target.as_str(), error)),
        })
    }

    /// Every target's report, in target order, skipping the failed ones.
    pub fn reports(&self) -> impl Iterator<Item = &AdoptReport> {
        self.results.iter().filter_map(|(_, result)| result.as_ref().ok())
    }
}

/// State shared by every target of one invocation.
pub(crate) struct AdoptSession<'m> {
    managed: &'m [ManagedOutput],
    base_dir: PathBuf,
    cache: HashMap<PathBuf, Classification>,
    /// Diff rendering is a full `similar::TextDiff` over both sides of a file; a path two
    /// targets both select is diffed once. ~keep
    diff_bodies: HashMap<PathBuf, String>,
    /// `record_scaffold_owned_paths` is a read-modify-write of the entire manifest, so one
    /// call per target is one full parse and rewrite per target. Accumulated here and
    /// flushed once by [`Self::finish`]. ~keep
    to_record: Vec<PathBuf>,
    classification_passes: usize,
}

impl<'m> AdoptSession<'m> {
    pub(crate) fn new(base_dir: &Path, managed: &'m [ManagedOutput]) -> Self {
        Self {
            managed,
            base_dir: base_dir.to_path_buf(),
            cache: HashMap::new(),
            diff_bodies: HashMap::new(),
            to_record: Vec::new(),
            classification_passes: 0,
        }
    }

    pub(crate) fn classification_passes(&self) -> usize {
        self.classification_passes
    }

    /// Read and classify one managed output, or return the memoized classification.
    fn classify_output(&mut self, output: &ManagedOutput) -> Result<Classification> {
        if let Some(cached) = self.cache.get(&output.relative) {
            return Ok(cached.clone());
        }
        let full_path = self.base_dir.join(&output.relative);
        let classification = if full_path.exists() {
            let bytes = std::fs::read(&full_path)
                .with_context(|| format!("failed to read existing {}", full_path.display()))?;
            self.classification_passes += 1;
            self.classify_bytes(&full_path, output, bytes)
        } else {
            Classification::Absent
        };
        self.cache.insert(output.relative.clone(), classification.clone());
        Ok(classification)
    }

    fn classify_bytes(&self, full_path: &Path, output: &ManagedOutput, bytes: Vec<u8>) -> Classification {
        if crate::cli::pipeline::is_base64_binary_output(&output.relative) {
            return match classify_binary(&self.base_dir, full_path, output, &bytes) {
                Some(candidate) => Classification::Ready(Rc::new(candidate)),
                None => Classification::Unreadable,
            };
        }
        match String::from_utf8(bytes) {
            Ok(existing) => Classification::Ready(Rc::new(classify(
                &self.base_dir,
                full_path,
                &output.relative,
                &output.content,
                &existing,
                output.create_once,
            ))),
            Err(_) => Classification::Unreadable,
        }
    }

    /// Collect every managed output `matcher` selects, refusing targets alef does not
    /// generate and targets that do not exist on disk yet.
    fn select(&mut self, options: &AdoptOptions, matcher: &TargetMatcher) -> Result<SelectedCandidates> {
        // Copied out of `self` first: `matched` borrows the surface for `'m`, not for the
        // life of this `&mut self`, which is what lets classification run against it. ~keep
        let managed: &'m [ManagedOutput] = self.managed;
        let mut matched: Vec<&'m ManagedOutput> = managed
            .iter()
            .filter(|output| matcher.matches(&output.relative))
            .collect();
        matched.sort_by(|left, right| left.relative.cmp(&right.relative));

        if matched.is_empty() {
            // Refusing an unmatched target is the property that keeps `alef adopt` from
            // being a general-purpose "stamp this file" tool: a path alef does not generate
            // can never be adopted, whatever the human types. ~keep
            bail!(
                "no alef-managed output matches `{}` -- adopt only applies to paths alef generates",
                options.target
            );
        }

        let mut candidates = Vec::with_capacity(matched.len());
        let mut unreadable: Vec<PathBuf> = Vec::new();
        for output in matched {
            match self.classify_output(output)? {
                Classification::Ready(candidate) => candidates.push(candidate),
                Classification::Unreadable => unreadable.push(output.relative.clone()),
                Classification::Absent => {}
            }
        }

        if candidates.is_empty() && unreadable.is_empty() {
            // Distinct from the unmatched bail above: the path IS alef's, it just is not on disk
            // yet, so there is no ownership conflict to resolve -- generation simply writes it.
            // ~keep
            bail!(
                "`{}` matches alef-managed output but nothing exists on disk yet -- run `alef generate`",
                options.target
            );
        }
        Ok(SelectedCandidates { candidates, unreadable })
    }

    fn diff_body(&mut self, candidate: &AdoptCandidate) -> String {
        if let Some(body) = self.diff_bodies.get(&candidate.relative) {
            return body.clone();
        }
        let body = render_diff(candidate);
        self.diff_bodies.insert(candidate.relative.clone(), body.clone());
        body
    }

    /// Refresh the cached classification to exactly what re-reading the file after this
    /// adoption would produce, so a path a later target also selects sees what the
    /// per-target implementation's fresh read showed it.
    ///
    /// Three rails, and they genuinely differ. A stamped path now carries a marker, so it
    /// re-classifies [`AdoptionState::AlreadyOwned`]. A binary path's ownership went into
    /// `.alef-ownership.toml`, which `classify_binary` consults, so it is `AlreadyOwned`
    /// too. An unstampable *text* path changes no byte and its ownership record is not read
    /// back by [`classify`], so its cached verdict is already the one a re-read would give
    /// and is deliberately left alone — including the fact that a later target adopts it a
    /// second time, which is what the per-target loop did. ~keep
    fn refresh_after_apply(&mut self, candidate: &AdoptCandidate) {
        let refreshed = match (&candidate.stamped, &candidate.binary) {
            (Some(stamped), _) => classify(
                &self.base_dir,
                &candidate.full_path,
                &candidate.relative,
                &candidate.generated,
                stamped,
                candidate.create_once,
            ),
            (None, Some(facts)) => AdoptCandidate {
                relative: candidate.relative.clone(),
                full_path: candidate.full_path.clone(),
                existing: String::new(),
                generated: String::new(),
                state: AdoptionState::AlreadyOwned,
                stamped: None,
                create_once: candidate.create_once,
                binary: Some(facts.clone()),
            },
            (None, None) => return,
        };
        self.cache
            .insert(candidate.relative.clone(), Classification::Ready(Rc::new(refreshed)));
    }

    /// Decide and (under `--write`) apply one target.
    ///
    /// The body below is the whole of `alef adopt`'s decision, reached identically from the
    /// single-target and multi-target entry points. Nothing in it consults the session's
    /// caches for anything but cost. ~keep
    pub(crate) fn adopt_target(&mut self, options: &AdoptOptions, matcher: &TargetMatcher) -> Result<AdoptReport> {
        let SelectedCandidates { candidates, unreadable } = self.select(options, matcher)?;
        let mut report = AdoptReport {
            preview: !options.write,
            unreadable,
            ..AdoptReport::default()
        };

        // Partitioned before anything is classified for the report, so an excluded seed
        // contributes no diff, no converged tally and no adopted entry -- it is not a thing
        // this run is deciding about, it is a thing this run refuses to decide about.
        //
        // `AlreadyOwned` seeds are deliberately not excluded: the file already carries a
        // marker, so there is no consent left to give and no content this command could put
        // at risk. Listing them as blocked would be a false alarm on the exact list whose
        // whole value is that every line on it is genuinely dangerous. ~keep
        let (adoptable, blocked): (Vec<Rc<AdoptCandidate>>, Vec<Rc<AdoptCandidate>>) =
            candidates.into_iter().partition(|candidate| {
                options.clobber_create_once_seeds
                    || !candidate.create_once
                    || candidate.state == AdoptionState::AlreadyOwned
            });
        for candidate in &blocked {
            report.skipped_create_once.push(candidate.relative.clone());
            tracing::warn!(
                path = %candidate.relative.display(),
                "create-once seed: alef emits this path only when absent, so adopting it consents to alef \
                 replacing its contents with a placeholder seed on the next OVERWRITING regen -- an \
                 `alef version` sync or `alef all --clobber-create-once-seeds`. A plain `alef generate` \
                 skips it"
            );
        }

        for candidate in &adoptable {
            match candidate.state {
                AdoptionState::AlreadyOwned => report.already_owned.push(candidate.relative.clone()),
                AdoptionState::Converged => report.converged.push(candidate.relative.clone()),
                AdoptionState::Drifted => {
                    let body = self.diff_body(candidate);
                    report.diffs.push(AdoptDiff {
                        relative: candidate.relative.clone(),
                        state: candidate.state,
                        body,
                    });
                }
            }
        }

        for diff in report.drifted() {
            tracing::warn!(
                path = %diff.relative.display(),
                "content differs from generated output: adopting consents to alef replacing it on the next generate"
            );
        }

        if !options.write {
            return Ok(report);
        }

        // Non-zero only when the exclusion emptied the run, so a mixed glob still does its
        // legitimate work instead of forcing the operator to re-type a narrower target (and
        // learn, from the failure, that the way to make adopt succeed is to pass the
        // dangerous flag). A `--write` that adopted nothing at all is a different matter: it
        // exits 0 today only because "nothing matched" is already a `bail!`, and staying
        // silent here would let a seeds-only glob look like a successful adoption. ~keep
        let has_work = adoptable.iter().any(|c| c.state != AdoptionState::AlreadyOwned);
        if !has_work && !report.skipped_create_once.is_empty() {
            bail!(
                "`{}` matches only create-once seeds, so nothing was written. alef emits these only \
                 when absent, so adopting one consents to alef replacing its contents with a placeholder \
                 seed on the next OVERWRITING regen -- an `alef version` sync or `alef all \
                 --clobber-create-once-seeds`, not a plain `alef generate`. Pass \
                 --clobber-create-once-seeds to adopt them anyway.",
                options.target
            );
        }

        for candidate in adoptable.iter().filter(|c| c.state != AdoptionState::AlreadyOwned) {
            if options.converged_only && candidate.state == AdoptionState::Drifted {
                report.skipped_drifted.push(candidate.relative.clone());
                continue;
            }
            apply(candidate, &mut report, &mut self.to_record)?;
            self.refresh_after_apply(candidate);
            if candidate.state == AdoptionState::Drifted {
                // Per-file at DEBUG for converged paths and INFO only for drifted ones: a
                // converged bulk adoption is one event of 12,000 paths, and emitting 12,000
                // INFO lines for it drowns the drifted adoptions, which are the ones a reader
                // has to be able to find in the log afterwards. ~keep
                tracing::info!(path = %candidate.relative.display(), "adopted (drifted): marker stamped, content kept");
            } else {
                tracing::debug!(path = %candidate.relative.display(), "adopted (converged): marker stamped");
            }
        }
        Ok(report)
    }

    /// Write the accumulated ownership record once for the whole invocation.
    pub(crate) fn finish(self) -> Result<()> {
        let record_refs: Vec<&Path> = self.to_record.iter().map(PathBuf::as_path).collect();
        crate::cli::cache::record_scaffold_owned_paths(&self.base_dir, &record_refs)
    }
}

/// Adopt one target against a pre-computed managed surface.
pub(crate) fn run_single(options: &AdoptOptions, managed: &[ManagedOutput]) -> Result<AdoptReport> {
    let mut session = AdoptSession::new(&options.base_dir, managed);
    let matcher = TargetMatcher::new(&options.target);
    let report = session.adopt_target(options, &matcher)?;
    session.finish()?;
    Ok(report)
}

/// Adopt every target of one invocation against a single pre-computed managed surface.
///
/// One target's refusal must not silently cancel the other fifty-three:
/// [`AdoptSession::adopt_target`] bails whenever a target resolves to nothing adoptable --
/// no match, or (far more commonly on a repo-wide sweep) only create-once seeds -- and
/// propagating that straight out of the loop meant a single `config.m4` early in a sorted
/// list of 54 refused paths ended the command before one file was stamped, with an exit code
/// that named only that path. Each target's result is captured independently here and the
/// caller fails iff any did. ~keep
pub fn run_batch(
    targets: &[String],
    options: &AdoptBatchOptions,
    managed: &[ManagedOutput],
) -> Result<AdoptBatchOutcome> {
    let mut session = AdoptSession::new(&options.base_dir, managed);
    let mut results = Vec::with_capacity(targets.len());
    for target in targets {
        let per_target = options.for_target(target);
        let matcher = TargetMatcher::new(target);
        results.push((target.clone(), session.adopt_target(&per_target, &matcher)));
    }
    let classification_passes = session.classification_passes();
    tracing::debug!(
        targets = targets.len(),
        classifications = classification_passes,
        "adopt resolved every target against one managed surface"
    );
    session.finish()?;
    Ok(AdoptBatchOutcome {
        results,
        classification_passes,
    })
}

#[cfg(test)]
mod tests;
