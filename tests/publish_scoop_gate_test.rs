//! Static assertion that `scripts/publish/compute-scoop-gate.sh` -- the script
//! `.github/workflows/publish.yaml` uses to derive the Scoop release gate from the
//! `release_targets` output of `prepare-release-metadata@v1` -- evaluates to `true` for every
//! shape a scoop-enabled `alef release-metadata` run can produce, including the `all` case,
//! and does not false-positive on a target name that merely contains "scoop" as a substring.

use std::path::PathBuf;
use std::process::{Command, Output};

#[path = "support/publish_shell_diagnostics.rs"]
mod shell_diagnostics;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/publish/compute-scoop-gate.sh")
}

fn run_gate(release_targets: &str) -> Output {
    Command::new("bash")
        .arg(script_path())
        .env("RELEASE_TARGETS", release_targets)
        .output()
        .expect("compute-scoop-gate.sh must run")
}

fn assert_gate(release_targets: &str, expected: &str) {
    let output = run_gate(release_targets);
    assert!(
        output.status.success(),
        "compute-scoop-gate.sh exited non-zero for RELEASE_TARGETS={release_targets:?}: {}",
        shell_diagnostics::describe(&output)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert_eq!(
        stdout.trim(),
        expected,
        "RELEASE_TARGETS={release_targets:?} must evaluate to {expected:?}"
    );
}

#[test]
fn scoop_only_target_gates_true() {
    assert_gate("scoop", "true");
}

#[test]
fn scoop_among_other_targets_gates_true() {
    assert_gate("crates,cli,scoop", "true");
    assert_gate("scoop,homebrew", "true");
}

#[test]
fn all_targets_gates_true() {
    // `release_targets` collapses to the literal string "all" when every recognised target is
    // enabled (see `ReleaseMetadata::compute` in src/cli/commands/release_metadata.rs) -- the
    // gate must treat that case as scoop-enabled too, since "all" is reachable from a
    // `targets: all` dispatch input.
    assert_gate("all", "true");
}

#[test]
fn targets_without_scoop_gate_false() {
    assert_gate("crates,cli", "false");
    assert_gate("cli,homebrew", "false");
}

#[test]
fn none_targets_gates_false() {
    assert_gate("none", "false");
}

#[test]
fn substring_match_does_not_false_positive() {
    // A target list containing a name that merely contains "scoop" as a substring must not
    // gate the release -- the match is on exact comma-separated tokens only.
    assert_gate("crates,cli,scoope", "false");
}

#[test]
fn whitespace_around_scoop_token_still_gates_true() {
    // Defensive trimming: a leading/trailing space on the whole string or on one
    // comma-separated element must not flip the gate the wrong way. Untrimmed, " scoop" used
    // to read as absent -- the mirror image of the untrimmed-whole-string bug fixed in
    // normalize-release-targets.sh (see tests/publish_normalize_targets_test.rs).
    assert_gate(" scoop", "true");
    assert_gate("scoop ", "true");
    assert_gate("  scoop  ", "true");
    assert_gate(" crates , scoop ", "true");
    assert_gate(" all ", "true");
}
