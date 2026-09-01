//! Alef defect #142: a snippet session's `before` hook builds this language's artifacts
//! (`cargo build --release -p <crate>-jni`, `pnpm run build:all`, ...) before any of its
//! snippets can validate. When that hook outlives `timeout_secs` -- readily hit on a loaded
//! machine, or right after `alef all --clean` wiped the artifacts it is meant to rebuild -- the
//! failure used to collapse into the same bare `SnippetStatus::Error` as a genuinely broken
//! snippet or a misconfigured session, with a message that was just the raw timeout text. That
//! makes three fundamentally different situations read identically in the report:
//!
//!   (a) the snippet itself is wrong                    -> a real validation failure
//!   (b) the toolchain is missing                       -> a clear skip
//!   (c) the artifact was never built / `--clean` removed it -> an ordering problem
//!
//! These tests pin all three down as distinguishable outcomes, through both runner dispatch
//! paths (`fail_fast_results` and the batched/parallel path via `batch::group_batchable_snippets`)
//! -- the two components that independently classified a session preparation failure before this
//! fix, and the split this defect turned out to be.

use super::*;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;

/// A validator that is never actually reached in these tests -- every scenario here resolves
/// before `validate_one` would call it -- but the registry needs *something* registered for
/// `TypeScript` so a missing-validator branch never masquerades as the behavior under test.
struct UnreachableValidator;

impl SnippetValidator for UnreachableValidator {
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
        panic!("this validator must never be invoked: session preparation should short-circuit first");
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

/// A validator whose toolchain is simply not installed in this environment -- the ordinary
/// `(b)` case, distinct from an unbuilt artifact.
struct MissingToolchainValidator;

impl SnippetValidator for MissingToolchainValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::TypeScript
    }

    fn is_available(&self) -> bool {
        false
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        panic!("an unavailable toolchain must never be invoked");
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

/// A validator whose toolchain runs and genuinely rejects the snippet -- the `(a)` case, which
/// must stay a real failure rather than being pulled into the ordering bucket.
struct GenuinelyBrokenValidator;

impl SnippetValidator for GenuinelyBrokenValidator {
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
        Ok((SnippetStatus::Fail, Some("expected `;`, found end of file".to_string())))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn typescript_snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::TypeScript,
        title: None,
        code: "const value: number = 1;".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

fn timing_out_session(working_directory: &std::path::Path) -> HashMap<String, SessionSpec> {
    HashMap::from([(
        "typescript".to_string(),
        SessionSpec {
            language: crate::snippets::types::Language::TypeScript,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            before: vec![sleep_hook(2)],
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        },
    )])
}

#[cfg(unix)]
fn sleep_hook(seconds: u64) -> String {
    format!("sleep {seconds}")
}

/// `timeout /t` refuses to run at all when stdin is not a console -- `run_command` pipes the
/// child's output and CI hands the test binary a redirected stdin, so it exited immediately
/// with "ERROR: Input redirection is not supported" and the hook never outlived the timeout
/// this fixture exists to trigger. `ping` against loopback is the console-free cmd sleep: one
/// packet goes out immediately and each subsequent one waits a second, so `seconds + 1`
/// packets take about `seconds`, whether or not the pings are answered. ~keep
#[cfg(windows)]
fn sleep_hook(seconds: u64) -> String {
    format!("ping -n {} 127.0.0.1", seconds + 1)
}

/// Case (c), parallel/batched dispatch: `group_batchable_snippets` in `batch.rs` is the path a
/// non-`fail_fast` run takes. Before this fix it stamped `SnippetStatus::Error` directly from the
/// stringified session error, with no way to tell an unbuilt artifact from a broken session.
#[test]
fn session_preparation_timeout_is_an_ordering_problem_not_a_bare_error_on_the_parallel_path() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        timeout_secs: 1,
        sessions: timing_out_session(directory.path()),
        fail_fast: false,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.errors, 0, "an unbuilt artifact must not count as a bare error");
    assert_eq!(summary.failed, 0, "an unbuilt artifact is not a snippet failure");
    assert_eq!(summary.unavailable, 1);
    assert_eq!(summary.unresolved_dependency, 1);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(outcome.unresolved_dependency);
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("Run `alef build` before validating"),
        "message must point at the build ordering, not read as a bare timeout: {message}"
    );
}

/// Case (c), fail-fast dispatch: `validate_one`'s own `session_preparation_error` branch in
/// `runner.rs` is the *other* component that independently classified this failure. Both paths
/// must agree.
#[test]
fn session_preparation_timeout_is_an_ordering_problem_not_a_bare_error_on_the_fail_fast_path() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        timeout_secs: 1,
        sessions: timing_out_session(directory.path()),
        fail_fast: true,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.errors, 0, "an unbuilt artifact must not count as a bare error");
    assert_eq!(summary.unavailable, 1);
    assert_eq!(summary.unresolved_dependency, 1);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(outcome.unresolved_dependency);
    assert!(
        outcome
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("Run `alef build` before validating")
    );
}

/// Case (b): a genuinely missing toolchain is `Unavailable` too, but must never be mistaken for
/// case (c) -- there is no unbuilt artifact here, just no compiler on `PATH`.
#[test]
fn missing_toolchain_is_unavailable_but_not_flagged_as_unresolved_dependency() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(MissingToolchainValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.unavailable, 1);
    assert_eq!(
        summary.unresolved_dependency, 0,
        "a missing toolchain is not the same problem as an unbuilt artifact"
    );
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(!outcome.unresolved_dependency);
}

/// Case (a): a validator that actually ran and rejected the snippet on its own merits must stay
/// a real failure, not be pulled into the ordering/unresolved-dependency bucket.
#[test]
fn a_genuinely_broken_snippet_stays_a_real_failure() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(GenuinelyBrokenValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.failed, 1);
    assert_eq!(summary.unresolved_dependency, 0);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Fail);
    assert!(!outcome.unresolved_dependency);
}

/// task #130: a `tsc` toolchain that ran to completion and reported a genuine type error
/// (TS2322 "not assignable") must come back through `finalize_result` as `Fail` with the
/// compiler's own message, never `Unavailable` captioned "toolchain ran but reported a missing
/// dependency or build artifact -- run `alef build` first". That caption sent the reader to
/// rebuild toolchains for a defect no rebuild could fix, and `Unavailable` is an incomplete
/// status that fails a `--strict` run for a reason it had misnamed. This drives the real
/// `TypeScriptValidator::is_dependency_error`, not a stub, through `finalize_result` directly. ~keep
#[test]
fn finalize_result_keeps_a_real_type_error_as_fail_with_the_compiler_message() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let diagnostic = "snippet.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.";
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some(diagnostic.to_string()),
        duration_ms: 5,
        timed_out: false,
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        None,
        ValidationLevel::Compile,
        outcome,
    );

    assert_eq!(result.status, SnippetStatus::Fail, "got: {result:?}");
    assert!(
        !result.unresolved_dependency,
        "a real type error must not be flagged as a dependency gap"
    );
    assert_eq!(
        result.message.as_deref(),
        Some(diagnostic),
        "a real type error's message must stay the compiler's own text verbatim, not be recaptioned \
         as a missing dependency"
    );
    assert!(
        !result.message.as_deref().unwrap_or_default().contains("alef build"),
        "a real type error must never tell the reader to rebuild toolchains: {result:?}"
    );
}

/// The complementary case: a genuinely unresolved module (`tsc` could not locate it at all) must
/// still classify as `unresolved_dependency` -- proving the narrowed pattern set didn't
/// overcorrect into treating every `tsc` failure as a snippet defect.
#[test]
fn finalize_result_still_flags_a_real_missing_module_as_unresolved_dependency() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some("snippet.ts(1,1): error TS2307: Cannot find module 'widgets'.".to_string()),
        duration_ms: 5,
        timed_out: false,
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        None,
        ValidationLevel::Compile,
        outcome,
    );

    assert_eq!(result.status, SnippetStatus::Unavailable, "got: {result:?}");
    assert!(
        result.unresolved_dependency,
        "a genuinely unresolved module must still be flagged: {result:?}"
    );
    assert!(
        result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("Cannot find module 'widgets'"),
        "the original diagnostic must still be included, not replaced: {result:?}"
    );
}

/// The regression this pins: `finalize_result`'s own reclassification, not just the message
/// builder it calls, must actually route a session-less snippet to the no-session wording. A
/// consumer with no `[workspace.docs.snippets.sessions.<target>]` entry for a language sees
/// exactly this shape -- `session: None` reaching `finalize_result` -- and running `alef build`
/// cannot fix it, so the message must not tell them to. ~keep
#[test]
fn finalize_result_with_no_session_names_the_no_session_cause_not_alef_build() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some("snippet.ts(1,1): error TS2307: Cannot find module 'widgets'.".to_string()),
        duration_ms: 5,
        timed_out: false,
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        None,
        ValidationLevel::Compile,
        outcome,
    );

    let message = result.message.as_deref().unwrap_or_default();
    assert!(
        message.contains(super::dependency_reclassification::NO_SESSION_CONFIGURED_PHRASE),
        "a session-less reclassification must name the real cause: {message}"
    );
    assert!(
        !message.contains("run `alef build` first"),
        "a session-less reclassification must not send the reader to rebuild artifacts a \
         session-less run could never see: {message}"
    );
}

/// The complementary case: with a real, prepared session, the reclassification keeps the ordering
/// wording -- the artifact genuinely might just not be built yet, and `alef build` can fix that.
#[test]
fn finalize_result_with_a_configured_session_keeps_the_ordering_message() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some("snippet.ts(1,1): error TS2307: Cannot find module 'widgets'.".to_string()),
        duration_ms: 5,
        timed_out: false,
    };
    let session = crate::snippets::session::ValidationSession {
        language: crate::snippets::types::Language::TypeScript,
        working_directory: std::path::PathBuf::from("."),
        manifest: None,
        fingerprint: "fixture".into(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        Some(&session),
        ValidationLevel::Compile,
        outcome,
    );

    let message = result.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("run `alef build` first"),
        "a session-backed reclassification must still point at the real remedy: {message}"
    );
    assert!(
        !message.contains(super::dependency_reclassification::NO_SESSION_CONFIGURED_PHRASE),
        "a configured session must never be reported as missing: {message}"
    );
}

/// Alef defect #127: two configured sessions target the same language (a consumer's real
/// `[docs.snippets.sessions.typescript]` + `[docs.snippets.sessions.wasm]`, both TypeScript) and
/// a hand-written snippet carries no explicit `target:` to break the tie. Before this fix, the
/// fallback was a literal `sessions.get("typescript")` lookup: whichever session happened to be
/// spelled like the bare language silently claimed every such snippet, validated it against that
/// session's toolchain, and reported a normal `Pass`/`Fail` -- with no signal anywhere that the
/// claim was an accident of naming, or that the sibling `wasm` session got no hand-written
/// coverage at all. `UnreachableValidator` proves the ambiguity is caught before any validator
/// ever runs, on both dispatch paths.
fn two_same_language_sessions(node: &std::path::Path, wasm: &std::path::Path) -> HashMap<String, SessionSpec> {
    let spec = |working_directory: &std::path::Path| SessionSpec {
        language: crate::snippets::types::Language::TypeScript,
        working_directory: working_directory.to_path_buf(),
        manifest: None,
        before: Vec::new(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    };
    HashMap::from([("typescript".to_string(), spec(node)), ("wasm".to_string(), spec(wasm))])
}

#[test]
fn an_ambiguous_session_claim_is_a_real_error_on_the_fail_fast_path() {
    let node = tempfile::tempdir().expect("node session directory");
    let wasm = tempfile::tempdir().expect("wasm session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: two_same_language_sessions(node.path(), wasm.path()),
        fail_fast: true,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(
        summary.failed, 0,
        "an ambiguous claim is a configuration gap, not a snippet defect"
    );
    assert_eq!(summary.errors, 1);
    assert!(
        summary.has_failures(),
        "an ambiguous claim must fail every run, not just --strict"
    );
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error);
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("typescript"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("wasm"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("target:"),
        "message must tell the reader how to resolve it: {message}"
    );
}

#[test]
fn an_ambiguous_session_claim_is_a_real_error_on_the_parallel_path() {
    let node = tempfile::tempdir().expect("node session directory");
    let wasm = tempfile::tempdir().expect("wasm session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: two_same_language_sessions(node.path(), wasm.path()),
        fail_fast: false,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.errors, 1);
    assert!(summary.has_failures());
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error, "got: {outcome:?}");
    // The message must name the ambiguity itself, not just any `SnippetStatus::Error` --
    // `UnreachableValidator` never overrides `validate_in_session`, so if resolution ever slipped
    // back to picking a session by naming coincidence (the pre-fix bug), the *default*
    // `validate_in_session` would reject that session too and land on the same bare `Error`
    // status for a completely different reason. Only the message tells the two apart.
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("typescript"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("wasm"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("target:"),
        "message must tell the reader how to resolve it: {message}"
    );
}

/// A single configured session still claims a target-less snippet no matter what it is named --
/// the other half of #127. Three of the four real consumer configs surveyed name their TypeScript
/// sessions `node`/`wasm`, never `typescript`; before this fix the bare-language fallback missed
/// every one of them and every hand-written snippet validated with no session at all.
///
/// `GenuinelyBrokenValidator` never overrides `validate_in_session`, so `SnippetValidator`'s
/// default implementation is what actually runs: it rejects outright when handed `Some(session)`
/// ("does not support binding-aware sessions") and only calls through to `validate` when handed
/// `None`. That default is a precise discriminator here -- before this fix the `node`-named
/// session never resolved for a target-less snippet, so validation fell through to `None` and
/// `GenuinelyBrokenValidator::validate`'s own `Fail`. After the fix, `session_for` resolves the
/// single same-language candidate regardless of its name, so the snippet reaches the validator
/// *with* a session and the default rejection fires instead. ~keep
#[test]
fn a_single_differently_named_session_still_claims_a_target_less_snippet() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(GenuinelyBrokenValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: HashMap::from([(
            "node".to_string(),
            SessionSpec {
                language: crate::snippets::types::Language::TypeScript,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: Vec::new(),
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        )]),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error, "got: {outcome:?}");
    assert!(
        outcome
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("binding-aware sessions"),
        "the snippet must have reached the validator carrying the `node` session, not `None`: {outcome:?}"
    );
}

/// A `before` hook builds a whole package while `timeout_secs` bounds a single snippet's compiler
/// invocation. While the two shared one number, giving a cold Gradle or pnpm build the minutes it
/// needs meant handing every snippet compile the same minutes — which is how a runaway hook came
/// to have half an hour to run out. This pins the hook to its own budget.
///
/// `timeout_secs` here is 600: if the hook were still bounded by it, `sleep 2` would finish, the
/// session would prepare successfully, and `UnreachableValidator` would panic instead of this
/// assertion failing. The test cannot pass without the hook budget being the one in effect. ~keep
#[test]
fn a_before_hook_is_bounded_by_its_own_budget_when_one_is_configured() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        timeout_secs: 600,
        before_timeout_secs: Some(1),
        sessions: timing_out_session(directory.path()),
        fail_fast: false,
        ..RunnerConfig::default()
    };
    let started = std::time::Instant::now();

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.unresolved_dependency, 1);
    assert_eq!(summary.results[0].status, SnippetStatus::Unavailable);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the hook must be cut off at its own 1s budget, not at the 600s snippet budget"
    );
}

/// The default must not change what a consumer already configured: with no hook budget set, the
/// hook is bounded by `timeout_secs` exactly as before.
#[test]
fn an_unset_hook_budget_falls_back_to_the_snippet_timeout() {
    let shared = RunnerConfig {
        timeout_secs: 42,
        ..RunnerConfig::default()
    };
    let separate = RunnerConfig {
        timeout_secs: 42,
        before_timeout_secs: Some(900),
        ..RunnerConfig::default()
    };

    assert_eq!(shared.resolved_before_timeout_secs(), 42);
    assert_eq!(separate.resolved_before_timeout_secs(), 900);
}

fn go_snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::Go,
        title: None,
        code: "package main\n\nfunc main() {}\n".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

fn go_session_fixture() -> crate::snippets::session::ValidationSession {
    crate::snippets::session::ValidationSession {
        language: crate::snippets::types::Language::Go,
        working_directory: std::path::PathBuf::from("."),
        manifest: None,
        fingerprint: "fixture".into(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    }
}

/// Task #505: a consumer's CI run reported 141 Go snippets as genuine `ld: cannot find -l<name>`
/// failures, when the real cause in every one was that `alef build` had simply not run yet. This
/// drives the real `GoValidator`/`finalize_result` bucketing pipeline -- not `is_dependency_error`
/// in isolation -- over every linker phrasing for "the named library does not exist" that this
/// module now recognizes: GNU ld and Apple ld (both confirmed against the original consumer
/// report) and LLVM `lld` (confirmed absent from the classifier until this task, reachable via
/// `-fuse-ld=lld`). A session is threaded through so the message asserts against the real
/// "run `alef build` first" ordering wording, matching the shape the original bug actually hit
/// (a session-backed run whose artifact just was not built yet). ~keep
#[test]
fn finalize_result_buckets_every_captured_go_linker_missing_library_form_as_unavailable() {
    let cases: &[(&str, &str)] = &[
        (
            "GNU ld (Linux gcc/clang default)",
            "# example.test/module\n/usr/bin/ld: cannot find -lsample_ffi: No such file or directory\n\
             collect2: error: ld returned 1 exit status\n",
        ),
        (
            "Apple ld (macOS clang default)",
            "# example.test/module\nld: library not found for -lsample_ffi\n\
             clang: error: linker command failed with exit code 1 (use -v to see invocation)\n",
        ),
        (
            "LLVM lld (-fuse-ld=lld)",
            "# example.test/module\nld.lld: error: unable to find library -lsample_ffi\n\
             clang: error: linker command failed with exit code 1 (use -v to see invocation)\n",
        ),
    ];
    let validator = crate::snippets::validators::go::GoValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let session = go_session_fixture();

    for (platform, raw_output) in cases {
        let outcome = ValidationOutcome {
            status: SnippetStatus::Fail,
            message: Some((*raw_output).to_string()),
            duration_ms: 5,
            timed_out: false,
        };

        let result = finalize_result(
            &go_snippet(),
            &validator,
            &config,
            Some(&session),
            ValidationLevel::Compile,
            outcome,
        );

        assert_eq!(
            result.status,
            SnippetStatus::Unavailable,
            "{platform}: a missing linked library must bucket as unavailable, not a real failure: {result:?}"
        );
        assert!(
            result.unresolved_dependency,
            "{platform}: must be flagged as an unresolved dependency: {result:?}"
        );
        let message = result.message.as_deref().unwrap_or_default();
        assert!(
            message.contains("run `alef build` first"),
            "{platform}: a session-backed reclassification must point at the real remedy: {message}"
        );
        assert!(
            message.contains("sample_ffi"),
            "{platform}: the original linker diagnostic must survive in the message: {message}"
        );
    }
}

/// Negative control for the table above: a genuine compile error in the snippet's own code must
/// still bucket as `Fail`, never be swept into `Unavailable` by a pattern widened too far to tell
/// "not built yet" from "does not compile". Without this test, the fix above is indistinguishable
/// from "call every Go failure unavailable" -- exactly the regression the design constraint on
/// this task warns against. ~keep
#[test]
fn finalize_result_keeps_a_real_go_compile_error_as_fail_not_unavailable() {
    let validator = crate::snippets::validators::go::GoValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let diagnostic = "./snippet.go:3:10: cannot use \"text\" (untyped string constant) as int value in assignment";
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some(diagnostic.to_string()),
        duration_ms: 5,
        timed_out: false,
    };

    let result = finalize_result(
        &go_snippet(),
        &validator,
        &config,
        Some(&go_session_fixture()),
        ValidationLevel::Compile,
        outcome,
    );

    assert_eq!(result.status, SnippetStatus::Fail, "got: {result:?}");
    assert!(
        !result.unresolved_dependency,
        "a real compile error must not be flagged as a dependency gap"
    );
    assert_eq!(
        result.message.as_deref(),
        Some(diagnostic),
        "a real compile error's message must stay the compiler's own text verbatim: {result:?}"
    );
    assert!(
        !result.message.as_deref().unwrap_or_default().contains("alef build"),
        "a real compile error must never tell the reader to rebuild toolchains: {result:?}"
    );
}
