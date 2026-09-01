//! Builds the message for a validator `Fail` that `finalize_result` reclassifies to
//! `SnippetStatus::Unavailable` because the toolchain's own output was dependency-shaped (see
//! `SnippetValidator::is_dependency_error`).
//!
//! Two different root causes reach that reclassification through the identical `Fail` ->
//! `Unavailable` shape, and they need different remediation advice:
//!
//! - A `docs.snippets.sessions` target *is* configured for the snippet's language, but this run's
//!   `alef build` has not produced the artifact that target's manifest points at yet (or
//!   `alef all --clean` just removed it). Building fixes this -- the "ordering problem" wording.
//! - *No* session is configured for the language at all, so the snippet validated in an isolated
//!   scratch directory with no manifest and no access to the real package -- see
//!   `TypeScriptValidator::check_batch`, `ZigValidator::validate_in_session`, and every other
//!   validator's `session.is_none()` branch, which all fall back to a bare temp directory. No
//!   amount of building can fix this: the toolchain was never told where the package lives.
//!
//! Before this module existed, both cases shared one "run `alef build`" message. That was false
//! advice for the second case, and it was the entire reason a consumer running `alef build` then
//! `alef snippets check` back to back saw byte-identical unresolved-dependency counts: the six
//! languages driving that count had no configured session, so no build this side of adding one
//! could ever change their result.
//!
//! `finalize_result` already knows which case applies from whether `session` -- the *prepared*
//! `ValidationSession` for this snippet's claim -- is `Some` or `None`: by the time it runs,
//! `None` only ever means no configured session claimed this language. Every session that *was*
//! claimed but failed to prepare short-circuits through
//! `session_prep::session_preparation_result` before a validator ever runs, so that case never
//! reaches `finalize_result` with `session: None`. ~keep

use crate::snippets::types::{Language, Snippet, ValidationLevel};

/// Precedes every reclassified message when no session was configured for the snippet's
/// language. [`crate::snippets::output::unresolved_dependency_rollup`] matches this exact phrase
/// to report the two causes on separate lines with separate remediation, without adding a new
/// serialized `ValidationResult` field purely for this one report. ~keep
pub(crate) const NO_SESSION_CONFIGURED_PHRASE: &str = "no snippet validation session is configured for";

/// The message for a validator `Fail` reclassified to `Unavailable` -- see the module doc for why
/// the wording branches on `no_session_configured`. `pub(crate)`, not `pub(super)`, so
/// `output`'s own tests can build a realistic fixture message instead of hand-duplicating this
/// wording -- see `output::tests::unresolved_with_no_session`.
///
/// `session_target` -- not `language` -- is what belongs in the actionable
/// `[workspace.docs.snippets.sessions.<target>]` hint (task #470): sessions are keyed by the
/// *target* a snippet resolves to (`node`, `wasm`, `kotlin_android`, ...), not by its language,
/// and a language like TypeScript or Kotlin can resolve to more than one target. Naming the
/// language there told a reader to add a config key (e.g. `[sessions.typescript]`) that would
/// never be the one the snippet's own `target:` front matter actually resolved against, sending
/// them to the wrong key. ~keep
pub(crate) fn unresolved_dependency_message(
    no_session_configured: bool,
    language: Language,
    session_target: &str,
    effective_level: ValidationLevel,
    raw_output: &str,
) -> String {
    if no_session_configured {
        format!(
            "could not validate at {effective_level}: {NO_SESSION_CONFIGURED_PHRASE} {language} snippets, so \
             this one ran in an isolated scratch directory with no access to the crate's built package. \
             `alef build` does not fix this; add a `[workspace.docs.snippets.sessions.{session_target}]` \
             entry: {raw_output}"
        )
    } else {
        format!(
            "could not validate at {effective_level}: {language} toolchain ran but reported a missing \
             dependency or build artifact -- run `alef build` first if this crate validates snippets against \
             built artifacts: {raw_output}"
        )
    }
}

/// The session key a reader must add for `snippet` -- see `unresolved_dependency_message`'s doc
/// comment for why this is the snippet's own resolved *target*, not its language (task #470).
/// `pub(super)`, called only from `runner::finalize_result`. ~keep
pub(super) fn session_target_hint(snippet: &Snippet) -> String {
    snippet
        .metadata
        .target
        .as_deref()
        .map(Language::normalize_session_target)
        .unwrap_or_else(|| snippet.language.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the split: a session-less reclassification must never send the reader
    /// to rebuild artifacts a session-less run could never see in the first place.
    #[test]
    fn no_session_configured_message_never_points_at_alef_build() {
        let message = unresolved_dependency_message(
            true,
            Language::Go,
            "go",
            ValidationLevel::Compile,
            "cannot find package",
        );

        assert!(message.contains(NO_SESSION_CONFIGURED_PHRASE), "{message}");
        assert!(
            !message.contains("run `alef build`"),
            "no-session message must not tell the reader to rebuild: {message}"
        );
        assert!(
            message.contains("cannot find package"),
            "raw validator output must survive: {message}"
        );
    }

    /// Regression for task #470: the actionable config-key hint must name the *session target*
    /// (what actually goes in `[workspace.docs.snippets.sessions.<target>]`), not the language --
    /// a snippet whose front matter declares `target: wasm` needs a `sessions.wasm` entry, and a
    /// message telling it to add `sessions.typescript` instead sends the reader to a config key
    /// that snippet's own session resolution would never look up. ~keep
    #[test]
    fn no_session_configured_message_names_the_target_not_the_language() {
        let message = unresolved_dependency_message(
            true,
            Language::TypeScript,
            "wasm",
            ValidationLevel::Compile,
            "cannot find module",
        );

        assert!(
            message.contains("sessions.wasm]"),
            "the actionable hint must name the target `wasm`, not the language: {message}"
        );
        assert!(
            !message.contains("sessions.typescript]"),
            "the actionable hint must not name the language as if it were the target: {message}"
        );
    }

    /// Negative control: the ordering case (a session exists) keeps the original remediation and
    /// must never claim no session is configured.
    #[test]
    fn ordering_message_still_points_at_alef_build() {
        let message = unresolved_dependency_message(
            false,
            Language::Go,
            "go",
            ValidationLevel::Compile,
            "cannot find package",
        );

        assert!(message.contains("run `alef build` first"), "{message}");
        assert!(!message.contains(NO_SESSION_CONFIGURED_PHRASE), "{message}");
        assert!(
            message.contains("cannot find package"),
            "raw validator output must survive: {message}"
        );
    }
}
