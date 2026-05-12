/// Integration tests verifying the format flag plumbing for `alef generate` and `alef all`.
///
/// Both `alef generate` and `alef all` use `--format` (opt-in: formatters are off by default).
///
/// These tests exercise only the CLI flag plumbing: they confirm the binary
/// exposes the right flags and that the help text is consistent.  Full
/// formatting behaviour is covered by e2e tests that run against a real alef project.
use std::process::Command;

fn alef_binary() -> std::path::PathBuf {
    // `cargo test` sets CARGO_BIN_EXE_alef when the binary is declared in Cargo.toml.
    // Fall back to finding it in the target directory for environments that don't set
    // the env var (e.g. when tests are run from a parent workspace).
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_alef") {
        return std::path::PathBuf::from(path);
    }
    // Traverse up to locate the workspace target directory.
    let mut dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent")
        .to_path_buf();
    // The test binary lives in target/<profile>/deps/; the CLI binary is in target/<profile>/.
    if dir.ends_with("deps") {
        dir = dir.parent().expect("parent of deps").to_path_buf();
    }
    dir.join("alef")
}

/// `alef generate --help` must list `--format` (opt-in) and must NOT list `--no-format`.
#[test]
fn generate_help_shows_format_flag() {
    let output = Command::new(alef_binary())
        .args(["generate", "--help"])
        .output()
        .expect("failed to run alef generate --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    // We check for the standalone flag line "  --format" (leading spaces that clap uses).
    assert!(
        combined.contains("  --format"),
        "`alef generate --help` must list --format flag; got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef generate --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef all --help` must list `--format` (opt-in: formatters are off by default).
#[test]
fn all_help_shows_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--help"])
        .output()
        .expect("failed to run alef all --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("  --format"),
        "`alef all --help` must list --format flag; got:\n{combined}"
    );
    assert!(
        !combined.contains("--no-format"),
        "`alef all --help` must not list --no-format; got:\n{combined}"
    );
}

/// `alef generate --format` must be accepted by clap (it is the opt-in formatter flag).
#[test]
fn generate_accepts_format_flag() {
    // clap parses flags before any config loading — `--format` should be accepted even
    // without a valid alef.toml.  The process may exit non-zero due to missing config,
    // but the exit code must not be the "unknown argument" error (2).
    let output = Command::new(alef_binary())
        .args(["generate", "--format"])
        .output()
        .expect("failed to spawn alef");

    // Exit code 2 is clap's "unknown argument" error.  Any other exit code means clap
    // accepted the flag and the failure (if any) is from config loading, not flag parsing.
    assert_ne!(
        output.status.code(),
        Some(2),
        "alef generate --format must be accepted by clap (not an unknown argument); got exit code 2"
    );
}

/// `alef all --format` must be accepted by clap (opt-in formatter flag).
#[test]
fn all_accepts_format_flag() {
    let output = Command::new(alef_binary())
        .args(["all", "--format"])
        .output()
        .expect("failed to spawn alef");

    // Exit code 2 is clap's "unknown argument" error.
    assert_ne!(
        output.status.code(),
        Some(2),
        "alef all --format must be accepted by clap (not an unknown argument); got exit code 2"
    );
}
