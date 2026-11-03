//! Batching, session dispatch, and per-snippet timeout budget tests for `run_validation`.

use super::*;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use std::sync::Arc;

struct RecordingValidator {
    language: crate::snippets::types::Language,
    batches: Arc<Mutex<Vec<(crate::snippets::types::Language, usize, bool)>>>,
    singles: Arc<Mutex<usize>>,
}

/// Overruns its timeout on the first call only, then returns immediately. A shared group
/// deadline is fully consumed by that first call, so any snippet the runner still executes
/// afterwards proves the budget is per-snippet. ~keep
#[cfg(unix)]
struct ExhaustingValidator {
    timeouts: Arc<Mutex<Vec<u64>>>,
}

#[cfg(unix)]
impl SnippetValidator for ExhaustingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::Bash
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let call = {
            let mut timeouts = self.timeouts.lock().expect("timeouts");
            timeouts.push(timeout_secs);
            timeouts.len()
        };
        if call == 1 {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "sleep 30 & wait"]);
            crate::snippets::validators::run_command(&mut command, timeout_secs)?;
        }
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

/// Records the timeout handed to each entry point, and optionally opts in to batching, so a
/// test can assert which budget a validator actually receives without consuming wall clock.
struct BudgetRecordingValidator {
    singles: Arc<Mutex<Vec<u64>>>,
    batched: Arc<Mutex<Vec<(usize, u64)>>>,
    supports_batching: bool,
}

impl SnippetValidator for BudgetRecordingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::Rust
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        self.singles.lock().expect("single timeouts").push(timeout_secs);
        Ok((SnippetStatus::Pass, None))
    }

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        _level: ValidationLevel,
        timeout_secs: u64,
        _session: Option<&crate::snippets::session::ValidationSession>,
    ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
        if !self.supports_batching {
            return None;
        }
        self.batched
            .lock()
            .expect("batch timeouts")
            .push((snippets.len(), timeout_secs));
        Some(Ok(vec![(SnippetStatus::Pass, None); snippets.len()]))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn supports_batching(&self) -> bool {
        self.supports_batching
    }
}

impl SnippetValidator for RecordingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        self.language
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
        *self.singles.lock().expect("single count") += 1;
        Ok((SnippetStatus::Pass, None))
    }

    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        _level: ValidationLevel,
        _timeout_secs: u64,
        session: Option<&crate::snippets::session::ValidationSession>,
    ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
        self.batches
            .lock()
            .expect("batch records")
            .push((self.language, snippets.len(), session.is_some()));
        Some(Ok(vec![(SnippetStatus::Pass, None); snippets.len()]))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn supports_batching(&self) -> bool {
        true
    }
}

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

#[test]
fn side_effect_policy_only_blocks_execution() {
    let snippet = network_snippet();
    let compile = RunnerConfig {
        level: ValidationLevel::Compile,
        ..RunnerConfig::default()
    };
    let run = RunnerConfig {
        level: ValidationLevel::Run,
        ..RunnerConfig::default()
    };

    assert_eq!(side_effect_rejection(&snippet, &compile), None);
    assert_eq!(
        side_effect_rejection(&snippet, &run).as_deref(),
        Some("side effect class network is not allowed")
    );
}

#[test]
fn nullable_tags_do_not_bypass_network_execution_policy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("network.md"),
        "---\nside_effect: network\ntags: null\n---\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    let snippets = crate::snippets::discovery::discover_snippets(&[dir.path().to_path_buf()], None).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].metadata.tags, Vec::<String>::new());
    let config = RunnerConfig {
        level: ValidationLevel::Run,
        ..RunnerConfig::default()
    };
    assert_eq!(
        side_effect_rejection(&snippets[0], &config).as_deref(),
        Some("side effect class network is not allowed")
    );
}

#[test]
fn nullable_requires_preserve_explicit_skip_annotation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("skip.md"),
        "---\nskip: true\nreason: needs service\nrequires:\n---\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    let snippets = crate::snippets::discovery::discover_snippets(&[dir.path().to_path_buf()], None).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].metadata.requires, Vec::<String>::new());
    assert_eq!(
        snippets[0].annotation,
        Some(crate::snippets::types::SnippetAnnotation {
            kind: SnippetAnnotationKind::Skip,
            reason: Some("needs service".into()),
        })
    );
}

fn unclassified_snippet() -> Snippet {
    Snippet {
        metadata: SnippetMetadata {
            side_effect: None,
            ..SnippetMetadata::default()
        },
        ..network_snippet()
    }
}

/// `deny_unclassified` is the gate that actually decides whether a snippet with no side-effect
/// classification reaches real execution at `ValidationLevel::Run` or is pre-emptively skipped.
/// `cli::commands::snippets::resolved_deny_unclassified` decides the boolean fed into this
/// field from `--strict` and `[crates.docs.snippets].strict`/`deny_unclassified`; this proves
/// the field itself has the effect that decision assumes, at both settings. ~keep
#[test]
fn deny_unclassified_rejects_only_when_enabled() {
    let snippet = unclassified_snippet();
    let permissive = RunnerConfig {
        level: ValidationLevel::Run,
        deny_unclassified: false,
        ..RunnerConfig::default()
    };
    let strict = RunnerConfig {
        level: ValidationLevel::Run,
        deny_unclassified: true,
        ..RunnerConfig::default()
    };

    assert_eq!(
        side_effect_rejection(&snippet, &permissive),
        None,
        "an unclassified snippet must be allowed through when deny_unclassified is off"
    );
    assert_eq!(
        side_effect_rejection(&snippet, &strict).as_deref(),
        Some("unclassified side effects are denied")
    );
}

#[test]
fn annotations_cap_validation_instead_of_skipping_it() {
    let mut snippet = network_snippet();
    snippet.annotation = Some(crate::snippets::types::SnippetAnnotation {
        kind: SnippetAnnotationKind::SyntaxOnly,
        reason: None,
    });

    assert_eq!(
        effective_validation_level(&snippet, ValidationLevel::TypeCheck),
        ValidationLevel::Syntax
    );

    snippet.annotation = None;
    assert_eq!(
        effective_validation_level(&snippet, ValidationLevel::TypeCheck),
        ValidationLevel::TypeCheck
    );
}

/// `session_for` is a thin wrapper over `session_resolution::resolve_session_claim`
/// (exhaustively tested there); this pins the wrapper itself to the two outcomes a real
/// dispatch path actually observes. An explicit `target:` wins outright even when its
/// language has another same-language session configured. Once that explicit target is gone,
/// two sessions for one language (a real consumer's `[sessions.typescript]` +
/// `[sessions.wasm]`, both TypeScript -- see alef defect #127) must resolve to no session at
/// all, not to whichever one happens to be spelled like the bare language. ~keep
#[test]
fn target_session_precedes_an_ambiguous_language_fallback() {
    let mut snippet = network_snippet();
    snippet.language = crate::snippets::types::Language::TypeScript;
    snippet.metadata.target = Some("wasm".into());
    let sessions = HashMap::from([
        (
            "typescript".into(),
            crate::snippets::session::ValidationSession {
                language: crate::snippets::types::Language::TypeScript,
                working_directory: "bindings/node".into(),
                manifest: None,
                fingerprint: "node".into(),
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        ),
        (
            "wasm".into(),
            crate::snippets::session::ValidationSession {
                language: crate::snippets::types::Language::TypeScript,
                working_directory: "bindings/wasm".into(),
                manifest: None,
                fingerprint: "wasm".into(),
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        ),
    ]);

    assert_eq!(
        session_for(&snippet, &sessions).map(|session| session.fingerprint.as_str()),
        Some("wasm")
    );
    snippet.metadata.target = None;
    assert_eq!(
        session_for(&snippet, &sessions).map(|session| session.fingerprint.as_str()),
        None
    );
}

#[test]
fn groups_batches_by_language_session_and_preserves_order() {
    let batches = Arc::new(Mutex::new(Vec::new()));
    let singles = Arc::new(Mutex::new(0));
    let mut registry = ValidatorRegistry::new();
    for language in [
        crate::snippets::types::Language::Rust,
        crate::snippets::types::Language::Python,
    ] {
        registry.register(Box::new(RecordingValidator {
            language,
            batches: Arc::clone(&batches),
            singles: Arc::clone(&singles),
        }));
    }
    let first_directory = tempfile::tempdir().expect("first session");
    let second_directory = tempfile::tempdir().expect("second session");
    let session = |directory: &std::path::Path| SessionSpec {
        language: crate::snippets::types::Language::Rust,
        working_directory: directory.into(),
        manifest: None,
        before: Vec::new(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    };
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: HashMap::from([
            ("alpha".into(), session(first_directory.path())),
            ("beta".into(), session(second_directory.path())),
        ]),
        ..RunnerConfig::default()
    };
    let mut snippets = vec![network_snippet(), network_snippet(), network_snippet()];
    snippets[0].id = Some("first".into());
    snippets[0].metadata.target = Some("alpha".into());
    snippets[1].id = Some("second".into());
    snippets[1].metadata.target = Some("beta".into());
    snippets[2].id = Some("third".into());
    snippets[2].language = crate::snippets::types::Language::Python;

    let summary = run_validation(&snippets, &registry, &config).expect("validation succeeds");

    assert_eq!(
        summary
            .results
            .iter()
            .map(|value| value.snippet.id.as_deref())
            .collect::<Vec<_>>(),
        [Some("first"), Some("second"), Some("third")]
    );
    assert_eq!(*singles.lock().expect("single count"), 0);
    let batches = batches.lock().expect("batch records");
    assert_eq!(batches.len(), 3);
    assert!(batches.iter().all(|(_, size, _)| *size == 1));
}

#[test]
fn session_preparation_errors_do_not_abort_healthy_targets() {
    let batches = Arc::new(Mutex::new(Vec::new()));
    let singles = Arc::new(Mutex::new(0));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(RecordingValidator {
        language: crate::snippets::types::Language::Rust,
        batches: Arc::clone(&batches),
        singles,
    }));
    let directory = tempfile::tempdir().expect("session directory");
    let session = |manifest| SessionSpec {
        language: crate::snippets::types::Language::Rust,
        working_directory: directory.path().into(),
        manifest,
        before: Vec::new(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    };
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: HashMap::from([
            ("broken".into(), session(Some(directory.path().join("missing.toml")))),
            ("healthy".into(), session(None)),
        ]),
        ..RunnerConfig::default()
    };
    let mut snippets = vec![network_snippet(), network_snippet()];
    snippets[0].metadata.target = Some("broken".into());
    snippets[1].metadata.target = Some("healthy".into());

    let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 2);
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.passed, 1);
    assert!(summary.has_failures());
    assert_eq!(summary.results[0].status, SnippetStatus::Error);
    assert!(
        summary.results[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("target `broken`") && message.contains("manifest does not exist"))
    );
    assert_eq!(summary.results[1].status, SnippetStatus::Pass);
    assert_eq!(
        batches.lock().expect("batch records").as_slice(),
        &[(crate::snippets::types::Language::Rust, 1, true)]
    );
}

#[test]
fn cached_cells_are_excluded_from_batches() {
    let batches = Arc::new(Mutex::new(Vec::new()));
    let singles = Arc::new(Mutex::new(0));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(RecordingValidator {
        language: crate::snippets::types::Language::Rust,
        batches: Arc::clone(&batches),
        singles,
    }));
    let cache_directory = tempfile::tempdir().expect("cache directory");
    let mut snippets = vec![network_snippet(), network_snippet()];
    snippets[1].code = "fn main() { let _value = 2; }".into();
    let cached = result(
        &snippets[0],
        SnippetStatus::Pass,
        ValidationLevel::Compile,
        ValidationLevel::Compile,
        None,
        1,
    );
    ValidationCache::new(cache_directory.path().into())
        .store(&snippets[0], ValidationLevel::Compile, None, false, &[], &cached)
        .expect("cache entry");
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        changed_only: true,
        cache_dir: Some(cache_directory.path().into()),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&snippets, &registry, &config).expect("validation succeeds");

    assert_eq!(summary.results.len(), 2);
    assert_eq!(summary.results[0].duration_ms, 0);
    assert_eq!(
        batches.lock().expect("batch records").as_slice(),
        &[(crate::snippets::types::Language::Rust, 1, false)]
    );
}

/// One snippet exhausting its timeout must not consume the budget of the next one. The
/// assertions are on call count and status, never on elapsed wall clock, and the overrun is
/// 30x the budget so no plausible scheduling delay can flip the outcome. ~keep
#[cfg(unix)]
#[test]
fn a_snippet_that_times_out_does_not_consume_the_next_snippets_budget() {
    let timeouts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(ExhaustingValidator {
        timeouts: Arc::clone(&timeouts),
    }));
    let mut snippets = vec![network_snippet(), network_snippet()];
    for snippet in &mut snippets {
        snippet.language = crate::snippets::types::Language::Bash;
    }
    let config = RunnerConfig {
        level: ValidationLevel::Run,
        parallelism: 1,
        timeout_secs: 1,
        cache_dir: None,
        // network_snippet() carries SideEffectClass::Network; side_effect_rejection()
        // skips unlisted side effects at ValidationLevel::Run before the validator
        // ever runs (see side_effect_policy_only_blocks_execution), so it must be
        // allow-listed here or the path under test never executes. ~keep
        allowed_side_effects: vec![SideEffectClass::Network],
        ..RunnerConfig::default()
    };

    let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

    assert_eq!(*timeouts.lock().expect("timeouts"), vec![1, 1]);
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.passed, 1);
    for value in &summary.results {
        let message = value.message.as_deref().unwrap_or_default();
        assert!(
            !message.contains("batch"),
            "reported against a batch command: {message}"
        );
    }
}

/// A validator that runs one process per snippet receives the configured timeout for every
/// snippet, and no snippet is reported against a command the runner never spawned.
#[test]
fn non_batching_validators_receive_the_configured_timeout_per_snippet() {
    let singles = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(BudgetRecordingValidator {
        singles: Arc::clone(&singles),
        batched: Arc::new(Mutex::new(Vec::new())),
        supports_batching: false,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        parallelism: 1,
        timeout_secs: 7,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let snippets = vec![network_snippet(), network_snippet(), network_snippet()];

    let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

    assert_eq!(*singles.lock().expect("single timeouts"), vec![7, 7, 7]);
    assert_eq!(summary.passed, 3);
    assert_eq!(summary.errors, 0);
}

/// Group budgeting is retained where it is meaningful: a validator that really does cover N
/// snippets with a single process is handed the whole budget once, not a share of it.
#[test]
fn batching_validators_still_receive_the_group_budget_once() {
    let singles = Arc::new(Mutex::new(Vec::new()));
    let batched = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(BudgetRecordingValidator {
        singles: Arc::clone(&singles),
        batched: Arc::clone(&batched),
        supports_batching: true,
    }));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        parallelism: 1,
        timeout_secs: 7,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let snippets = vec![network_snippet(), network_snippet(), network_snippet()];

    let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

    assert_eq!(*batched.lock().expect("batch timeouts"), vec![(3, 7)]);
    assert!(singles.lock().expect("single timeouts").is_empty());
    assert_eq!(summary.passed, 3);
}

fn plain_rust_snippet(id: &str) -> Snippet {
    Snippet {
        id: Some(id.to_string()),
        path: "example.md".into(),
        language: crate::snippets::types::Language::Rust,
        title: None,
        code: format!("fn {id}() {{}}"),
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

/// The "artifacts absent" half of task #542. Once a snippet's `compile`-level check has run and
/// established (and cached) that it cannot back itself against a missing build artifact --
/// `SnippetStatus::Unavailable` with `unresolved_dependency` set, exactly the shape
/// `finalize_result` (in `runner.rs`, this module's parent) produces for that cause -- a later
/// run with the identical snippet and session state must replay the cached verdict rather than
/// re-invoking the validator. `alef
/// all`/`alef docs` never build first (see `docs::enforce_snippet_summary`'s doc comment), so on
/// a steady-state development loop this cache hit is the common case, and re-attempting a
/// compiler invocation that can only fail the same way on every single run is exactly the cost
/// `docs::generate_docs_stage_without_snippet_compile_validation`'s own doc comment already
/// measured for a different caller (thousands of subprocess spawns to answer a question the cache
/// already answered). ~keep
#[test]
fn a_cached_unresolved_dependency_result_is_replayed_without_reinvoking_the_validator() {
    let singles = Arc::new(Mutex::new(0));
    let batches = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(RecordingValidator {
        language: crate::snippets::types::Language::Rust,
        batches: Arc::clone(&batches),
        singles: Arc::clone(&singles),
    }));
    let cache_directory = tempfile::tempdir().expect("cache directory");
    let snippet = plain_rust_snippet("fixture_unbuilt");
    let cached = ValidationResult {
        unresolved_dependency: true,
        ..result(
            &snippet,
            SnippetStatus::Unavailable,
            ValidationLevel::Compile,
            ValidationLevel::Compile,
            Some("cannot find crate `sample_core`".into()),
            1,
        )
    };
    ValidationCache::new(cache_directory.path().into())
        .store(&snippet, ValidationLevel::Compile, None, false, &[], &cached)
        .expect("cache entry");
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        changed_only: true,
        cache_dir: Some(cache_directory.path().into()),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(
        *singles.lock().expect("single count"),
        0,
        "a cache-hit snippet must never reach the validator's single-snippet path"
    );
    assert!(
        batches.lock().expect("batch records").is_empty(),
        "a cache-hit snippet must never reach the validator's batch path either"
    );
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].status, SnippetStatus::Unavailable);
    assert!(summary.results[0].unresolved_dependency);
    assert_eq!(summary.unresolved_dependency, 1);
}

/// The "artifacts present" half of task #542, and the one that matters: nothing about replaying a
/// cached unavailable verdict above may turn into "compile-level validation never runs for real".
/// With no matching cache entry -- the state right after a real `alef build` produces the
/// artifact the snippet above was missing, which changes the session's content and therefore its
/// cache key (see `session::fingerprint`'s doc comment on why built output must be inside the
/// hashed tree) -- the validator must still be invoked and can still pass. ~keep
#[test]
fn an_uncached_snippet_still_reaches_the_validator_at_the_requested_level() {
    let singles = Arc::new(Mutex::new(0));
    let batches = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(RecordingValidator {
        language: crate::snippets::types::Language::Rust,
        batches: Arc::clone(&batches),
        singles: Arc::clone(&singles),
    }));
    let cache_directory = tempfile::tempdir().expect("cache directory");
    let snippet = plain_rust_snippet("fixture_built");
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        changed_only: true,
        cache_dir: Some(cache_directory.path().into()),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

    assert_eq!(
        batches.lock().expect("batch records").as_slice(),
        &[(crate::snippets::types::Language::Rust, 1, false)],
        "with no cache entry, the snippet must actually reach the validator's batch path"
    );
    assert_eq!(summary.passed, 1);
    assert_eq!(summary.unresolved_dependency, 0);
}
