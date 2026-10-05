use super::*;

fn ffi_manifest(ffi_toml: &str, core_features: Option<&str>) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = test_config_from_toml(&format!("[crates.ffi]\n{ffi_toml}\n"));
    config.workspace_root = Some(dir.path().to_path_buf());
    config.sources = vec![PathBuf::from("crates/my-lib/src/lib.rs")];
    if let Some(features) = core_features {
        let crate_dir = dir.path().join("crates/my-lib");
        std::fs::create_dir_all(&crate_dir).expect("core crate directory");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!("[package]\nname = \"my-lib\"\nversion = \"0.1.0\"\n\n[features]\n{features}\n"),
        )
        .expect("core Cargo.toml");
    }
    let files = scaffold(&test_api(), &config, &[Language::Ffi]).expect("ffi scaffold");
    files
        .into_iter()
        .find(|file| crate::test_support::portable_path_string(&file.path).ends_with("-ffi/Cargo.toml"))
        .expect("ffi Cargo.toml is scaffolded")
        .content
}

fn line_starting_with<'a>(manifest: &'a str, prefix: &str) -> &'a str {
    manifest
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{manifest}"))
}

const CORE: &str = "native-http = []\nfull = []\nwasm-http = []\npdf = []\ndefault = [\"native-http\"]";

#[test]
fn core_dependency_carries_no_forced_features_and_the_ffi_default_forwards_them() {
    let manifest = ffi_manifest(
        "features = [\"native-http\", \"full\"]\nextra_features = [\"wasm-http\"]",
        Some(CORE),
    );
    let dep = line_starting_with(&manifest, "my-lib = {");
    assert!(dep.contains("default-features = false"), "got {dep}");
    assert!(
        !dep.contains("features = ["),
        "no forced features on the dependency: {dep}"
    );
    assert_eq!(
        line_starting_with(&manifest, "default = ["),
        r#"default = ["native-http", "full"]"#,
        "core's own default `native-http` is already forwarded, so nothing is added"
    );
    assert!(manifest.contains(r#"native-http = ["my-lib/native-http"]"#));
    assert!(
        manifest.contains(r#"wasm-http = ["my-lib/wasm-http"]"#),
        "extra_features stay declared but off by default"
    );
}

#[test]
fn core_default_features_the_ffi_crate_does_not_list_are_forwarded_so_none_are_lost() {
    let manifest = ffi_manifest(
        "features = [\"full\"]",
        Some("full = []\npdf = []\nocr = []\ndefault = [\"pdf\", \"ocr\"]"),
    );
    assert_eq!(
        line_starting_with(&manifest, "default = ["),
        r#"default = ["full", "my-lib/pdf", "my-lib/ocr"]"#
    );
    assert!(line_starting_with(&manifest, "my-lib = {").contains("default-features = false"));
}

#[test]
fn excluded_and_extra_features_are_never_forwarded_from_the_core_default() {
    let manifest = ffi_manifest(
        "features = [\"full\"]\nextra_features = [\"wasm-http\"]\nexcluded_default_features = [\"pdf\"]",
        Some("full = []\npdf = []\nwasm-http = []\nocr = []\ndefault = [\"pdf\", \"wasm-http\", \"ocr\"]"),
    );
    assert_eq!(
        line_starting_with(&manifest, "default = ["),
        r#"default = ["full", "my-lib/ocr"]"#
    );
}

#[test]
fn serde_stays_on_the_dependency() {
    let manifest = ffi_manifest("features = [\"serde\", \"full\"]", Some("full = []\nserde = []"));
    let dep = line_starting_with(&manifest, "my-lib = {");
    assert!(
        dep.contains(r#"default-features = false, features = ["serde"]"#),
        "got {dep}"
    );
}

#[test]
fn unreadable_core_manifest_keeps_the_historical_dependency_line() {
    let manifest = ffi_manifest("features = [\"native-http\", \"full\"]", None);
    let dep = line_starting_with(&manifest, "my-lib = {");
    assert!(dep.contains(r#"features = ["native-http", "full"]"#), "got {dep}");
    assert!(!dep.contains("default-features"), "got {dep}");
}

#[test]
fn core_default_that_cannot_be_forwarded_keeps_the_historical_dependency_line() {
    let manifest = ffi_manifest(
        "features = [\"full\"]",
        Some("full = []\ndefault = [\"dep:thing\", \"reqwest/http2\"]"),
    );
    let dep = line_starting_with(&manifest, "my-lib = {");
    assert!(dep.contains(r#"features = ["full"]"#), "got {dep}");
    assert!(!dep.contains("default-features"), "got {dep}");
    assert_eq!(line_starting_with(&manifest, "default = ["), r#"default = ["full"]"#);
}

#[test]
fn target_dep_overrides_keep_the_explicit_default_branch() {
    let manifest = ffi_manifest(
        "features = [\"full\"]\n[[crates.ffi.target_dep_overrides]]\ncfg = \"target_os = \\\"android\\\"\"\ndefault_features = false\nfeatures = [\"full\"]",
        Some("full = []\npdf = []\ndefault = [\"pdf\"]"),
    );
    assert!(
        manifest.contains(r#"my-lib = { path = "../my-lib", version = "0.1.0", features = ["full"] }"#),
        "the non-override branch keeps its explicit features:\n{manifest}"
    );
    assert_eq!(line_starting_with(&manifest, "default = ["), r#"default = ["full"]"#);
}
