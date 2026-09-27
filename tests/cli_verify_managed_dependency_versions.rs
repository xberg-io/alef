//! `alef verify` has to say, on a tree it reports clean, that the dependency versions in the
//! manifests it owns are alef's and not the consumer's.
//!
//! The loss the finding warns about is silent and delayed: the consumer's `task upgrade`
//! succeeds, CI may go green on it, and the revert lands on whoever next runs `alef all`. A
//! notice that only appears on a failing run would never be seen, so this prints on a clean
//! one -- which is exactly why it must not touch the exit code. Both halves are asserted
//! here, against the real binary, because the defect would be in what the command reports.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

fn run_alef(root: &Path, args: &[&str]) -> Output {
    Command::new(alef_binary())
        .current_dir(root)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run alef {args:?}: {error}"))
}

fn context(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"managed-versions-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "managed-versions-fixture"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "managed_versions_fixture"

[crates.python.stubs]
output = "packages/python/managed_versions_fixture"
"#;

/// The exact heading `verify_informational::MANAGED_DEPENDENCY_VERSIONS_HEADING` prints.
///
/// Duplicated deliberately: the constant is private to the binary, and a test that imported
/// it could not fail if the wording were emptied. This literal is the contract.
const HEADING: &str = "Dependency versions in these generated manifests are alef-managed";

#[test]
fn verify_names_the_managed_manifests_without_failing_the_run() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");

    let generated = run_alef(&root, &["all", "--skip-frb"]);
    assert!(
        generated.status.success(),
        "alef all must succeed against the fixture.\n{}",
        context(&generated)
    );

    let output = run_alef(&root, &["verify"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains(HEADING),
        "verify must print the managed-dependency-version finding.\n{}",
        context(&output)
    );
    assert!(
        stdout.contains("[crates.python.dependency_versions]"),
        "the finding must name the real override table, or it tells the consumer nothing \
         actionable.\n{}",
        context(&output)
    );
    assert!(
        stdout.contains("pyproject.toml"),
        "the finding must name the manifests it is about.\n{}",
        context(&output)
    );
    // The contract most at risk. An informational finding that gates the exit code turns
    // every consumer's release gate red on the upgrade that adds it, for a condition no
    // rerun clears -- and a gate routed around reports nothing.
    assert!(
        output.status.success(),
        "the finding is informational: it must leave a clean tree exiting 0.\n{}",
        context(&output)
    );
}
