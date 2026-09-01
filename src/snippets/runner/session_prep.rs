//! Classifies a snippet whose *session* never became usable -- as opposed to one whose validator
//! ran and produced a real result. Before this module existed, both `validate_one` (the
//! fail-fast path) and `batch::group_batchable_snippets` (the batched/parallel path) built their
//! own `SnippetStatus::Error` result straight from the stringified session preparation error,
//! independently of each other. That duplication is exactly how alef defect #142 happened: a
//! session's `before` hook (which builds this language's artifacts) timing out and a session's
//! manifest simply not existing produced the identical `SnippetStatus::Error`, with a message
//! that -- for the timeout case -- was just the raw "command timed out after 120s" text. A reader
//! could not tell an unbuilt artifact from a broken snippet from a misconfigured session; all
//! three read as "validation failed". One function, used by both dispatch paths, is what keeps
//! them from drifting apart again.

use super::session_resolution::{SessionClaim, resolve_session_claim};
use super::{RunnerConfig, result};
use crate::snippets::session::SessionPreparationError;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationResult};
use std::collections::HashMap;

/// The preparation error (if any) that applies to `snippet`: a real failure of the session an
/// explicit `target` names, or -- new with alef defect #127 -- a synthetic error when the
/// snippet's language matches more than one configured session and no `target` breaks the tie.
/// Both dispatch paths (`fail_fast_results`, `batch::group_batchable_snippets`) already
/// short-circuit on `Some` here before `session_for`/`session_key` ever run, so an ambiguous
/// claim never reaches a validator at all.
///
/// The no-explicit-target branch resolves against `config.sessions` -- every *configured*
/// `SessionSpec`, prepared or not -- rather than only the sessions that prepared successfully.
/// A language with exactly one configured session must still report that session's own
/// preparation failure even though the failed session never made it into a `ValidationSession`
/// map; resolving against successes only would make that single candidate vanish and let the
/// snippet reach a validator with no session and no error, exactly the silent-success shape
/// defect #142 already fixed once for a differently-shaped bug. ~keep
pub(super) fn session_preparation_error(
    snippet: &Snippet,
    config: &RunnerConfig,
    errors: &HashMap<String, SessionPreparationError>,
) -> Option<SessionPreparationError> {
    if let Some(target) = snippet.metadata.target.as_deref() {
        let normalized = Language::normalize_session_target(target);
        return errors.get(&normalized).cloned();
    }
    match resolve_session_claim(snippet, &config.sessions, |spec| spec.language) {
        SessionClaim::Ambiguous(candidates) => Some(ambiguous_session_error(snippet, &candidates)),
        SessionClaim::Claimed(key) => errors.get(key).cloned(),
        SessionClaim::Unclaimed => None,
    }
}

/// The `SessionPreparationError` synthesized for an ambiguous claim -- not `ordering`, because
/// this is a standing configuration gap (add an explicit `target:`), not a build the next run
/// might resolve on its own. `session_preparation_result` turns a non-`ordering` error into
/// `SnippetStatus::Error`, which fails every run regardless of `--strict`: a snippet alef cannot
/// even decide which session to validate against must never read as a clean pass. ~keep
fn ambiguous_session_error(snippet: &Snippet, candidates: &[&str]) -> SessionPreparationError {
    SessionPreparationError {
        message: format!(
            "{} configured sessions validate `{}` snippets ({}) and no explicit `target:` names one of \
             them; add a `target:` naming the session this snippet validates against",
            candidates.len(),
            snippet.language,
            candidates.join(", "),
        ),
        ordering: false,
    }
}

/// Builds the `ValidationResult` for a snippet whose session never became usable.
///
/// An `ordering` preparation error -- the session's `before` hook hit `timeout_secs` while
/// building this language's artifacts, not while validating any particular snippet -- reclassifies
/// through the same `unresolved_dependency` bucket a validator's own dependency-shaped `Fail`
/// uses (see `runner::finalize_result`): both share the same root cause, an artifact `alef build`
/// was supposed to produce that validation could not see yet. Every other preparation failure (a
/// missing manifest, a missing working directory, a `before` hook that ran to completion and
/// failed on its own terms) stays `SnippetStatus::Error`: that is broken configuration, not a
/// build-order gap, and must not be waved through the way an ordering problem is.
pub(super) fn session_preparation_result(
    snippet: &Snippet,
    config: &RunnerConfig,
    error: &SessionPreparationError,
) -> ValidationResult {
    let status = if error.ordering {
        SnippetStatus::Unavailable
    } else {
        SnippetStatus::Error
    };
    let mut outcome = result(
        snippet,
        status,
        config.level,
        config.level,
        Some(error.message.clone()),
        0,
    );
    outcome.unresolved_dependency = error.ordering;
    outcome
}
