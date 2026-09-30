//! Regression coverage for `scripts/toolchain-census.sh` and `scripts/toolchain-census-dir.sh`
//! resolving the census directory through `cargo metadata`'s `target_directory` instead of a
//! hard-coded `target/toolchain-census` -- which a custom `CARGO_TARGET_DIR` or
//! `.cargo/config.toml` `build.target-dir` leaves empty, even though
//! `src/test_support/toolchain.rs` (`census_file()`) already wrote the real tallies there.
//!
//! Exercises the scripts directly, the same way `tests/publish_scoop_gate_test.rs` exercises
//! `scripts/publish/compute-scoop-gate.sh`, rather than reimplementing their logic in Rust.

use std::path::PathBuf;
use std::process::Output;

#[path = "support/publish_shell_diagnostics.rs"]
mod shell_diagnostics;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn census_script() -> PathBuf {
    repo_root().join("scripts/toolchain-census.sh")
}

fn census_dir_script() -> PathBuf {
    repo_root().join("scripts/toolchain-census-dir.sh")
}

/// Runs `toolchain-census.sh` from the repo root (so its own `cargo metadata` default resolves
/// against this workspace), optionally with a `CARGO_TARGET_DIR` override.
fn run_census(args: &[&str], cargo_target_dir: Option<&std::path::Path>) -> Output {
    let mut command = shell_diagnostics::bash_command();
    command.arg(census_script()).args(args).current_dir(repo_root());
    match cargo_target_dir {
        Some(dir) => {
            command.env("CARGO_TARGET_DIR", dir);
        }
        None => {
            command.env_remove("CARGO_TARGET_DIR");
        }
    }
    command.output().expect("toolchain-census.sh must run")
}

fn write_row(census_dir: &std::path::Path, toolchain: &str, attempted: u32, executed: u32, absent: u32, unusable: u32) {
    std::fs::create_dir_all(census_dir).expect("create census dir");
    std::fs::write(
        census_dir.join("some-test-binary.tsv"),
        format!("{toolchain}\t{attempted}\t{executed}\t{absent}\t{unusable}\n"),
    )
    .expect("write census row");
}

#[test]
fn explicit_dir_empty_fails_a_required_toolchain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    std::fs::create_dir_all(&census_dir).expect("create empty census dir");

    let output = run_census(&["--dir", census_dir.to_str().unwrap(), "--require", "go"], None);

    assert!(
        !output.status.success(),
        "an empty --dir must fail a required toolchain: {}",
        shell_diagnostics::describe(&output)
    );
}

#[test]
fn explicit_dir_with_a_zero_executed_row_fails_a_required_toolchain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    write_row(&census_dir, "go", 3, 0, 3, 0);

    let output = run_census(&["--dir", census_dir.to_str().unwrap(), "--require", "go"], None);

    assert!(
        !output.status.success(),
        "a row with executed=0 must fail a required toolchain: {}",
        shell_diagnostics::describe(&output)
    );
}

#[test]
fn optional_unusable_toolchain_is_reported_distinctly_from_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    write_row(&census_dir, "go", 2, 0, 0, 2);

    let output = run_census(&["--dir", census_dir.to_str().unwrap()], None);

    assert!(output.status.success(), "{}", shell_diagnostics::describe(&output));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("0 absent, 2 unusable"), "unexpected census:\n{stdout}");
    assert!(stdout.contains("present but unusable"), "unexpected census:\n{stdout}");
}

#[test]
fn required_unusable_toolchain_fails_with_the_unusable_count() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    write_row(&census_dir, "swift", 1, 0, 0, 1);

    let output = run_census(&["--dir", census_dir.to_str().unwrap(), "--require", "swift"], None);

    assert!(!output.status.success(), "{}", shell_diagnostics::describe(&output));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(
        stdout.contains("0 absent and 1 unusable"),
        "unexpected census:\n{stdout}"
    );
}

#[test]
fn inconsistent_outcome_total_is_rejected_even_for_an_optional_toolchain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    write_row(&census_dir, "go", 2, 1, 0, 0);

    let output = run_census(&["--dir", census_dir.to_str().unwrap()], None);

    assert!(!output.status.success(), "{}", shell_diagnostics::describe(&output));
}

#[test]
fn required_toolchain_fails_unless_every_attempt_executed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    write_row(&census_dir, "go", 2, 1, 1, 0);

    let output = run_census(&["--dir", census_dir.to_str().unwrap(), "--require", "go"], None);

    assert!(!output.status.success(), "{}", shell_diagnostics::describe(&output));
}

#[test]
fn malformed_numeric_census_row_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let census_dir = temp.path().join("toolchain-census");
    std::fs::create_dir_all(&census_dir).expect("create census dir");
    std::fs::write(census_dir.join("malformed.tsv"), "go\ttwo\t1\t1\t0\n").expect("write malformed row");

    let output = run_census(&["--dir", census_dir.to_str().unwrap()], None);

    assert!(!output.status.success(), "{}", shell_diagnostics::describe(&output));
}

#[test]
fn default_dir_under_a_custom_cargo_target_dir_empty_fails_a_required_toolchain() {
    let target_dir = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    // Nothing is written under `<target_dir>/toolchain-census` at all.

    let output = run_census(&["--require", "go"], Some(target_dir.path()));

    assert!(
        !output.status.success(),
        "an empty default dir under a custom CARGO_TARGET_DIR must fail a required toolchain: {}",
        shell_diagnostics::describe(&output)
    );
}

#[test]
fn default_dir_under_a_custom_cargo_target_dir_with_a_zero_executed_row_fails() {
    let target_dir = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    write_row(&target_dir.path().join("toolchain-census"), "go", 2, 0, 2, 0);

    let output = run_census(&["--require", "go"], Some(target_dir.path()));

    assert!(
        !output.status.success(),
        "a row with executed=0 under the default dir must fail a required toolchain: {}",
        shell_diagnostics::describe(&output)
    );
}

#[test]
fn default_dir_under_a_custom_cargo_target_dir_passes_when_fully_executed() {
    let target_dir = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");
    write_row(&target_dir.path().join("toolchain-census"), "go", 2, 2, 0, 0);

    let output = run_census(&["--require", "go"], Some(target_dir.path()));

    assert!(
        output.status.success(),
        "a fully-executed row under a custom CARGO_TARGET_DIR must pass: {}",
        shell_diagnostics::describe(&output)
    );
}

#[test]
fn custom_cargo_target_dir_resolves_to_its_own_toolchain_census_subdirectory() {
    let target_dir = tempfile::tempdir().expect("tempdir for CARGO_TARGET_DIR");

    let mut command = shell_diagnostics::bash_command();
    command
        .arg(census_dir_script())
        .current_dir(repo_root())
        .env("CARGO_TARGET_DIR", target_dir.path());
    let output = command.output().expect("toolchain-census-dir.sh must run");

    assert!(output.status.success(), "{}", shell_diagnostics::describe(&output));
    let printed = String::from_utf8(output.stdout).expect("stdout must be utf8");
    // Compared as paths, not strings: the script always prints forward slashes (see
    // `json_escaped_windows_target_directory_is_printed_with_forward_slashes` below), while
    // `Path::join` on Windows produces backslashes. `Path`'s `PartialEq` treats both as
    // separators, so this still fails if the directory itself is wrong.
    assert_eq!(
        PathBuf::from(printed.trim()),
        target_dir.path().join("toolchain-census"),
        "the default census dir must sit under CARGO_TARGET_DIR, not the crate's own target/"
    );
}

/// On Windows `cargo metadata` JSON-escapes the target directory (`D:\\a\\x\\target`); the helper
/// must hand back a spelling `rm -rf` in Git-for-Windows bash accepts, i.e. forward slashes. ~keep
#[test]
fn json_escaped_windows_target_directory_is_printed_with_forward_slashes() {
    let stub_bin = tempfile::tempdir().expect("tempdir for the cargo stub");
    let cargo_stub = stub_bin.path().join("cargo");
    std::fs::write(
        &cargo_stub,
        "#!/usr/bin/env bash\nprintf '%s' '{\"target_directory\":\"D:\\\\a\\\\alef\\\\alef\\\\target\",\"workspace_root\":\"D:\\\\a\"}'\n",
    )
    .expect("write cargo stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cargo_stub, std::fs::Permissions::from_mode(0o755)).expect("chmod cargo stub");
    }
    let path = format!(
        "{}:{}",
        stub_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut command = shell_diagnostics::bash_command();
    command
        .arg(census_dir_script())
        .current_dir(repo_root())
        .env("PATH", path)
        .env_remove("CARGO_TARGET_DIR");
    let output = command.output().expect("toolchain-census-dir.sh must run");

    assert!(output.status.success(), "{}", shell_diagnostics::describe(&output));
    let printed = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert_eq!(
        printed.trim(),
        "D:/a/alef/alef/target/toolchain-census",
        "doubled JSON backslashes must be turned into forward slashes"
    );
}
