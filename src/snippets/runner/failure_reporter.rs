//! Live per-language reporting of snippet failures while a validation pass is still running.
//!
//! Split out of `runner.rs` unchanged when that file reached its 1,000-line ceiling: "tell the
//! operator what is going wrong, now, without emitting one line per failure" is one concern with
//! one reason to change, and it was the largest one `runner.rs` still carried. ~keep

use crate::snippets::types::{Snippet, SnippetStatus, ValidationResult};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// A language emits one `WARN` for its first failure, then one more every this many failures.
/// Sized so a pathological run (1,753 failures spread over six languages) produces on the order of
/// seventy lines rather than one per failure, while a language that fails a handful of times still
/// gets its first-failure line immediately.
pub(super) const FAILURE_PROGRESS_STRIDE: usize = 25;

/// How much of a validator's own error output the first-failure event carries. Long enough for a
/// `javac`/`tsc` diagnostic's first lines (the part that names the actual problem), short enough
/// that a log line stays a log line.
pub(super) const FAILURE_MESSAGE_PREVIEW_CHARS: usize = 400;

#[derive(Clone, Copy, Default)]
struct LanguageTally {
    completed: usize,
    failed: usize,
    unavailable: usize,
    /// Snippets of this language that actually reached a toolchain invocation, as counted by
    /// [`FailureReporter::record_toolchain_start`]. ~keep Distinct from `completed`, which counts
    /// every result including the ones `validate_one` short-circuits without running anything.
    invoked: usize,
}

/// Emits snippet failures *while* a validation pass is running.
///
/// Everything reported here was already known at the moment each `ValidationResult` was built —
/// `validate_one`/`validate_batches` hold the status and the validator's message, and
/// `finalize_result` writes both to the result cache. None of it reached the log: a run that
/// produced 1,753 failures across six languages failing at 100% was indistinguishable from a
/// healthy run until the stage ended and the summary printed.
///
/// One event per failure would be as unreadable as silence, so the budget is bounded per language:
/// the first failure carries the validator's own message (the only part that says *what* broke),
/// subsequent failures are counted and surfaced every [`FAILURE_PROGRESS_STRIDE`], and each
/// language emits exactly one terminal event once its last snippet lands — which is mid-run for
/// every language but the slowest, because `parallel_results` interleaves all languages.
pub(super) struct FailureReporter {
    totals: BTreeMap<crate::snippets::types::Language, usize>,
    tallies: Mutex<BTreeMap<crate::snippets::types::Language, LanguageTally>>,
    span: tracing::Span,
}

impl FailureReporter {
    pub(super) fn new(snippets: &[Snippet]) -> Self {
        let mut totals = BTreeMap::new();
        for snippet in snippets {
            *totals.entry(snippet.language).or_insert(0_usize) += 1;
        }
        Self {
            totals,
            tallies: Mutex::new(BTreeMap::new()),
            // Recorded from the constructing thread, which `run_validation` has already put in the
            // caller's span, and re-entered on every emission. Most `record` calls happen on a
            // rayon worker that `pool.install`'s one-off `Span::enter` never reached, so without
            // this the events a consumer most needs to correlate would be the span-less ones. ~keep
            span: tracing::Span::current(),
        }
    }

    /// Announce, once per language, that a snippet of that language has reached an actual
    /// toolchain invocation, and count every such invocation.
    ///
    /// ~keep This is called from the one place real work begins, rather than inferred beforehand
    /// from `validate_batches` leaving an entry unclaimed. That inference was wrong: `validate_one`
    /// short-circuits on a cache hit, a `skip` annotation, a side-effect rejection, a missing
    /// validator and an unavailable toolchain, none of which run anything. Because
    /// `docs::validate_snippets` runs once per crate with `changed_only`, later crates are ~100%
    /// cache hits, so every language announced `Starting per-snippet validation snippet_count=521`
    /// while doing nothing at all -- which read as though 13 of 14 languages had fallen out of
    /// batching when in fact only four had. Deriving the event from the work itself cannot drift
    /// from it the way a parallel predicate can.
    pub(super) fn record_toolchain_start(&self, language: crate::snippets::types::Language, timeout_secs: u64) {
        let Ok(mut tallies) = self.tallies.lock() else {
            return;
        };
        let tally = tallies.entry(language).or_default();
        tally.invoked += 1;
        let first = tally.invoked == 1;
        drop(tallies);
        if !first {
            return;
        }
        let snippet_count = self.totals.get(&language).copied().unwrap_or(0);
        self.span.in_scope(|| {
            tracing::info!(
                language = %language,
                snippet_count = snippet_count,
                timeout_secs = timeout_secs,
                "Starting per-snippet validation"
            );
        });
    }

    /// Per-language counts of snippets that actually invoked a toolchain.
    pub(super) fn invoked_by_language(&self) -> BTreeMap<crate::snippets::types::Language, usize> {
        let Ok(tallies) = self.tallies.lock() else {
            return BTreeMap::new();
        };
        tallies
            .iter()
            .filter(|(_, tally)| tally.invoked > 0)
            .map(|(language, tally)| (*language, tally.invoked))
            .collect()
    }

    pub(super) fn record(&self, result: &ValidationResult) {
        let language = result.snippet.language;
        let failed = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
        // `Unavailable` is tallied separately rather than folded into `failed`, and it is tallied at
        // all because it is not the harmless outcome its name suggests: under `strict` it fails the
        // run exactly like a `Fail`, and the `unresolved_dependency` reclassification below turns a
        // real validator `Fail` -- diagnostic and all -- into one. Counting only `Fail | Error` is
        // how 566 snippets across two languages reached the final summary as
        // "283 unresolved dependency" apiece with not one line anywhere in the log saying WHICH
        // dependency, while the validator's own message sat unread on every result. ~keep
        let unavailable = matches!(result.status, SnippetStatus::Unavailable);
        // A poisoned tally lock costs reporting only. Unwrapping here would let a panic in some
        // other worker's reporting turn a reportable run into an aborted one, which is exactly the
        // failure mode this reporter exists to prevent. ~keep
        let Ok(mut tallies) = self.tallies.lock() else {
            return;
        };
        let tally = tallies.entry(language).or_default();
        tally.completed += 1;
        if failed {
            tally.failed += 1;
        }
        if unavailable {
            tally.unavailable += 1;
        }
        let tally = *tally;
        drop(tallies);

        let snippet_count = self.totals.get(&language).copied().unwrap_or(tally.completed);
        self.span.in_scope(|| {
            Self::report_progress(result, tally, snippet_count);
            Self::report_completion(language, tally, snippet_count);
        });
    }

    fn report_progress(result: &ValidationResult, tally: LanguageTally, snippet_count: usize) {
        let language = result.snippet.language;
        let failed = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
        let unavailable = matches!(result.status, SnippetStatus::Unavailable);
        if failed && tally.failed == 1 {
            tracing::warn!(
                language = %language,
                path = %result.snippet.source_origin.path.display(),
                line = result.snippet.source_origin.line,
                snippet_count = snippet_count,
                error = %failure_preview(result.message.as_deref()),
                "First snippet validation failure for this language"
            );
        } else if unavailable && tally.unavailable == 1 {
            tracing::warn!(
                language = %language,
                path = %result.snippet.source_origin.path.display(),
                line = result.snippet.source_origin.line,
                snippet_count = snippet_count,
                unresolved_dependency = result.unresolved_dependency,
                error = %failure_preview(result.message.as_deref()),
                "First snippet validation unavailability for this language"
            );
        } else if failed && tally.failed.is_multiple_of(FAILURE_PROGRESS_STRIDE) {
            tracing::warn!(
                language = %language,
                failed = tally.failed,
                completed = tally.completed,
                snippet_count = snippet_count,
                "Snippet validation failures accumulating"
            );
        }
    }

    fn report_completion(language: crate::snippets::types::Language, tally: LanguageTally, snippet_count: usize) {
        if tally.completed < snippet_count {
            return;
        }
        if tally.failed > 0 {
            tracing::warn!(
                language = %language,
                failed = tally.failed,
                unavailable = tally.unavailable,
                snippet_count = snippet_count,
                "Finished snippet validation for this language with failures"
            );
        } else if tally.unavailable > 0 {
            tracing::warn!(
                language = %language,
                unavailable = tally.unavailable,
                snippet_count = snippet_count,
                "Finished snippet validation for this language with every result unvalidated"
            );
        } else {
            tracing::debug!(
                language = %language,
                snippet_count = snippet_count,
                "Finished snippet validation for this language"
            );
        }
    }
}

/// Flattens a validator's multi-line diagnostic onto one bounded log line. Truncation is by
/// character, not byte, so a diagnostic quoting non-ASCII source cannot panic on a split boundary.
pub(super) fn failure_preview(message: Option<&str>) -> String {
    let joined = message
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        return "<no validator output>".to_string();
    }
    match joined.char_indices().nth(FAILURE_MESSAGE_PREVIEW_CHARS) {
        Some((index, _)) => format!("{}...", &joined[..index]),
        None => joined,
    }
}
