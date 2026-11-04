use super::*;
use crate::cli::commands::validate_versions::{checks_pass, run};
use crate::core::config::NewAlefConfig;
use crate::test_support::write_file;

const CANONICAL: &str = "1.2.3";

const GO_COMPONENT: &str = r#"
[[crates.components]]
name = "codec"
producer_language = "go"
provides = [{ contract = "codec", implementation = "NewCodec" }]
targets = ["x86_64-unknown-linux-gnu"]
producer_dir = "producers/go"
"#;

const RUST_COMPONENT: &str = r#"
[[crates.components]]
name = "pdf"
provides = [{ contract = "extractor", implementation = "demo::PdfExtractor" }]
features = ["pdf"]
default_features = false
targets = ["x86_64-unknown-linux-gnu"]
producer_dir = "producers/pdf"
"#;

fn fixture(components: &str) -> (tempfile::TempDir, ResolvedCrateConfig) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write_file(
        root,
        "Cargo.toml",
        &format!("[package]\nname = \"demo\"\nversion = \"{CANONICAL}\"\nedition = \"2024\"\n"),
    );
    let alef_toml = format!(
        r#"[workspace]

[[crates]]
name = "demo"
sources = []
version_from = "{}"

[[crates.component_contracts]]
name = "extractor"
trait_path = "demo::Extractor"
interface_version = 1

[[crates.component_contracts]]
name = "codec"
trait_path = "demo::Codec"
interface_version = 1
{components}"#,
        root.join("Cargo.toml").display().to_string().replace('\\', "/"),
    );
    let parsed: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let config = parsed.resolve().expect("resolve config").remove(0);
    (dir, config)
}

fn write_producer(root: &Path, version: &str, embedded: &str) {
    write_file(
        root,
        "producers/pdf/Cargo.toml",
        &format!("[package]\nname = \"demo-pdf-component\"\nversion = \"{version}\"\nedition = \"2024\"\n"),
    );
    write_file(
        root,
        "producers/pdf/src/lib.rs",
        &format!("{VERSION_CONSTANT_PREFIX}{embedded}\";\n"),
    );
}

#[test]
fn go_only_components_only_crate_verifies_clean_with_nothing_to_examine() {
    let (dir, config) = fixture(GO_COMPONENT);

    assert!(config.is_components_only());
    assert!(nothing_to_examine(&config));
    let checks = run(&config, dir.path(), false).expect("a Go-only crate must not fail the examined-0 guard");
    assert!(
        checks.is_empty(),
        "a Go producer carries no versioned manifest: {checks:?}"
    );
    let verified = crate::cli::pipeline::verify_versions(&config, dir.path()).expect("verify path agrees");
    assert!(verified.is_empty());
}

#[test]
fn mixed_crate_checks_only_the_rust_producer() {
    let (dir, config) = fixture(&format!("{RUST_COMPONENT}{GO_COMPONENT}"));
    write_producer(dir.path(), CANONICAL, CANONICAL);

    assert!(!nothing_to_examine(&config));
    let checks = collect(&config, dir.path(), CANONICAL);
    let labels: Vec<_> = checks.iter().map(|check| check.label.as_str()).collect();
    assert_eq!(labels, ["producers/pdf/Cargo.toml", "producers/pdf/src/lib.rs"]);
    assert!(checks_pass(&checks), "{checks:?}");
}

#[test]
fn deleted_producer_manifest_is_a_failing_check_not_a_dropped_one() {
    let (dir, config) = fixture(RUST_COMPONENT);
    write_producer(dir.path(), CANONICAL, CANONICAL);
    std::fs::remove_file(dir.path().join("producers/pdf/Cargo.toml")).expect("remove manifest");

    let checks = collect(&config, dir.path(), CANONICAL);
    let manifest = checks
        .iter()
        .find(|check| check.label == "producers/pdf/Cargo.toml")
        .expect("the missing manifest must still produce a check");
    assert!(!manifest.matches);
    assert_eq!(manifest.found, None);
    assert!(!checks_pass(&checks));
}

#[test]
fn drifted_producer_version_fails() {
    let (dir, config) = fixture(RUST_COMPONENT);
    write_producer(dir.path(), "1.2.2", CANONICAL);

    let checks = collect(&config, dir.path(), CANONICAL);
    let manifest = checks.iter().find(|check| check.label.ends_with("Cargo.toml")).unwrap();
    assert!(!manifest.matches);
    assert_eq!(manifest.found.as_deref(), Some("1.2.2"));
    assert!(!checks_pass(&checks));
}

#[test]
fn drifted_embedded_component_version_fails() {
    let (dir, config) = fixture(RUST_COMPONENT);
    write_producer(dir.path(), CANONICAL, "1.2.2");

    let checks = collect(&config, dir.path(), CANONICAL);
    let lib = checks.iter().find(|check| check.label.ends_with("lib.rs")).unwrap();
    assert!(!lib.matches);
    assert_eq!(lib.found.as_deref(), Some("1.2.2"));
    assert!(!checks_pass(&checks));
}
