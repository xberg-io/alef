//! `FailureReporter` and `finalize_result` tests: a language failing at scale must surface its
//! first failure/unavailability immediately, log the rest without a firehose, and never lose the
//! `unresolved_dependency` reclassification on the returned `ValidationResult`.

use super::*;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use tracing_test::traced_test;

fn network_snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::Rust,
        title: None,
        code: "fn main() {}".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata {
            side_effect: Some(SideEffectClass::Network),
            ..SnippetMetadata::default()
        },
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

/// Always reports the same failure, standing in for a language failing at 100% — the shape of
/// the run that produced 1,753 failures with no log output until the stage ended.
struct FailingValidator;

impl SnippetValidator for FailingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::Java
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Ok((
            SnippetStatus::Fail,
            Some("Example.java:1: error: duplicate class: Example\n  1 error".into()),
        ))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn failing_snippet(language: crate::snippets::types::Language) -> Snippet {
    let mut snippet = network_snippet();
    snippet.language = language;
    snippet
}

fn failure_result(snippet: &Snippet, message: &str) -> ValidationResult {
    result(
        snippet,
        SnippetStatus::Fail,
        ValidationLevel::Compile,
        ValidationLevel::Compile,
        Some(message.to_string()),
        1,
    )
}

/// The defect: 1,753 snippet failures went straight to the result cache and the final summary,
/// so six languages failing at 100% were indistinguishable from a healthy run for the entire
/// stage. What makes "before the stage ends" checkable here is the *absence* of the terminal
/// per-language event: two of this language's three snippets are still outstanding, so the
/// summary cannot have run, yet the first failure and its validator message are already out. ~keep
#[traced_test]
#[test]
fn a_languages_first_failure_is_reported_before_its_stage_ends() {
    let snippets = vec![
        failing_snippet(crate::snippets::types::Language::Java),
        failing_snippet(crate::snippets::types::Language::Java),
        failing_snippet(crate::snippets::types::Language::Java),
    ];
    let reporter = FailureReporter::new(&snippets);

    reporter.record(&failure_result(
        &snippets[0],
        "error: cannot find symbol\n  symbol: class Missing",
    ));

    assert!(logs_contain("First snippet validation failure for this language"));
    assert!(logs_contain("cannot find symbol | symbol: class Missing"));
    assert!(!logs_contain("Finished snippet validation for this language"));
}

fn unavailable_result(snippet: &Snippet, message: &str) -> ValidationResult {
    let mut value = result(
        snippet,
        SnippetStatus::Unavailable,
        ValidationLevel::Compile,
        ValidationLevel::Compile,
        Some(message.to_string()),
        1,
    );
    value.unresolved_dependency = true;
    value
}

/// `Unavailable` is not the harmless outcome its name suggests: under `strict` it fails the run
/// exactly like a `Fail`, and the `unresolved_dependency` reclassification turns a real
/// validator failure -- diagnostic and all -- into one. Tallying only `Fail | Error` is how 566
/// snippets across two languages reached the final summary as "283 unresolved dependency"
/// apiece with not one line anywhere saying WHICH dependency, while the validator's own message
/// sat unread on every result. ~keep
#[traced_test]
#[test]
fn a_languages_first_unavailable_result_is_reported_with_the_validator_message() {
    let snippets = vec![
        failing_snippet(crate::snippets::types::Language::Csharp),
        failing_snippet(crate::snippets::types::Language::Csharp),
    ];
    let reporter = FailureReporter::new(&snippets);

    reporter.record(&unavailable_result(
        &snippets[0],
        "error NU1101: Unable to find package Contoso.Sample",
    ));

    assert!(logs_contain(
        "First snippet validation unavailability for this language"
    ));
    assert!(logs_contain("Unable to find package Contoso.Sample"));
}

/// A language whose every snippet came back unvalidated must say so at the end of its stage. It
/// used to fall through to the `debug!` "Finished" arm reserved for a clean run, so a total
/// blackout logged exactly like a total pass. ~keep
#[traced_test]
#[test]
fn a_language_with_no_validated_result_at_all_is_not_reported_as_finished_clean() {
    let snippets = vec![
        failing_snippet(crate::snippets::types::Language::Csharp),
        failing_snippet(crate::snippets::types::Language::Csharp),
    ];
    let reporter = FailureReporter::new(&snippets);

    for snippet in &snippets {
        reporter.record(&unavailable_result(snippet, "error NU1101: Unable to find package"));
    }

    assert!(logs_contain(
        "Finished snippet validation for this language with every result unvalidated"
    ));
}

/// The other half of the requirement: visible, but not a firehose. Failure two emits nothing
/// at all, and a running count only appears once the stride is reached — so a 1,753-failure
/// run costs tens of lines, not 1,753. ~keep
#[traced_test]
#[test]
fn failures_after_the_first_are_counted_rather_than_logged_one_line_each() {
    let snippets = vec![failing_snippet(crate::snippets::types::Language::Java); FAILURE_PROGRESS_STRIDE + 1];
    let reporter = FailureReporter::new(&snippets);

    for snippet in snippets.iter().take(2) {
        reporter.record(&failure_result(snippet, "compilation failed"));
    }
    assert!(logs_contain("First snippet validation failure for this language"));
    assert!(!logs_contain("Snippet validation failures accumulating"));

    for snippet in snippets.iter().take(FAILURE_PROGRESS_STRIDE).skip(2) {
        reporter.record(&failure_result(snippet, "compilation failed"));
    }
    assert!(logs_contain("Snippet validation failures accumulating"));
    assert!(!logs_contain("Finished snippet validation for this language"));
}

/// The reporter must be wired into the real per-snippet dispatch path, not just constructible:
/// `parallel_results` is where all 1,753 failures were produced and dropped on the floor.
#[traced_test]
#[test]
fn a_failing_language_is_reported_through_the_real_validation_run() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(FailingValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        parallelism: 1,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let snippets = vec![
        failing_snippet(crate::snippets::types::Language::Java),
        failing_snippet(crate::snippets::types::Language::Java),
    ];

    let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

    assert_eq!(summary.failed, 2);
    assert!(logs_contain("First snippet validation failure for this language"));
    assert!(logs_contain("duplicate class: Example"));
    assert!(logs_contain(
        "Finished snippet validation for this language with failures"
    ));
}

#[test]
fn a_failure_preview_is_a_single_bounded_line() {
    let long = "x".repeat(FAILURE_MESSAGE_PREVIEW_CHARS + 50);
    let preview = failure_preview(Some(long.as_str()));
    assert_eq!(preview.len(), FAILURE_MESSAGE_PREVIEW_CHARS + 3);
    assert!(preview.ends_with("..."));
    assert_eq!(failure_preview(Some("  \n\n ")), "<no validator output>");
    assert_eq!(failure_preview(None), "<no validator output>");
    assert_eq!(failure_preview(Some("first\n\nsecond")), "first | second");
}

struct DependencyFailingValidator;

impl SnippetValidator for DependencyFailingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::TypeScript
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        unreachable!("this test drives finalize_result directly, not through validate")
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, error_output: &str) -> bool {
        error_output.contains("Cannot find module")
    }
}

/// The bug this guards: `finalize_result` computes `unresolved_dependency` as a local, uses it
/// to reclassify `status` and build `message`, then only ever copies `capability_capped` and
/// `downgrade_reason` onto the `ValidationResult` it returns — never `unresolved_dependency`
/// itself, so the field stayed `false` on every result the real producer ever built. The
/// pre-existing guard in `types.rs` (`unresolved_dependency_is_a_reconcilable_subset_of_
/// unavailable`) hand-builds a `ValidationResult` and passes the flag in as a parameter, so it
/// only ever exercises `RunSummary::from_results` — never `finalize_result` — and stayed green
/// through the whole regression. This test drives the real producer instead: a `Fail` outcome
/// whose message the validator's own `is_dependency_error` recognizes, at a level above
/// `Syntax`, must come back `Unavailable` with `unresolved_dependency` set on the
/// `ValidationResult` `finalize_result` actually returns. ~keep
#[test]
fn finalize_result_sets_unresolved_dependency_on_the_returned_result() {
    let snippet = failing_snippet(crate::snippets::types::Language::TypeScript);
    let validator = DependencyFailingValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some("error TS2307: Cannot find module 'widgets'".to_string()),
        duration_ms: 5,
        timed_out: false,
    };

    let result = finalize_result(&snippet, &validator, &config, None, ValidationLevel::Compile, outcome);

    assert_eq!(result.status, SnippetStatus::Unavailable, "got: {result:?}");
    assert!(
        result.unresolved_dependency,
        "finalize_result must set unresolved_dependency on the ValidationResult it returns, not just use it \
         locally to reclassify status and message"
    );
}

#[traced_test]
#[test]
fn mixed_failure_and_unavailability_finish_with_failures_without_inventing_toolchain_work() {
    use crate::snippets::types::Language;

    let snippets = vec![failing_snippet(Language::Java); 2];
    let reporter = FailureReporter::new(&snippets);
    reporter.record_toolchain_start(Language::Java, 10);
    reporter.record(&failure_result(&snippets[0], "compilation failed"));
    assert!(!logs_contain("Finished snippet validation for this language"));

    reporter.record(&unavailable_result(&snippets[1], "missing dependency"));

    assert_eq!(reporter.invoked_by_language(), [(Language::Java, 1)].into());
    assert!(logs_contain("First snippet validation failure for this language"));
    assert!(logs_contain(
        "First snippet validation unavailability for this language"
    ));
    assert!(logs_contain(
        "Finished snippet validation for this language with failures"
    ));
    assert!(!logs_contain("with every result unvalidated"));
}
