//! Tests for [`super`], the e2e generation driver.
//!
//! Split out of `mod.rs` so the driver stays comfortably under the modularization cap rather than
//! sitting on it — threading one new IR input through the snippet stage was enough to reach it.

use super::*;

#[test]
fn undocumented_missing_recipe_fails_generation() {
    let coverage = snippets::SnippetCoverageLedger {
        missing: vec![snippets::MissingSnippet {
            key: snippets::SnippetCoverageKey {
                fixture_id: "create_record".into(),
                language: "go".into(),
            },
            reason: "built-in `go` snippet recipe has no function identity".into(),
        }],
        ..Default::default()
    };

    let error = ensure_snippet_coverage_complete(&coverage).expect_err("missing recipe must fail closed");
    assert!(error.to_string().contains("create_record"));
    assert!(error.to_string().contains("go"));
}

#[test]
fn documented_exceptions_do_not_fail_generation() {
    let coverage = snippets::SnippetCoverageLedger {
        documented_exceptions: vec![snippets::DocumentedSnippetException {
            key: snippets::SnippetCoverageKey {
                fixture_id: "stream_records".into(),
                language: "swift".into(),
            },
            reason: "streaming recipe is documented separately".into(),
            reference: "docs/streaming.md".into(),
        }],
        ..Default::default()
    };

    ensure_snippet_coverage_complete(&coverage).expect("documented exception is intentional");
}

fn key(fixture_id: &str, language: &str) -> snippets::SnippetCoverageKey {
    snippets::SnippetCoverageKey {
        fixture_id: fixture_id.into(),
        language: language.into(),
    }
}

fn metadata(fixture_id: &str, language: &str, path: &str) -> snippets::GeneratedSnippetMetadata {
    snippets::GeneratedSnippetMetadata {
        key: key(fixture_id, language),
        path: std::path::PathBuf::from(path),
        language: language.into(),
        target: language.into(),
        session: language.into(),
        requires: Vec::new(),
        side_effect: crate::e2e::fixture::SideEffectClass::Safe,
    }
}

fn write_previous_manifest(output_root: &Path, metadata_entries: Vec<snippets::GeneratedSnippetMetadata>) {
    let ledger = snippets::SnippetCoverageLedger {
        format_version: snippets::COVERAGE_MANIFEST_VERSION,
        generated_paths: metadata_entries.iter().map(|entry| entry.path.clone()).collect(),
        generated: metadata_entries.iter().map(|entry| entry.key.clone()).collect(),
        expected: metadata_entries.iter().map(|entry| entry.key.clone()).collect(),
        generated_metadata: metadata_entries,
        missing: Vec::new(),
        documented_exceptions: Vec::new(),
    };
    std::fs::write(
        output_root.join(snippets::COVERAGE_MANIFEST),
        serde_json::to_string_pretty(&ledger).expect("serialize previous coverage ledger"),
    )
    .expect("write previous coverage manifest");
}

/// A fixture that stopped rendering between runs must have its stale
/// on-disk `.md` file deleted, not left behind for `alef verify` to keep
/// validating forever. This is task #542.
#[test]
fn prune_orphaned_snippets_deletes_a_file_this_run_no_longer_generates() {
    let directory = tempfile::tempdir().expect("temporary output directory");
    let output_root = directory.path();
    let generated_dir = output_root.join("python");
    std::fs::create_dir_all(&generated_dir).expect("python output dir");
    let stale_file = generated_dir.join("register_ocr_backend_trait_bridge.md");
    std::fs::write(&stale_file, "stale 0.60.0 content\n").expect("write stale snippet");

    write_previous_manifest(
        output_root,
        vec![metadata(
            "register_ocr_backend_trait_bridge",
            "python",
            "python/register_ocr_backend_trait_bridge.md",
        )],
    );

    // The key was evaluated this run and rejected: it is in `expected`
    // but produced no file.
    let current = snippets::SnippetCoverageLedger {
        format_version: snippets::COVERAGE_MANIFEST_VERSION,
        expected: vec![key("register_ocr_backend_trait_bridge", "python")],
        missing: vec![snippets::MissingSnippet {
            key: key("register_ocr_backend_trait_bridge", "python"),
            reason: "test-backend fixture requires an extension-owned documentation recipe".into(),
        }],
        ..Default::default()
    };

    prune_orphaned_snippets(output_root, &current);

    assert!(
        !stale_file.exists(),
        "expected {} to be pruned, but it still exists",
        stale_file.display()
    );
}

/// A hand-authored `.md` file that alef never generated (absent from the
/// previous run's `generated_metadata`) must never be deleted, even if a
/// fixture with a colliding id/language key is reported as `missing`.
#[test]
fn prune_orphaned_snippets_never_deletes_a_file_alef_does_not_own() {
    let directory = tempfile::tempdir().expect("temporary output directory");
    let output_root = directory.path();
    let hand_authored_dir = output_root.join("python");
    std::fs::create_dir_all(&hand_authored_dir).expect("python output dir");
    let hand_authored_file = hand_authored_dir.join("register_ocr_backend_trait_bridge.md");
    std::fs::write(&hand_authored_file, "hand-authored recipe, not alef output\n").expect("write hand-authored");

    // The previous manifest never claims this path: alef never generated it.
    write_previous_manifest(output_root, Vec::new());

    let current = snippets::SnippetCoverageLedger {
        format_version: snippets::COVERAGE_MANIFEST_VERSION,
        expected: vec![key("register_ocr_backend_trait_bridge", "python")],
        missing: vec![snippets::MissingSnippet {
            key: key("register_ocr_backend_trait_bridge", "python"),
            reason: "test-backend fixture requires an extension-owned documentation recipe".into(),
        }],
        ..Default::default()
    };

    prune_orphaned_snippets(output_root, &current);

    assert!(
        hand_authored_file.exists(),
        "hand-authored file must survive: {}",
        hand_authored_file.display()
    );
}

#[test]
fn generation_does_not_write_fixture_schema() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let e2e_config = E2eConfig {
        fixtures: directory.path().display().to_string(),
        ..E2eConfig::default()
    };

    let (_files, deferred_error) = generate_e2e(
        &ResolvedCrateConfig::default(),
        &e2e_config,
        Some(&[]),
        &[],
        &[],
        &[],
        &[],
    )
    .expect("generate empty E2E suite");

    assert!(
        deferred_error.is_none(),
        "an empty fixture set has no backend to fail: {deferred_error:?}"
    );
    assert!(!directory.path().join("schema.json").exists());
}

fn write_docs_only_fixture(directory: &Path, filename: &str, references: serde_json::Value) {
    std::fs::write(
        directory.join(filename),
        serde_json::json!({
            "kind": "docs_only",
            "id": "config_discovery",
            "topic": "guides",
            "content": "Configuration is discovered by walking up from the working directory.",
            "references": references,
        })
        .to_string(),
    )
    .expect("write docs-only fixture");
}

/// A docs-only fixture referencing a field that does not exist in the extracted API surface
/// must fail the real `generate_e2e` pipeline, not just the unit-level validator. This is the
/// end-to-end shape of the coverage requirement: docs-only support is a validated capability,
/// not a skip.
#[test]
fn docs_only_fixture_with_a_bad_reference_fails_generation_through_the_real_pipeline() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    write_docs_only_fixture(
        directory.path(),
        "config_discovery.json",
        serde_json::json!([{"kind": "field", "path": "ConfigSource.does_not_exist"}]),
    );

    let e2e_config = E2eConfig {
        fixtures: directory.path().display().to_string(),
        ..E2eConfig::default()
    };

    let error = generate_e2e(
        &ResolvedCrateConfig::default(),
        &e2e_config,
        Some(&[]),
        &[],
        &[],
        &[],
        &[],
    )
    .expect_err("a docs-only fixture referencing a nonexistent field must fail generation");
    let message = format!("{error:#}");
    assert!(message.contains("config_discovery"), "{message}");
    assert!(message.contains("does_not_exist"), "{message}");
}

/// The converse: a docs-only fixture whose references all resolve renders under its own
/// `docs-only/` output slug and never enters the snippet coverage ledger -- proving both
/// coverage guarantees end to end (validated, and structurally isolated from runtime
/// coverage) rather than only at the unit level in `fixture::docs_only`'s own tests.
#[test]
fn docs_only_fixture_renders_under_its_own_slug_and_never_touches_snippet_coverage() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    write_docs_only_fixture(directory.path(), "config_discovery.json", serde_json::json!([]));

    let e2e_config = E2eConfig {
        fixtures: directory.path().display().to_string(),
        snippets: Some(crate::core::config::e2e::SnippetConfig {
            output: "docs/snippets-generated".to_string(),
            ..crate::core::config::e2e::SnippetConfig::default()
        }),
        ..E2eConfig::default()
    };

    let (files, deferred_error) = generate_e2e(
        &ResolvedCrateConfig::default(),
        &e2e_config,
        Some(&[]),
        &[],
        &[],
        &[],
        &[],
    )
    .expect("a docs-only fixture with resolvable references must generate cleanly");
    assert!(deferred_error.is_none(), "got: {deferred_error:?}");

    let docs_only_file = files
        .iter()
        .find(|file| file.path.starts_with("docs/snippets-generated/docs-only"))
        .expect("docs-only output file must be present");
    assert_eq!(
        docs_only_file.path,
        Path::new("docs/snippets-generated/docs-only/guides/config_discovery.md")
    );
    assert!(docs_only_file.content.contains("kind: docs_only"));

    let coverage_file = files
        .iter()
        .find(|file| file.path.ends_with(snippets::COVERAGE_MANIFEST))
        .expect("coverage manifest must still be written");
    let coverage: snippets::SnippetCoverageLedger =
        serde_json::from_str(&coverage_file.content).expect("coverage manifest must parse");
    for cell in coverage
        .expected
        .iter()
        .chain(coverage.generated.iter())
        .chain(coverage.missing.iter().map(|missing| &missing.key))
    {
        assert_ne!(
            cell.fixture_id, "config_discovery",
            "a docs-only fixture must never appear in the runtime snippet coverage ledger"
        );
    }
}

/// The real-world trigger behind the `adopt`/`verify` deadlock this crate's release
/// blocker fixed: a fixture asserting on a field the availability oracle rejects, with
/// no `skip` declared, must still fail strict mode by default -- proving the condition
/// `bin_cli::helpers::collect_managed_surface`'s `StageFailure` handling exists to
/// tolerate is genuinely reachable through the real generation pipeline, not only
/// through `codegen::strict_assertion_failure`'s own synthetic-string unit tests. ~keep
#[test]
fn an_unresolvable_field_with_no_skip_fails_strict_mode_through_the_real_pipeline() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    std::fs::write(
        directory.path().join("smoke.json"),
        r#"{
            "id": "smoke",
            "description": "smoke test",
            "assertions": [
                { "type": "not_empty", "field": "bogus_field_xyz" }
            ]
        }"#,
    )
    .expect("write fixture");

    let e2e_config = E2eConfig {
        fixtures: directory.path().display().to_string(),
        languages: vec!["python".to_string()],
        result_fields: std::collections::HashSet::from(["id".to_string()]),
        call: crate::core::config::e2e::CallConfig {
            function: "complete".to_string(),
            module: "gatelib".to_string(),
            ..crate::core::config::e2e::CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let (_files, deferred_error) = generate_e2e(&ResolvedCrateConfig::default(), &e2e_config, None, &[], &[], &[], &[])
        .expect("the files that did render must still be returned alongside a deferred failure");

    let error = deferred_error.expect("an unresolvable field with no `skip` must fail strict mode by default");
    assert!(
        format!("{error:#}").contains("e2e assertion(s) reference a field the availability oracle cannot resolve"),
        "got: {error:#}"
    );
}

struct FailingGenerator;

impl codegen::E2eCodegen for FailingGenerator {
    fn generate(
        &self,
        _groups: &[fixture::FixtureGroup],
        _e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        anyhow::bail!("simulated leaf-field resolution failure")
    }

    fn language_name(&self) -> &'static str {
        "failing"
    }

    /// Test double, no snippet recipe -- forwards to [`codegen::E2eCodegen::render_snippet_body`]'s
    /// own "does not support documentation snippets" default, same as `gleam`/`php_ext`/`homebrew`.
    fn render_snippet_body_with_functions(
        &self,
        fixture: &fixture::Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        self.render_snippet_body(fixture, e2e_config, config, type_defs, enums)
    }
}

struct SucceedingGenerator;

impl codegen::E2eCodegen for SucceedingGenerator {
    fn generate(
        &self,
        _groups: &[fixture::FixtureGroup],
        _e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        Ok(vec![GeneratedFile {
            path: std::path::PathBuf::from("ok/output.txt"),
            content: "generated".into(),
            generated_header: false,
        }])
    }

    fn language_name(&self) -> &'static str {
        "succeeding"
    }

    /// Test double, no snippet recipe -- forwards to [`codegen::E2eCodegen::render_snippet_body`]'s
    /// own "does not support documentation snippets" default, same as `gleam`/`php_ext`/`homebrew`.
    fn render_snippet_body_with_functions(
        &self,
        fixture: &fixture::Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        self.render_snippet_body(fixture, e2e_config, config, type_defs, enums)
    }
}

/// The regression this guards: a consumer's C backend hit `ensure_leaf_field_exists`'s
/// `bail!`, and because the old loop propagated it with `?` immediately, every
/// later-listed language generator was skipped too -- not just the C backend. One
/// backend's codegen failure must not stop its siblings from generating.
#[test]
fn run_generators_isolates_one_backend_failure_from_the_rest() {
    let generators: Vec<Box<dyn codegen::E2eCodegen>> = vec![Box::new(FailingGenerator), Box::new(SucceedingGenerator)];

    let (files, failures) = run_generators(
        &generators,
        &[],
        &E2eConfig::default(),
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
        &[],
    );

    assert_eq!(
        files.len(),
        1,
        "the succeeding backend's file must still be produced: {files:?}"
    );
    assert_eq!(files[0].path, std::path::PathBuf::from("ok/output.txt"));
    assert_eq!(
        failures.len(),
        1,
        "the failing backend's failure must be recorded: {failures:?}"
    );
    assert!(
        failures[0].contains("[failing]") && failures[0].contains("simulated leaf-field resolution failure"),
        "failure must name the backend and carry its own diagnostic verbatim: {failures:?}"
    );
}

struct FailingExtension;

impl crate::Extension for FailingExtension {
    fn name(&self) -> &str {
        "failing-e2e-extension"
    }

    fn emit_e2e(
        &self,
        _groups: &[fixture::FixtureGroup],
        _e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
        _language: &str,
        _type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        anyhow::bail!("simulated extension emission failure")
    }
}

/// Regression for the same defect class as
/// `run_generators_isolates_one_backend_failure_from_the_rest`, one layer over: an e2e
/// extension's `emit_e2e` failure must not discard the backend generator output
/// `run_generators` already merged into `all_files`, and must still surface as a deferred
/// failure rather than being swallowed.
///
/// Exercised through `generate_e2e_with_extensions` (a synthetic extensions list passed
/// directly) instead of the public `generate_e2e` (which reads `crate::EXTENSIONS`, a
/// process-global `OnceLock` settable exactly once per process) -- no individual test can
/// mutate that global without leaking its registration into every other test sharing this
/// binary for the rest of the process's lifetime. `generate_e2e_with_extensions` exists so
/// this failure mode is provable without that. ~keep
#[test]
fn generate_e2e_with_extensions_defers_an_extension_failure_so_backend_output_survives() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let e2e_config = E2eConfig {
        fixtures: directory.path().display().to_string(),
        output: "e2e".to_string(),
        languages: vec!["rust".to_string()],
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig::default();
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FailingExtension)];

    let log = crate::e2e::diagnostic_log::DiagnosticLog::new();

    let (files, deferred_error) =
        generate_e2e_with_extensions(&config, &e2e_config, None, &[], &[], &[], &[], &extensions, &log)
            .expect("an extension failure must defer through the `Option` slot, not propagate as a hard `Err`");

    let error = deferred_error.expect("the extension's failure must still be reported, not silently dropped");
    assert!(
        format!("{error:#}").contains("simulated extension emission failure"),
        "the deferred error must carry the extension's own diagnostic verbatim: {error:#}"
    );

    assert!(
        files
            .iter()
            .any(|file| file.path == *std::path::Path::new("e2e/rust/Cargo.toml")),
        "the `rust` backend's own e2e suite must survive an unrelated extension's failure: {files:?}"
    );
}

#[test]
fn ensure_no_generator_failures_passes_through_when_nothing_failed() {
    assert!(
        ensure_no_generator_failures(&[], 3).is_none(),
        "no failures must not produce a deferred error"
    );
}

#[test]
fn ensure_no_generator_failures_names_every_failed_backend_and_the_total_count() {
    let failures = vec![
        "[c] simulated leaf-field resolution failure".to_string(),
        "[go] simulated template error".to_string(),
    ];

    let message = ensure_no_generator_failures(&failures, 5)
        .expect("collected failures must still produce a deferred error")
        .to_string();

    assert!(
        message.contains("2 of 5"),
        "must report how many of the total backends failed: {message}"
    );
    assert!(
        message.contains("[c] simulated leaf-field resolution failure"),
        "{message}"
    );
    assert!(message.contains("[go] simulated template error"), "{message}");
}

/// Two java `module` overrides that each look like a class, so this fixture has exactly two
/// distinct validator diagnostics to account for -- enough that a dedup which silently dropped
/// one would not look the same as a dedup which only removed the repetition. ~keep
fn two_distinct_java_module_warnings() -> E2eConfig {
    use crate::core::config::e2e::{CallConfig, CallOverride};

    let java_class_module = |module: &str| {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "java".to_string(),
            CallOverride {
                module: Some(module.to_string()),
                ..CallOverride::default()
            },
        );
        call
    };

    E2eConfig {
        call: java_class_module("io.sample.Alpha"),
        calls: std::collections::BTreeMap::from([("beta".to_string(), java_class_module("io.sample.Beta"))]),
        ..E2eConfig::default()
    }
}

/// The pair `collect_managed_surface` renders for every crate: the same configuration in local
/// and registry dep mode, which no e2e validator reads. Each render's log is a separate argument
/// so a test can decide whether the two share one.
fn render_dep_modes(fixtures: &Path, local_log: &DiagnosticLog, registry_log: &DiagnosticLog) {
    let java = ["java".to_string()];
    let local = E2eConfig {
        fixtures: fixtures.display().to_string(),
        ..two_distinct_java_module_warnings()
    };
    let registry = E2eConfig {
        dep_mode: DependencyMode::Registry,
        ..local.clone()
    };
    for (e2e_config, log) in [(&local, local_log), (&registry, registry_log)] {
        generate_e2e_with_log(
            &ResolvedCrateConfig::default(),
            e2e_config,
            Some(java.as_slice()),
            &[],
            &[],
            &[],
            &[],
            log,
        )
        .expect("rendering an empty fixture set must not fail");
    }
}

fn module_warning_counts(lines: &[&str]) -> (usize, usize) {
    let count = |needle: &str| lines.iter().filter(|line| line.contains(needle)).count();
    (count("io.sample.Alpha"), count("io.sample.Beta"))
}

/// PROOF THE DEDUP IS WIRED TO SOMETHING. Without a shared log the two renders are two
/// independent invocations, so every diagnostic lands twice -- the 2x-per-crate log
/// `collect_managed_surface` produced before it shared one. If this ever reports 1, the
/// suppression asserted below is passing for some reason other than the one it claims. ~keep
#[tracing_test::traced_test]
#[test]
fn separate_logs_report_the_same_diagnostic_once_per_render() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");

    render_dep_modes(directory.path(), &DiagnosticLog::new(), &DiagnosticLog::new());

    logs_assert(|lines: &[&str]| match module_warning_counts(lines) {
        (2, 2) => Ok(()),
        other => Err(format!(
            "expected each diagnostic once per un-shared render, got {other:?}"
        )),
    });
}

/// One log across both dep modes halves the emitted count while leaving the distinct set whole:
/// both `io.sample.Alpha` and `io.sample.Beta` must still be reported, each exactly once. A test
/// that only asserted "fewer" would also pass if the second diagnostic had been dropped.
#[tracing_test::traced_test]
#[test]
fn one_shared_log_reports_each_diagnostic_once_across_both_dep_modes() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let log = DiagnosticLog::new();

    render_dep_modes(directory.path(), &log, &log);

    logs_assert(|lines: &[&str]| match module_warning_counts(lines) {
        (1, 1) => Ok(()),
        other => Err(format!(
            "the registry pass must repeat neither diagnostic and drop neither, got {other:?}"
        )),
    });
}

/// A later invocation is a new question, not a repeat of the answered one: suppression that
/// outlived its log would silence a diagnostic the operator has not yet seen in this run. ~keep
#[tracing_test::traced_test]
#[test]
fn a_later_invocations_identical_diagnostic_is_reported_again() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");

    let first_invocation = DiagnosticLog::new();
    let second_invocation = DiagnosticLog::new();

    render_dep_modes(directory.path(), &first_invocation, &first_invocation);
    render_dep_modes(directory.path(), &second_invocation, &second_invocation);

    logs_assert(|lines: &[&str]| match module_warning_counts(lines) {
        (2, 2) => Ok(()),
        other => Err(format!(
            "a fresh log must report what the previous one reported, got {other:?}"
        )),
    });
}

#[path = "lang_scope_tests.rs"]
mod lang_scope;
