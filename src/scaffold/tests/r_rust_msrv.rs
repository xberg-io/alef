use super::*;
use std::fs;

fn r_config(root: &Path, source: &str) -> ResolvedCrateConfig {
    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    config.sources = vec![source.to_string()];
    config
}

fn write_manifest(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn r_requirement(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let files = scaffold(&test_api(), config, &[Language::R])?;
    let description = files.iter().find(|file| file.path.ends_with("DESCRIPTION")).unwrap();
    Ok(description
        .content
        .lines()
        .find(|line| line.starts_with("SystemRequirements:"))
        .unwrap()
        .to_string())
}

fn assert_requirement(config: &ResolvedCrateConfig, version: &str) {
    assert_eq!(
        r_requirement(config).unwrap(),
        format!("SystemRequirements: Cargo (Rust's package manager), rustc (>= {version})")
    );
}

#[test]
fn r_description_uses_bound_crate_msrv_with_bridge_floor() {
    let dir = tempfile::tempdir().unwrap();
    let config = r_config(dir.path(), "src/lib.rs");
    for (field, expected) in [
        ("rust-version = \"1.100\"", "1.100"),
        ("rust-version = \"1.97\"", "1.99"),
        ("", "1.99"),
    ] {
        write_manifest(
            dir.path(),
            "Cargo.toml",
            &format!("[package]\nname = \"core\"\n{field}\n"),
        );
        assert_requirement(&config, expected);
    }
}

#[test]
fn r_description_inherits_workspace_msrv_for_nested_sources() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(
        dir.path(),
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/core\"]\n[workspace.package]\nrust-version = \"1.100\"\n",
    );
    write_manifest(
        dir.path(),
        "crates/core/Cargo.toml",
        "[package]\nname = \"core\"\nrust-version.workspace = true\n",
    );
    assert_requirement(&r_config(dir.path(), "crates/core/src/nested/lib.rs"), "1.100");
}

#[test]
fn r_description_explicit_msrv_overrides_workspace_before_bridge_floor() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(
        dir.path(),
        "Cargo.toml",
        "[workspace.package]\nrust-version = \"1.100\"\n",
    );
    write_manifest(
        dir.path(),
        "crates/core/Cargo.toml",
        "[package]\nname = \"core\"\nrust-version = \"1.97\"\n",
    );
    assert_requirement(&r_config(dir.path(), "crates/core/src/lib.rs"), "1.99");
}

#[test]
fn r_description_inherits_root_package_msrv() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"core\"\nrust-version.workspace = true\n[workspace.package]\nrust-version = \"1.100\"\n",
    );
    assert_requirement(&r_config(dir.path(), "src/lib.rs"), "1.100");
}

#[test]
fn r_description_resolves_explicit_workspace_path() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(
        dir.path(),
        "workspace/Cargo.toml",
        "[workspace.package]\nrust-version = \"1.100\"\n",
    );
    write_manifest(
        dir.path(),
        "crates/core/Cargo.toml",
        "[package]\nname = \"core\"\nworkspace = \"../../workspace\"\nrust-version.workspace = true\n",
    );
    assert_requirement(&r_config(dir.path(), "crates/core/src/lib.rs"), "1.100");
}

#[test]
fn r_description_ignores_unrelated_version_manifest_and_missing_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(dir.path(), "Cargo.toml", "[package]\nrust-version = \"1.120\"\n");
    let mut config = r_config(dir.path(), "crates/core/src/lib.rs");
    config.version_from = "Cargo.toml".to_string();
    assert_requirement(&config, "1.99");
    write_manifest(
        dir.path(),
        "crates/core/Cargo.toml",
        "[package]\nrust-version = \"1.100\"\n",
    );
    assert_requirement(&config, "1.100");
}

#[test]
fn r_description_rejects_invalid_bound_manifest_with_path() {
    let dir = tempfile::tempdir().unwrap();
    let config = r_config(dir.path(), "src/lib.rs");
    for body in [
        "[package",
        "[package]\nrust-version = 100",
        "[package]\nrust-version = \"latest\"",
        "[package]\nrust-version.workspace = false",
        "[package]\nrust-version.workspace = true",
    ] {
        write_manifest(dir.path(), "Cargo.toml", body);
        let error = r_requirement(&config).expect_err("invalid MSRV must fail scaffold");
        assert!(format!("{error:#}").contains(&dir.path().join("Cargo.toml").display().to_string()));
    }
}

#[test]
fn r_description_rejects_broken_explicit_workspace_inheritance() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(
        dir.path(),
        "crates/core/Cargo.toml",
        "[package]\nworkspace = \"../../missing\"\nrust-version.workspace = true\n",
    );
    let config = r_config(dir.path(), "crates/core/src/lib.rs");
    let error = r_requirement(&config).expect_err("missing explicit workspace must fail scaffold");
    assert!(format!("{error:#}").contains("Cargo.toml"));
}
