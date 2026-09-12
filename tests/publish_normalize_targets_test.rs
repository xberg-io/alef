//! Static assertion that `scripts/publish/normalize-release-targets.sh` -- the script
//! `.github/workflows/publish.yaml` runs before `prepare-release-metadata@v1` -- collapses a
//! whitespace-only `targets` workflow input to `crates,cli` instead of letting it survive the
//! `inputs.targets || client_payload.targets || 'crates,cli'` fallback chain as a non-empty,
//! truthy string that Alef's own `--targets` parser would then trim to empty and treat as
//! "release everything" (see `parse_targets` in src/cli/commands/release_metadata.rs).

use std::path::PathBuf;
use std::process::Output;

#[path = "support/publish_shell_diagnostics.rs"]
mod shell_diagnostics;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/publish/normalize-release-targets.sh")
}

fn run_normalize(raw_targets: Option<&str>) -> Output {
    let mut command = shell_diagnostics::bash_command();
    command.arg(script_path());
    if let Some(raw) = raw_targets {
        command.env("RAW_TARGETS", raw);
    } else {
        command.env_remove("RAW_TARGETS");
    }
    command.output().expect("normalize-release-targets.sh must run")
}

fn assert_normalized(raw_targets: Option<&str>, expected: &str) {
    let output = run_normalize(raw_targets);
    assert!(
        output.status.success(),
        "normalize-release-targets.sh exited non-zero for RAW_TARGETS={raw_targets:?}: {}",
        shell_diagnostics::describe(&output)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert_eq!(
        stdout.trim(),
        expected,
        "RAW_TARGETS={raw_targets:?} must normalize to {expected:?}"
    );
}

#[test]
fn shell_command_uses_the_absolute_path_resolved_from_path() {
    let expected = which::which("bash").expect("bash must be installed for publish script tests");
    assert!(expected.is_absolute(), "resolved bash must be absolute: {expected:?}");
    let command = shell_diagnostics::bash_command();
    assert_eq!(command.get_program(), expected.as_os_str());
}

#[test]
fn whitespace_only_input_falls_back_to_crates_cli() {
    // The bug this guards: a whitespace-only manual-dispatch value is non-empty and therefore
    // truthy in the workflow's `||` fallback chain, so it used to win over the `crates,cli`
    // default and then get trimmed to empty by Alef's own parser -- which treats empty as
    // "all", silently enabling every opt-in target including Homebrew and Scoop.
    assert_normalized(Some("   "), "crates,cli");
    assert_normalized(Some("\t"), "crates,cli");
    assert_normalized(Some(" \t \n "), "crates,cli");
}

#[test]
fn empty_input_falls_back_to_crates_cli() {
    assert_normalized(Some(""), "crates,cli");
}

#[test]
fn unset_input_falls_back_to_crates_cli() {
    assert_normalized(None, "crates,cli");
}

#[test]
fn explicit_all_passes_through_unchanged() {
    // "all" must stay "all" -- an explicit request for every target is not the bug this script
    // guards against, and must still reach Alef's parser as "all" (release_any: true for
    // every target).
    assert_normalized(Some("all"), "all");
    assert_normalized(Some("  all  "), "all");
}

#[test]
fn explicit_none_passes_through_unchanged() {
    // "none" must stay "none" -- an explicit request to release nothing must not be
    // reinterpreted as "unset" and defaulted to crates,cli.
    assert_normalized(Some("none"), "none");
    assert_normalized(Some("  none  "), "none");
}

#[test]
fn ordinary_target_list_is_trimmed_but_otherwise_unchanged() {
    assert_normalized(Some("crates,cli"), "crates,cli");
    assert_normalized(Some("  cli,scoop  "), "cli,scoop");
    assert_normalized(Some(" cli,homebrew"), "cli,homebrew");
}
