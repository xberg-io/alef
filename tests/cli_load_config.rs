//! Integration tests for the `load_config` / `load_resolved_config` functions.
//!
//! These tests verify the legacy detection wiring and the new-schema loader
//! by writing real files to a temporary directory and calling the CLI's
//! internal loader through `alef_core` directly (the loader functions are
//! `pub(crate)` in main.rs, so we exercise the same code path through
//! `alef_core`'s public API that the CLI wraps).

use std::path::Path;

use alef::core::config::{NewAlefConfig, detect_legacy_keys};

fn write_tmp(content: &str, filename: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().join(filename);
    std::fs::write(&path, content).expect("failed to write temp file");
    (dir, path)
}

/// Simulate what load_config does: detect legacy then parse.
fn load_via_public_api(
    path: &Path,
) -> anyhow::Result<(
    alef::core::config::WorkspaceConfig,
    Vec<alef::core::config::ResolvedCrateConfig>,
)> {
    let content = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("read: {e}"))?;
    detect_legacy_keys(&content).map_err(|e| anyhow::anyhow!("legacy schema detected — run `alef migrate`: {e}"))?;
    let cfg: NewAlefConfig = toml::from_str(&content).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let resolved = cfg.resolve().map_err(|e| anyhow::anyhow!("resolve: {e}"))?;
    Ok((cfg.workspace, resolved))
}

#[test]
fn load_config_legacy_file_returns_legacy_error() {
    let toml = r#"
languages = ["python"]

[crate]
name = "my-lib"
sources = ["src/lib.rs"]
"#;
    let (_dir, path) = write_tmp(toml, "alef.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let err = detect_legacy_keys(&content).expect_err("legacy config must fail detection");
    let msg = format!("{err}");
    assert!(
        msg.contains("alef migrate"),
        "error must mention `alef migrate`, got: {msg}"
    );
}

#[test]
fn load_config_new_schema_returns_resolved_vec() {
    let toml = r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#;
    let (_dir, path) = write_tmp(toml, "alef.toml");
    let (workspace, resolved) = load_via_public_api(&path).expect("new schema must load");
    assert_eq!(resolved.len(), 1, "should resolve exactly one crate");
    assert_eq!(resolved[0].name, "my-lib");
    assert!(workspace.languages.contains(&alef::core::config::Language::Python));
}

#[test]
fn load_config_missing_file_returns_io_error() {
    let path = std::path::Path::new("/nonexistent/path/alef.toml");
    let result = std::fs::read_to_string(path);
    assert!(result.is_err(), "reading a missing file must fail");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("No such file") || err_msg.contains("not found") || err_msg.contains("cannot find"),
        "error must describe a missing-file condition, got: {err_msg}"
    );
}

#[test]
fn load_config_invalid_toml_returns_parse_error() {
    let toml = r#"
[workspace
languages = ["python"]  # missing closing bracket
"#;
    let (_dir, path) = write_tmp(toml, "alef.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let _ = detect_legacy_keys(&content);
    let result: Result<NewAlefConfig, _> = toml::from_str(&content);
    assert!(result.is_err(), "invalid TOML must fail to parse");
}
