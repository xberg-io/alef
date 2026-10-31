//! Unsupported compatibility flags must remain rejected without promising work in CLI help.

use std::fs;
use std::process::{Command, Output};

fn run_verify(args: &[&str]) -> Output {
    let fixture = tempfile::tempdir().expect("temporary verification fixture");
    let config = "deliberately invalid TOML, never reached for unsupported flags\n";
    fs::write(fixture.path().join("alef.toml"), config).expect("write untouched configuration");
    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .current_dir(fixture.path())
        .arg("verify")
        .args(args)
        .output()
        .expect("execute real alef verification CLI");
    assert_eq!(
        fs::read_to_string(fixture.path().join("alef.toml")).expect("configuration remains readable"),
        config
    );
    assert_eq!(fs::read_dir(fixture.path()).expect("read fixture directory").count(), 1);
    output
}

fn assert_explicit_refusal(args: &[&str], flags: &[&str]) {
    let output = run_verify(args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("does not implement"), "wrong refusal reason: {stderr}");
    for flag in flags {
        assert!(stderr.contains(flag), "missing {flag} in {stderr}");
    }
    assert!(
        !stderr.contains("TOML parse error"),
        "configuration was examined before refusal: {stderr}"
    );
}

fn option_is_advertised(help: &str, flag: &str) -> bool {
    help.lines().any(|line| {
        line.trim_start()
            .strip_prefix(flag)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(char::is_whitespace))
    })
}

#[test]
fn verify_help_omits_all_unsupported_flags_but_keeps_report_only() {
    let output = run_verify(&["--help"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        option_is_advertised(&help, "--report-only"),
        "the real verification option disappeared: {help}"
    );
    for flag in ["--compile", "--lint", "--lang", "--exit-code"] {
        assert!(
            !option_is_advertised(&help, flag),
            "unsupported or deprecated {flag} is advertised: {help}"
        );
    }
}

#[test]
fn verify_compile_retains_explicit_runtime_rejection() {
    assert_explicit_refusal(&["--compile"], &["--compile"]);
}

#[test]
fn verify_lint_retains_explicit_runtime_rejection() {
    assert_explicit_refusal(&["--lint"], &["--lint"]);
}

#[test]
fn verify_python_filter_retains_explicit_runtime_rejection() {
    assert_explicit_refusal(&["--lang", "python"], &["--lang"]);
}

#[test]
fn verify_unknown_language_never_falls_back_to_full_verification() {
    assert_explicit_refusal(&["--lang", "not-a-language"], &["--lang"]);
}

#[test]
fn verify_empty_language_never_falls_back_to_full_verification() {
    let output = run_verify(&["--lang="]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--lang"),
        "empty language failed for the wrong reason: {stderr}"
    );
    assert!(
        !stderr.contains("TOML parse error"),
        "empty filter reached configuration loading: {stderr}"
    );
}

#[test]
fn verify_combined_unsupported_flags_names_each_requested_operation() {
    assert_explicit_refusal(
        &["--compile", "--lint", "--lang", "python"],
        &["--compile", "--lint", "--lang"],
    );
}
