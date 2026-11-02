// This module is a sanctioned stdout reporting surface. Every function here formats and prints a
// report table (validation summary, snippet listing) that is the primary deliverable of its
// `alef snippets` subcommand, not a diagnostic. Routing each of the many `println!` calls below
// through `crate::bin_cli::output::line` would be pure churn for no behavioral difference, so the
// whole module carries the allow instead of one per call site. ~keep
#![allow(clippy::print_stdout)]

use crate::snippets::error::Result;
use crate::snippets::types::{RunSummary, Snippet, SnippetStatus, ValidationResult};
use std::collections::BTreeMap;
use std::path::Path;

pub fn print_summary(summary: &RunSummary, show_code: bool) {
    println!();
    println!(
        "{:<60} {:<12} {:<10} {:<8} TIME",
        "SNIPPET", "LANGUAGE", "STATUS", "LEVEL"
    );
    println!("{}", "-".repeat(100));

    for result in &summary.results {
        print_result_row(result);
        if matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error) {
            print_failure_details(result, show_code);
        } else if let Some(line) = reason_line(result) {
            println!("{line}");
        }
    }

    println!("{}", "-".repeat(100));
    if let Some(line) = checked_vs_claimed_line(summary) {
        println!("{line}");
    }
    println!(
        "Total: {}  Passed: {}  Downgraded: {}  Failed: {}  Skipped: {}  Errors: {}  Unavailable: {}",
        summary.total,
        summary.passed,
        summary.downgraded,
        summary.failed,
        summary.skipped,
        summary.errors,
        summary.unavailable
    );
    if let Some(line) = timeout_line(summary) {
        println!("{line}");
    }
    if let Some(line) = preflight_skip_line(summary) {
        println!("{line}");
    }
    if let Some(line) = unresolved_dependency_rollup(summary) {
        println!("{line}");
    }
    println!();
}

fn print_result_row(result: &ValidationResult) {
    let file_name = result
        .snippet
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");

    let status = match result.status {
        SnippetStatus::Pass => "PASS",
        SnippetStatus::Downgraded => "DOWNGRADE",
        SnippetStatus::Fail => "FAIL",
        SnippetStatus::Skip => "SKIP",
        SnippetStatus::Error => "ERROR",
        SnippetStatus::Unavailable => "N/A",
    };

    println!(
        "{:<60} {:<12} {:<10} {:<8} {}ms",
        truncate(file_name, 58),
        display_language(&result.snippet),
        status,
        result.effective_level,
        result.duration_ms
    );
}

fn print_failure_details(result: &ValidationResult, show_code: bool) {
    let title_info = result
        .snippet
        .title
        .as_deref()
        .map(|title| format!(" (title: {title})"))
        .unwrap_or_default();
    println!(
        "  Source: {}:{}{}",
        result.snippet.path.display(),
        result.snippet.start_line,
        title_info
    );

    if let Some(message) = &result.message {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            println!("  Error:");
            for line in trimmed.lines() {
                println!("    {line}");
            }
        }
    }

    if show_code {
        println!("  Code:");
        for (index, line) in result.snippet.code.lines().enumerate() {
            println!("    {:>3} | {line}", index + 1);
        }
    }

    println!();
}

/// States how much of `Errors` is a stopwatch reading rather than a verdict, or `None` when no
/// result timed out.
///
/// The count stays inside `Errors` -- a toolchain that never finishes is a real problem and must
/// still fail the run -- but "32 failed, 0 errors" and "411 failed" were both reported by
/// consumers as if they named broken snippets, when a large part of each was invocations that ran
/// out of clock against artifacts that were never built. A timeout says nothing whatsoever about
/// the snippet: the compiler was killed before it reached a verdict. ~keep
fn timeout_line(summary: &RunSummary) -> Option<String> {
    if summary.timed_out == 0 {
        return None;
    }
    Some(format!(
        "Timed out: {} of {} (counted in Errors) -- these invocations were killed at the timeout before reporting \
         on the snippet, so they measure the budget, not the corpus. Raise `docs.snippets.timeout_secs`, or build \
         the artifacts they were waiting on.",
        summary.timed_out, summary.total
    ))
}

/// States how many snippets were never handed to a toolchain because the preflight already knew
/// their session's build artifacts were absent, or `None` when none were.
///
/// Printed as its own line rather than folded into the `unavailable` total precisely because a
/// skip that disappears into a bucket is indistinguishable from a check that ran. These snippets
/// were NOT validated and NOT passed. ~keep
fn preflight_skip_line(summary: &RunSummary) -> Option<String> {
    if summary.preflight_skipped == 0 {
        return None;
    }
    Some(format!(
        "Skipped before spawning: {} of {} -- their session's build artifacts do not exist, so no validator was \
         run for them and nothing about them was checked. Run `alef build` first, or pass \
         --skip-snippet-validation for a deliberately generate-only run.",
        summary.preflight_skipped, summary.total
    ))
}

/// The line task #488 exists for: how much of the corpus was actually checked at the level it
/// requested, stated first and prominently, not left for a reader to reconstruct by subtracting
/// `capability_capped`/`declared_capped`/`unavailable` from `passed` themselves. A run's own
/// `passed` count includes every `capability_capped`/`declared_capped` `Pass` -- a snippet that
/// never ran at the level it asked for -- so "1482 passed" alone cannot answer "how much of this
/// was actually checked", and burying the answer in per-result `downgrade_reason` fields is
/// exactly what let a run where 684 of 1985 results never validated at their requested level
/// still read as a success. `None` only when there is nothing to report at all (`total == 0`);
/// a fully clean run still prints "1985/1985 (100%)" rather than staying silent, so the absence
/// of this line is never itself ambiguous between "nothing to report" and "the tool forgot". ~keep
fn checked_vs_claimed_line(summary: &RunSummary) -> Option<String> {
    if summary.total == 0 {
        return None;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a snippet corpus is well within f64's exact integer range; this is a display percentage"
    )]
    let percent = (summary.fully_verified as f64 / summary.total as f64) * 100.0;
    Some(format!(
        "Checked at requested level: {}/{} ({percent:.1}%) -- capability-capped {}, declared-capped {}, \
         unavailable {}, downgraded {}, failed {}, skipped {}, errors {} did not fully verify",
        summary.fully_verified,
        summary.total,
        summary.capability_capped,
        summary.declared_capped,
        summary.unavailable,
        summary.downgraded,
        summary.failed,
        summary.skipped,
        summary.errors
    ))
}

/// Lines naming the languages whose results were reclassified as `unresolved_dependency`, or
/// `None` when there were none.
///
/// The per-row reclassification landed in `ValidationResult::unresolved_dependency`, but the
/// report still presented those rows one at a time — and when a language's package was never
/// built, *every* snippet in it reclassifies, so the reader sees hundreds of rows with a single
/// upstream cause and no statement of that cause anywhere. Rolling them up per language is what
/// turns "376 typescript results" into "the typescript package was not built": the count is
/// evidence of one environmental fact, not of 376 problems. ~keep
///
/// Two upstream causes reach `unresolved_dependency`, and only one of them is fixed by `alef
/// build`: see `runner::dependency_reclassification`'s module doc. Every reclassified message
/// this run wrote carries [`crate::snippets::runner::NO_SESSION_CONFIGURED_PHRASE`] when the
/// cause was "no session configured" -- matching it here is what keeps this report from repeating
/// the "run `alef build`" advice for a language `alef build` can never fix. Before this split, a
/// consumer running `alef build` then `alef snippets check` back to back saw byte-identical
/// counts for exactly these languages and had no way to tell that from a real ordering gap. ~keep
fn unresolved_dependency_rollup(summary: &RunSummary) -> Option<String> {
    if summary.unresolved_dependency == 0 {
        return None;
    }
    let mut no_session: BTreeMap<String, usize> = BTreeMap::new();
    let mut unbuilt: BTreeMap<String, usize> = BTreeMap::new();
    for result in summary.results.iter().filter(|result| result.unresolved_dependency) {
        let bucket = if result
            .message
            .as_deref()
            .is_some_and(|message| message.contains(crate::snippets::runner::NO_SESSION_CONFIGURED_PHRASE))
        {
            &mut no_session
        } else {
            &mut unbuilt
        };
        *bucket.entry(display_language(&result.snippet)).or_default() += 1;
    }
    let lines: Vec<String> = [
        rollup_line(
            "No session configured",
            &no_session,
            summary.total,
            "no `[workspace.docs.snippets.sessions.<target>]` is configured for these languages, so their \
             snippets validated in an isolated scratch directory with no access to the built package. Running \
             `alef build` cannot fix this -- configure a session for each language before validating.",
        ),
        rollup_line(
            "Unresolved dependencies",
            &unbuilt,
            summary.total,
            "these languages' packages were not built, so their snippets were never really validated. Run \
             `alef build` before validating; the counts above measure the environment, not the snippets.",
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// One rollup line for one `unresolved_dependency` bucket, or `None` when that bucket is empty.
/// `total` is the run's overall snippet count (`RunSummary::total`), not this bucket's own count,
/// so a reader can see what fraction of the *whole run* this bucket's cause explains. ~keep
fn rollup_line(label: &str, per_language: &BTreeMap<String, usize>, total: usize, remediation: &str) -> Option<String> {
    if per_language.is_empty() {
        return None;
    }
    let bucket_total: usize = per_language.values().sum();
    let breakdown = per_language
        .iter()
        .map(|(language, count)| format!("{language} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{label}: {bucket_total} of {total} ({breakdown}) -- {remediation}"
    ))
}

/// The `  Reason: ...` line for a row whose effective level differs from what was requested, or
/// `None` for a clean result. Covers every `downgrade_reason` the runner sets, not just
/// `Downgraded`/`capability_capped` — a `Pass` clamped by a snippet's own declared `level:`
/// (`DowngradeReason::Declared`) carries a reason too, and leaving it out of this table is what
/// let `docs.snippets.validation_level = "run"` clamp silently down to a fixture's stamped
/// `typecheck` ceiling with no visible trace in the human-readable report. ~keep
fn reason_line(result: &ValidationResult) -> Option<String> {
    result.downgrade_reason?;
    let message = result.message.as_deref()?.trim();
    (!message.is_empty()).then(|| format!("  Reason: {message}"))
}

/// Write validation results to a JSON file.
///
/// # Errors
///
/// Returns an error when serialization fails or the destination cannot be written.
pub fn write_json(summary: &RunSummary, path: &Path, show_code: bool) -> Result<()> {
    let mut value = serde_json::to_value(summary)?;
    if !show_code && let Some(results) = value.get_mut("results").and_then(serde_json::Value::as_array_mut) {
        for result in results {
            if let Some(snippet) = result.get_mut("snippet").and_then(serde_json::Value::as_object_mut) {
                snippet.remove("code");
            }
        }
    }
    let json = serde_json::to_string_pretty(&value)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Write a versioned summary as JSON or TOON, selected by the destination extension.
///
/// # Errors
///
/// Returns an error when serialization or writing fails.
pub fn write_report(summary: &RunSummary, path: &Path, show_code: bool) -> Result<()> {
    if path.extension().and_then(|value| value.to_str()) != Some("toon") {
        return write_json(summary, path, show_code);
    }
    let mut output = format!(
        "schema_version: {}\ntotal: {}\npassed: {}\ndowngraded: {}\nfailed: {}\nskipped: {}\nerrors: {}\nunavailable: {}\nfully_verified: {}\nresults[{}]:\n",
        summary.schema_version,
        summary.total,
        summary.passed,
        summary.downgraded,
        summary.failed,
        summary.skipped,
        summary.errors,
        summary.unavailable,
        summary.fully_verified,
        summary.results.len()
    );
    for result in &summary.results {
        output.push_str(&format!(
            "  - path: {}\n    line: {}\n    language: {}\n    status: {}\n    requested_level: {}\n    effective_level: {}\n",
            result.snippet.source_origin.path.display(),
            result.snippet.source_origin.line,
            display_language(&result.snippet),
            result.status,
            result.requested_level,
            result.effective_level
        ));
        if show_code {
            output.push_str("    code: |\n");
            for line in result.snippet.code.lines() {
                output.push_str(&format!("      {line}\n"));
            }
        }
    }
    std::fs::write(path, output)?;
    Ok(())
}

fn display_language(snippet: &Snippet) -> String {
    snippet.metadata.target.as_ref().map_or_else(
        || snippet.language.to_string(),
        |target| format!("{}/{target}", snippet.language),
    )
}

pub fn print_snippet_list(snippets: &[Snippet]) {
    println!("{:<60} {:<12} {:<8} TITLE", "FILE", "LANGUAGE", "LINE");
    println!("{}", "-".repeat(95));

    for snippet in snippets {
        let file_name = snippet.path.file_name().and_then(|name| name.to_str()).unwrap_or("?");

        println!(
            "{:<60} {:<12} {:<8} {}",
            truncate(file_name, 58),
            snippet.language,
            snippet.start_line,
            snippet.title.as_deref().unwrap_or("-")
        );
    }

    println!("{}", "-".repeat(95));
    println!("Total: {} snippets", snippets.len());
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else {
        format!("{}...", &value[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{DowngradeReason, Language, SnippetMetadata, SourceOrigin, ValidationLevel};

    fn sample_result(
        status: SnippetStatus,
        downgrade_reason: Option<DowngradeReason>,
        message: Option<&str>,
    ) -> ValidationResult {
        ValidationResult {
            snippet: Snippet {
                id: None,
                path: "example.md".into(),
                language: Language::Rust,
                title: None,
                code: "fn main() {}".into(),
                start_line: 1,
                block_index: 0,
                annotation: None,
                metadata: SnippetMetadata::default(),
                source_origin: SourceOrigin {
                    path: "example.md".into(),
                    line: 1,
                    block_index: 0,
                },
            },
            status,
            level: ValidationLevel::TypeCheck,
            requested_level: ValidationLevel::Run,
            effective_level: ValidationLevel::TypeCheck,
            message: message.map(str::to_owned),
            duration_ms: 0,
            capability_capped: false,
            downgrade_reason,
            unresolved_dependency: false,
            timed_out: false,
            preflight_skipped: false,
        }
    }

    /// Task #488's whole point: the prominent line must not silently disappear when nothing was
    /// checked -- `total == 0` (nothing discovered) is the only case that suppresses it, so a
    /// reader can never confuse "checked nothing because of a bug" with "no report line printed".
    #[test]
    fn checked_vs_claimed_line_is_absent_only_when_the_run_is_empty() {
        let summary = RunSummary::from_results(vec![]);

        assert_eq!(checked_vs_claimed_line(&summary), None);
    }

    /// The regression this line exists for: a run where most of the corpus was
    /// `capability_capped`/`unavailable` must state that fraction plainly, not leave a reader to
    /// infer it from `passed` alone -- `passed` includes every `capability_capped` result too. ~keep
    #[test]
    fn checked_vs_claimed_line_reports_the_verified_fraction_and_the_caveated_buckets() {
        let mut capability_capped =
            sample_result(SnippetStatus::Pass, Some(DowngradeReason::ValidatorCapability), None);
        capability_capped.capability_capped = true;
        let summary = RunSummary::from_results(vec![
            sample_result(SnippetStatus::Pass, None, None),
            capability_capped,
            sample_result(SnippetStatus::Unavailable, None, None),
        ]);

        let line = checked_vs_claimed_line(&summary).expect("non-empty run reports the line");

        assert!(line.starts_with("Checked at requested level: 1/3"), "{line}");
        assert!(line.contains("capability-capped 1"), "{line}");
        assert!(line.contains("unavailable 1"), "{line}");
    }

    /// Negative control: a fully clean run reports 100%, not silence -- the line's absence must
    /// never be ambiguous with "everything passed".
    #[test]
    fn checked_vs_claimed_line_reports_full_coverage_on_a_clean_run() {
        let summary = RunSummary::from_results(vec![
            sample_result(SnippetStatus::Pass, None, None),
            sample_result(SnippetStatus::Pass, None, None),
        ]);

        let line = checked_vs_claimed_line(&summary).expect("non-empty run reports the line");

        assert!(line.starts_with("Checked at requested level: 2/2 (100.0%)"), "{line}");
    }

    /// The regression this exists for: a `Pass` clamped by a snippet's own declared `level:`
    /// front matter used to print nothing at all — `print_summary`'s reason line only fired for
    /// `Downgraded` or `capability_capped` rows, so `docs.snippets.validation_level = "run"`
    /// clamped to a fixture's stamped `typecheck` ceiling with no visible trace anywhere in the
    /// human-readable report. ~keep
    #[test]
    fn declared_downgrade_reason_produces_a_reason_line() {
        let result = sample_result(
            SnippetStatus::Pass,
            Some(DowngradeReason::Declared),
            Some("requested run, validated at declared level typecheck"),
        );

        assert_eq!(
            reason_line(&result),
            Some("  Reason: requested run, validated at declared level typecheck".to_string())
        );
    }

    /// Negative control: an ordinary `Pass` for a legitimately-configured level with nothing
    /// clamped carries no `downgrade_reason` and must produce no reason line at all. ~keep
    #[test]
    fn ordinary_pass_with_no_downgrade_reason_has_no_reason_line() {
        let result = sample_result(SnippetStatus::Pass, None, None);

        assert_eq!(reason_line(&result), None);
    }

    fn unresolved_in(language: Language) -> ValidationResult {
        let mut result = sample_result(SnippetStatus::Unavailable, None, Some("cannot find module"));
        result.snippet.language = language;
        result.unresolved_dependency = true;
        result
    }

    /// The reader's takeaway has to be "two packages were not built", not "446 snippets are
    /// broken" — so the rollup names the languages and says the counts describe the environment.
    /// ~keep
    #[test]
    fn unresolved_dependency_rollup_names_each_language_and_its_count() {
        let summary = RunSummary::from_results(vec![
            unresolved_in(Language::TypeScript),
            unresolved_in(Language::TypeScript),
            unresolved_in(Language::Python),
            sample_result(SnippetStatus::Fail, None, Some("syntax error")),
        ]);

        let line = unresolved_dependency_rollup(&summary).expect("rollup for reclassified results");

        assert!(line.contains("Unresolved dependencies: 3 of 4"), "{line}");
        assert!(line.contains("python 1"), "{line}");
        assert!(line.contains("typescript 2"), "{line}");
        assert!(line.contains("alef build"), "{line}");
    }

    /// Negative control: a clean run must not grow an extra line implying anything was
    /// unavailable. ~keep
    #[test]
    fn unresolved_dependency_rollup_is_absent_when_nothing_was_reclassified() {
        let summary = RunSummary::from_results(vec![sample_result(SnippetStatus::Pass, None, None)]);

        assert_eq!(unresolved_dependency_rollup(&summary), None);
    }

    /// Builds a fixture message the same way `runner::finalize_result` builds a real one for a
    /// session-less reclassification, so this test exercises the exact text the rollup has to
    /// parse rather than a hand-duplicated approximation of it. ~keep
    fn unresolved_with_no_session(language: Language) -> ValidationResult {
        let message = crate::snippets::runner::unresolved_dependency_message(
            true,
            language,
            &language.to_string(),
            ValidationLevel::Compile,
            "cannot find package",
        );
        let mut result = sample_result(SnippetStatus::Unavailable, None, Some(&message));
        result.snippet.language = language;
        result.unresolved_dependency = true;
        result
    }

    /// The regression this pins: a language with no configured session must never be told to run
    /// `alef build` -- that advice is what made a consumer see byte-identical unresolved-dependency
    /// counts before and after building, for languages `alef build` could never fix. ~keep
    #[test]
    fn no_session_configured_results_get_their_own_line_without_the_alef_build_remediation() {
        let summary = RunSummary::from_results(vec![
            unresolved_with_no_session(Language::Go),
            unresolved_with_no_session(Language::Go),
        ]);

        let line = unresolved_dependency_rollup(&summary).expect("rollup for reclassified results");

        assert!(line.contains("No session configured: 2 of 2"), "{line}");
        assert!(line.contains("go 2"), "{line}");
        assert!(
            !line.contains("Run `alef build` before validating"),
            "a no-session result must not repeat the build remediation: {line}"
        );
        assert!(line.contains("configure a session"), "{line}");
    }

    /// Both causes can coexist in one run (some languages have a session and are simply unbuilt,
    /// others have none configured at all) -- each must keep its own line and its own count,
    /// never merged into one total that hides which remediation applies to which language. ~keep
    #[test]
    fn a_run_with_both_causes_reports_two_separate_lines() {
        let summary = RunSummary::from_results(vec![
            unresolved_with_no_session(Language::Go),
            unresolved_in(Language::TypeScript),
        ]);

        let line = unresolved_dependency_rollup(&summary).expect("rollup for reclassified results");

        assert!(line.contains("No session configured: 1 of 2"), "{line}");
        assert!(line.contains("Unresolved dependencies: 1 of 2"), "{line}");
        assert!(line.contains("go 1"), "{line}");
        assert!(line.contains("typescript 1"), "{line}");
    }
}
