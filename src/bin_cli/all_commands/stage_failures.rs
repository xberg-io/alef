//! Accumulates stage failures across a whole run so one rejected fixture, one backend's
//! post-build failure, or one crate's snippet-coverage gap can never deny every other stage --
//! and every other crate -- its regeneration (task #186).
//!
//! Shared by `alef all` (task #186), `alef generate` (task #546) and `alef docs` (alef#475), so
//! nothing it emits may name a single command: the same line is printed under all three. ~keep
//!
//! Before this existed, a failure in the pre-flight snippet-coverage check or in a crate's
//! post-build processing propagated with `?`/`return Err` immediately, which meant: (a) codegen
//! for every other crate in a multi-crate workspace never ran at all, and (b) even the FAILING
//! crate's own later stages (stubs, scaffold, e2e, docs) were abandoned mid-run. Both are worse
//! than the failure they report -- an operator who fixes the one broken fixture or backend still
//! has to invoke every remaining stage by hand to get a regen. This type lets every caller record
//! a failure and keep going; the accumulated failures are reported together, once, at the very
//! end, so the run still exits non-zero but names everything that went wrong instead of only the
//! first thing. ~keep
pub(crate) struct StageFailures {
    entries: Vec<(String, anyhow::Error)>,
}

impl StageFailures {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Record one stage's failure under `label` (e.g. `"[crate-name] post-build processing"`)
    /// and log it immediately, so it is visible in real time rather than only in the final
    /// summary a caller may never scroll back to. Takes `error` by value, not by reference: it
    /// is carried through to [`Self::into_result`] verbatim, `.context(...)` chain and all, when
    /// this turns out to be the only recorded failure -- see that method's doc comment.
    pub(crate) fn record(&mut self, label: &str, error: anyhow::Error) {
        tracing::error!("{label} failed -- continuing with the remaining stages: {error:#}");
        self.entries.push((label.to_string(), error));
    }

    /// `Ok(())` when nothing was recorded. When exactly one failure was recorded, returns that
    /// `anyhow::Error` unchanged -- not rebuilt into a new string-based error -- so a caller that
    /// applied `.context(...)` before recording (e.g. the docs-stage refusal-count wrapping in
    /// `all_commands.rs`) keeps that context, its source chain, and any typed error a downstream
    /// `downcast_ref` might still want. Only a run with two or more distinct failures pays for a
    /// synthesized summary, because that is the only shape a single `anyhow::Error` cannot
    /// represent at all: every stage this run could still attempt already ran by the time a
    /// caller reaches this, so the non-zero exit reports the failures, it does not withhold work.
    pub(crate) fn into_result(mut self) -> anyhow::Result<()> {
        match self.entries.len() {
            0 => Ok(()),
            1 => {
                let (_, error) = self.entries.pop().expect("length checked above");
                Err(error)
            }
            _ => {
                let mut message = format!(
                    "finished with {} stage failure(s); every stage that could still run did, \
                     and wrote its output -- fix each failure below, then re-run:",
                    self.entries.len()
                );
                for (label, error) in &self.entries {
                    message.push_str(&format!("\n  - {label}: {error:#}"));
                }
                Err(anyhow::anyhow!(message))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StageFailures;

    #[test]
    fn no_recorded_failures_is_ok() {
        assert!(StageFailures::new().into_result().is_ok());
    }

    /// A single recorded failure must come back exactly as it went in -- same `Display` text,
    /// including any `.context(...)` a caller applied before recording it -- not rebuilt into a
    /// generic summary string. This is the common case (one crate, one failing stage) and the
    /// one every existing `all_commands.rs` test asserting on a deferred error's exact text
    /// depends on. ~keep
    #[test]
    fn a_single_recorded_failure_is_returned_unchanged() {
        let mut failures = StageFailures::new();
        let original = anyhow::anyhow!("snippet validation failed").context("2 refused write(s)");
        let original_text = format!("{original:#}");
        failures.record("docs/snippet validation", original);

        let error = failures
            .into_result()
            .expect_err("one recorded failure must still bail");

        assert_eq!(
            format!("{error:#}"),
            original_text,
            "the error must not be rebuilt or reformatted"
        );
    }

    /// The decisive assertion for the defect this type fixes: every recorded failure must
    /// survive into the final message, not just the first one -- see `all_commands.rs`'s old
    /// `if let Some(error) = e2e_stage_error { return Err(error); }` followed by an unreachable
    /// `docs_stage_error` check, which silently dropped a second category's failure whenever the
    /// first was also present. ~keep
    #[test]
    fn every_recorded_failure_survives_into_the_final_message() {
        let mut failures = StageFailures::new();
        failures.record("[crate-a] post-build processing", anyhow::anyhow!("cargo build failed"));
        failures.record(
            "[crate-b] snippet coverage precondition",
            anyhow::anyhow!("1 undocumented gap"),
        );
        failures.record(
            "docs/snippet validation",
            anyhow::anyhow!("strict snippet validation failed"),
        );

        let message = format!("{:#}", failures.into_result().expect_err("recorded failures must bail"));

        assert!(message.contains("3 stage failure(s)"), "got: {message}");
        assert!(
            message.contains("[crate-a] post-build processing: cargo build failed"),
            "got: {message}"
        );
        assert!(
            message.contains("[crate-b] snippet coverage precondition: 1 undocumented gap"),
            "got: {message}"
        );
        assert!(
            message.contains("docs/snippet validation: strict snippet validation failed"),
            "got: {message}"
        );
    }
}
