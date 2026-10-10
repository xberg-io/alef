use std::path::PathBuf;

#[test]
fn published_alef_crate_excludes_repository_only_integration_tests() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read workspace manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("parse workspace manifest");
    let excludes = parsed["package"]["exclude"]
        .as_array()
        .expect("package.exclude must cap the published crate payload");

    assert!(
        excludes.iter().any(|entry| entry.as_str() == Some("tests/**")),
        "repository-only integration tests must not be uploaded to crates.io"
    );
}
