use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn hook_path() -> PathBuf {
    repo_root().join("hooks/check_project_mentions.py")
}

fn run_hook(files: &[&Path]) -> Output {
    let mut command = Command::new("python3");
    command.arg(hook_path());
    for file in files {
        command.arg(file);
    }
    command.output().expect("hook command must run")
}

fn forbidden(parts: &[&str], separator: &str) -> String {
    parts.join(separator)
}

#[test]
fn reports_case_insensitive_project_mentions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("code.rs");
    fs::write(
        &file,
        format!("const NAME: &str = {:?};\n", forbidden(&["Kreuz", "Berg"], "")),
    )
    .expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(!output.status.success(), "hook should reject forbidden mention");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("forbidden project mention"), "stderr: {stderr}");
    assert!(stderr.contains("Alef must stay project-agnostic"), "stderr: {stderr}");
}

#[test]
fn reports_dash_underscore_space_and_collapsed_variants() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dash = dir.path().join("dash.rs");
    let underscore = dir.path().join("underscore.rs");
    let spaced = dir.path().join("spaced.rs");
    let collapsed = dir.path().join("collapsed.rs");

    fs::write(&dash, forbidden(&["html", "to", "markdown"], "-")).expect("write dash fixture");
    fs::write(&underscore, forbidden(&["tree", "sitter", "language", "pack"], "_")).expect("write underscore fixture");
    fs::write(&spaced, forbidden(&["liter", "llm"], " ")).expect("write spaced fixture");
    fs::write(&collapsed, forbidden(&["ts", "pack"], "")).expect("write collapsed fixture");

    let output = run_hook(&[&dash, &underscore, &spaced, &collapsed]);

    assert!(!output.status.success(), "hook should reject all separator variants");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert_eq!(
        stderr.matches("forbidden project mention").count(),
        4,
        "stderr: {stderr}"
    );
}

#[test]
fn accepts_clean_generic_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("code.rs");
    fs::write(&file, "let package_name = config.package_name.clone();\n").expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn skips_prose_documentation_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("notes.md");
    fs::write(&file, forbidden(&["spik", "ard"], "")).expect("write fixture");

    let output = run_hook(&[&file]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reports_multiple_files_with_line_numbers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = dir.path().join("first.toml");
    let second = dir.path().join("second.snap");
    fs::write(&first, format!("name = {:?}\n", forbidden(&["h2", "m"], ""))).expect("write first fixture");
    fs::write(&second, format!("\n{}\n", forbidden(&["ll", "lm"], ""))).expect("write second fixture");

    let output = run_hook(&[&first, &second]);

    assert!(!output.status.success(), "hook should reject both files");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("first.toml:1:"), "stderr: {stderr}");
    assert!(stderr.contains("second.snap:2:"), "stderr: {stderr}");
}
