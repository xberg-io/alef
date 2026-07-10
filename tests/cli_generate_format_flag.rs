/// Integration tests for the `--format` flag plumbing on `alef generate` and `alef all`.
///
/// Formatting now always runs — the `--format` flag is accepted for backward
/// compatibility (so that `alef all --clean --format=false` still parses without
/// error) but is hidden from the `--help` output.
///
/// These tests exercise only CLI flag plumbing: they confirm that the flag is
/// hidden from help yet still accepted by clap, and that `--no-format` is not
/// introduced.  Full formatting behaviour is covered by e2e tests that run
/// against a real alef project.
use std::process::Command;

fn alef_binary() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_alef") {
        return std::path::PathBuf::from(path);
    }
    let mut dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent")
        .to_path_buf();
    if dir.ends_with("deps") {
        dir = dir.parent().expect("parent of deps").to_path_buf();
    }
    dir.join("alef")
}

/// `alef generate --help` must NOT list `--format` (it is hidden) and must NOT
/// list `--no-format`.
#[test]
fn generate_help_hides_format_flag() {
    let output = Command::new(alef_binary())
        .args(["generate", "--help"])
        .output()
        .expect("failed to run alef generate --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("  --format"),
        "`alef generate --help` must not list --format (it is hidden); got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef generate --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef all --help` must NOT list `--format` (it is hidden) and must NOT list
/// `--no-format`.
#[test]
fn all_help_hides_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--help"])
        .output()
        .expect("failed to run alef all --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains("  --format"),
        "`alef all --help` must not list --format (it is hidden); got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef all --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef generate --format` must be accepted by clap (backward-compat hidden flag).
#[test]
fn generate_accepts_format_flag() {
    let output = Command::new(alef_binary())
        .args(["generate", "--format"])
        .output()
        .expect("failed to spawn alef");

    assert_ne!(
        output.status.code(),
        Some(2),
        "alef generate --format must be accepted by clap (not an unknown argument); got exit code 2"
    );
}

/// `alef all --format` must be accepted by clap (backward-compat hidden flag).
#[test]
fn all_accepts_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--format"])
        .output()
        .expect("failed to spawn alef");

    assert_ne!(
        output.status.code(),
        Some(2),
        "alef all --format must be accepted by clap (not an unknown argument); got exit code 2"
    );
}
