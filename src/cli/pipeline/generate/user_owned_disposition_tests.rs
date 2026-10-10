//! `[workspace.ownership] user_owned` -- the declared "this file is mine, not alef's" state.
//!
//! In-crate rather than under `tests/` on purpose. The log assertions below use
//! `tracing_test::traced_test`, whose thread-local subscriber is filtered to the *test crate's*
//! name; from an integration test that crate is not `alef`, so a capture would see none of
//! these events and a passing assertion would prove nothing -- the exact vacuous-check shape
//! `tests/pipeline_regeneration_gate.rs` hand-rolls its own subscriber to avoid. ~keep

use crate::core::backend::GeneratedFile;
use std::path::{Path, PathBuf};
use tracing_test::traced_test;

fn seed_config(base_dir: &Path, user_owned: &[&str]) {
    let patterns = user_owned
        .iter()
        .map(|pattern| format!("  \"{pattern}\",\n"))
        .collect::<String>();
    let ownership = if user_owned.is_empty() {
        String::new()
    } else {
        format!("[workspace.ownership]\nuser_owned = [\n{patterns}]\n\n")
    };
    std::fs::write(
        base_dir.join("alef.toml"),
        format!("{ownership}[[crates]]\nname = \"sample_core\"\nsources = [\"src/lib.rs\"]\n"),
    )
    .expect("write alef.toml");
}

fn seed_existing(base_dir: &Path, relative: &str, content: &str) -> PathBuf {
    let full = base_dir.join(relative);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("create parent");
    std::fs::write(&full, content).expect("seed existing file");
    full
}

fn generated(relative: &str, content: &str, generated_header: bool) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(relative),
        content: content.to_owned(),
        generated_header,
    }
}

/// The reported defect, reduced: a hand-maintained create-once seed reached by a write stage
/// that passes `overwrite: true` (every e2e, test-apps, README and docs write does) is refused
/// by the ownership guard on every run, and the refusal names `alef adopt <path>` -- which
/// `alef adopt` then declines for the same file because it is a create-once seed. Declaring the
/// path must turn that permanent failure into a declared skip without touching a byte. ~keep
#[test]
fn a_declared_user_owned_file_is_skipped_rather_than_refused_and_is_not_overwritten() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["e2e/*/index.test.js"]);
    let existing = seed_existing(base_dir, "e2e/node/index.test.js", "// hand-maintained by this repo\n");

    let report = super::scaffold::write_scaffold_files_report(
        &[generated("e2e/node/index.test.js", "// alef placeholder\n", false)],
        base_dir,
        true,
    )
    .expect("write");

    assert_eq!(
        std::fs::read_to_string(&existing).expect("read back"),
        "// hand-maintained by this repo\n",
        "a declared user-owned file must not be rewritten"
    );
    assert!(
        report.refused_paths.is_empty(),
        "a declared path is a disposition, not a failure; refused: {:?}",
        report.refused_paths
    );
    assert_eq!(
        report.user_owned_paths.iter().collect::<Vec<_>>(),
        vec![&existing],
        "the declared skip must be counted so the run can state it"
    );
    assert!(
        report.expected_paths.contains(&existing),
        "a declared path must stay in the expected set or the next orphan sweep deletes it"
    );
}

/// The control. Nothing about the declaration may loosen the guard for a path nobody declared:
/// an undeclared create-once seed reached under `overwrite: true` must still be refused, by
/// name, and still counted as a refusal. ~keep
#[test]
fn an_undeclared_create_once_seed_is_still_refused_exactly_as_before() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &[]);
    let existing = seed_existing(base_dir, "e2e/node/index.test.js", "// hand-maintained by this repo\n");

    let report = super::scaffold::write_scaffold_files_report(
        &[generated("e2e/node/index.test.js", "// alef placeholder\n", false)],
        base_dir,
        true,
    )
    .expect("write");

    assert_eq!(
        std::fs::read_to_string(&existing).expect("read back"),
        "// hand-maintained by this repo\n",
        "the ownership guard must still leave an undeclared unmarked file alone"
    );
    assert_eq!(
        report.refused_paths.iter().collect::<Vec<_>>(),
        vec![&existing],
        "an undeclared seed must still be reported as a refusal"
    );
    assert!(
        report.user_owned_paths.is_empty(),
        "nothing was declared, so nothing may be counted as a declared skip"
    );
}

/// The pinned answer to "how does a declared-but-absent file behave": alef SEEDS it once, so
/// `alef verify`'s missing-generated-file check has something to find, and seeds it WITHOUT a
/// provenance marker, so no later run can mistake the declaration for alef ownership. The seed
/// is an ordinary write, counted in `changed_paths`, not a declared skip. ~keep
#[test]
fn a_declared_path_that_does_not_exist_is_seeded_once_and_left_unstamped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["e2e/*/index.test.js"]);
    let target = base_dir.join("e2e/node/index.test.js");

    let report = super::scaffold::write_scaffold_files_report(
        &[generated("e2e/node/index.test.js", "// alef placeholder\n", true)],
        base_dir,
        true,
    )
    .expect("write");

    let written = std::fs::read_to_string(&target).expect("the declared path must be seeded when absent");
    assert!(
        written.contains("// alef placeholder"),
        "the seed must carry the generator's content: {written:?}"
    );
    assert!(
        !crate::core::hash::content_has_alef_marker(&written),
        "a declared user-owned seed must NOT be stamped -- a marker would let a later run's \
         ownership guard authorise the overwrite the declaration forbids: {written:?}"
    );
    assert!(
        report.changed_paths.contains(&target),
        "the seeding write is an ordinary write and must be counted as one"
    );
    assert!(
        report.user_owned_paths.is_empty(),
        "nothing was skipped this run -- the path did not exist yet"
    );
}

/// The seeding write must not leave an ownership-record entry either. For an unmarkable format
/// (`.json` has no comment syntax) the committed record is the ONLY thing the write guard
/// consults, so recording the seed would hand a later run exactly the licence the declaration
/// withholds -- the marker-shaped hole in the same argument. ~keep
#[test]
fn a_seeded_declared_path_is_never_written_into_the_ownership_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["e2e/*/package.json"]);

    super::scaffold::write_scaffold_files_report(
        &[generated(
            "e2e/node/package.json",
            "{\n  \"name\": \"sample\"\n}\n",
            false,
        )],
        base_dir,
        true,
    )
    .expect("write");

    let record = std::fs::read_to_string(base_dir.join(crate::cli::cache::OWNERSHIP_MANIFEST)).unwrap_or_default();
    assert!(
        !record.contains("package.json"),
        "a declared user-owned seed must not be recorded as alef-owned: {record:?}"
    );
}

/// The whole point, end to end: the run AFTER the seeding one must leave a consumer's edits
/// alone and report the path as a declared skip. Without this the disposition would only be a
/// first-run property, which is precisely what `generated_header: false` already fails to be
/// for `overwrite: true` stages. ~keep
#[test]
fn a_second_run_leaves_a_hand_edited_declared_seed_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["e2e/*/index.test.js"]);
    let target = base_dir.join("e2e/node/index.test.js");

    super::scaffold::write_scaffold_files_report(
        &[generated("e2e/node/index.test.js", "// alef placeholder\n", true)],
        base_dir,
        true,
    )
    .expect("first write");
    std::fs::write(&target, "// grown past the placeholder\n").expect("hand edit");

    let second = super::scaffold::write_scaffold_files_report(
        &[generated("e2e/node/index.test.js", "// alef placeholder v2\n", true)],
        base_dir,
        true,
    )
    .expect("second write");

    assert_eq!(
        std::fs::read_to_string(&target).expect("read back"),
        "// grown past the placeholder\n",
        "the consumer's edit must survive every later run"
    );
    assert!(second.refused_paths.is_empty(), "still a disposition, never a failure");
    assert_eq!(second.user_owned_count(), 1, "the declared skip must be counted");
}

fn stale_dart_pubspec() -> &'static str {
    r#"name: sample
environment:
  sdk: '>=3.11.0 <4.0.0'
executables:
  download_libs:
dependencies:
  crypto: '^3.0.7'
dev_dependencies:
  freezed: '^4.0.0'
"#
}

/// Scaffold migrations run after the guarded write loop. A declared path must remain protected
/// through that second phase; otherwise Alef reports a user-owned skip and then mutates the same
/// file behind that report. ~keep
#[test]
fn a_declared_user_owned_manifest_is_not_rewritten_by_a_scaffold_migration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["packages/dart/pubspec.yaml"]);
    let target = seed_existing(base_dir, "packages/dart/pubspec.yaml", stale_dart_pubspec());

    let report = super::scaffold::write_scaffold_files_report(
        &[generated(
            "packages/dart/pubspec.yaml",
            "name: generated\nenvironment:\n  sdk: '>=3.13.0 <4.0.0'\n",
            false,
        )],
        base_dir,
        true,
    )
    .expect("write");

    assert_eq!(
        std::fs::read_to_string(&target).expect("read back"),
        stale_dart_pubspec(),
        "the migration phase must preserve a declared user-owned manifest byte-for-byte"
    );
    assert_eq!(report.user_owned_count(), 1, "the guarded writer must report the skip");
}

/// Negative control for the migration guard: protecting declared paths must not disable the
/// repair for ordinary Alef-managed create-once manifests. ~keep
#[test]
fn an_undeclared_manifest_still_receives_its_scaffold_migration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &[]);
    let target = seed_existing(base_dir, "packages/dart/pubspec.yaml", stale_dart_pubspec());

    super::scaffold::write_scaffold_files_report(
        &[generated(
            "packages/dart/pubspec.yaml",
            "name: generated\nenvironment:\n  sdk: '>=3.13.0 <4.0.0'\n",
            false,
        )],
        base_dir,
        true,
    )
    .expect("write");

    let migrated = std::fs::read_to_string(target).expect("read migrated manifest");
    assert!(
        migrated.contains(crate::core::template_versions::toolchain::DART_SDK_CONSTRAINT),
        "the undeclared Alef manifest must still migrate: {migrated}"
    );
    assert!(
        !migrated.contains(">=3.11.0"),
        "the negative control must prove that the migration actually fired: {migrated}"
    );
}

/// The same disposition applied by the binding writer, which has no create-once concept at all:
/// `write_files_report` refuses an unmarked pre-existing file on EVERY run regardless of any
/// flag, so a declaration that only reached the scaffold writer would leave half the reported
/// paths still failing. ~keep
#[test]
fn the_binding_writer_honours_the_same_declaration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_dir = temp.path();
    seed_config(base_dir, &["packages/node/handwritten.ts"]);
    let existing = seed_existing(base_dir, "packages/node/handwritten.ts", "// this repo's\n");

    let report = super::write::write_files_report(
        &[(
            crate::core::config::Language::Node,
            vec![generated("packages/node/handwritten.ts", "// alef output\n", true)],
        )],
        base_dir,
    )
    .expect("write");

    assert_eq!(
        std::fs::read_to_string(&existing).expect("read back"),
        "// this repo's\n",
        "the binding writer must honour the declaration too"
    );
    assert!(report.refused_paths.is_empty(), "declared, so not a refusal");
    assert_eq!(report.user_owned_count(), 1, "declared skips must be counted here too");
}

/// COVERAGE GAP CLOSED: before this test nothing anywhere asserted the "N file(s) were NOT
/// written" tally. `all_commands_refusal_tests` asserts only that the report *fired*
/// (`logs_contain("alef adopt")`), which a report saying "0 file(s)" or naming the wrong count
/// would pass identically. The count is the whole operator-facing fact. ~keep
#[test]
#[traced_test]
fn the_refusal_report_states_how_many_files_were_not_written() {
    let report = super::write::WriteReport {
        refused_paths: [PathBuf::from("/repo/a.rs"), PathBuf::from("/repo/b.rs")]
            .into_iter()
            .collect(),
        ..super::write::WriteReport::default()
    };

    super::write::report_refused_writes(&report);

    assert!(
        logs_contain("2 file(s) were NOT written"),
        "the refusal report must state the count, not merely fire"
    );
    assert!(logs_contain("/repo/a.rs"), "and must name each refused path");
}

/// A declared skip is reported as a count, in its own category, and must not be phrased as a
/// failure -- otherwise the option changes nothing an operator can see and the stable bad state
/// survives its own fix. ~keep
#[test]
#[traced_test]
fn the_declared_skip_report_states_a_count_without_calling_it_a_failure() {
    let report = super::write::WriteReport {
        user_owned_paths: ["a.json", "b.swift", "c.kt"]
            .into_iter()
            .map(|name| PathBuf::from("/repo").join(name))
            .collect(),
        ..super::write::WriteReport::default()
    };

    super::write::report_user_owned_skips(&report);

    assert!(
        logs_contain("3 file(s) were not written because"),
        "the declared-skip report must state its own count"
    );
    assert!(
        logs_contain("workspace.ownership"),
        "and must name the declaration a reader can go and audit"
    );
    assert!(
        !logs_contain("were NOT written"),
        "a declared skip must never be worded as the refusal tally -- the categories are the point"
    );
}

/// An empty report says nothing at all. A per-run line for a repository that declares nothing
/// would be noise, and noise is how the refusal tally became something operators stopped
/// reading. ~keep
#[test]
#[traced_test]
fn nothing_is_reported_when_no_path_is_declared() {
    super::write::report_user_owned_skips(&super::write::WriteReport::default());
    assert!(!logs_contain("workspace.ownership"));
}
