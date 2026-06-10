//! Format-preserving patch of `[workspace.lints.rust]` in the root `Cargo.toml`.
//!
//! Called during `alef scaffold` to add the `alef-meta` check-cfg allowlist so
//! that downstream crates can write
//! `#[cfg_attr(feature = "alef-meta", alef(since = "..."))]`
//! without declaring `alef-meta` as a real Cargo feature — which would cause
//! `cargo clippy --all-features` to activate the feature, invoke the (non-existent)
//! `alef` proc-macro, and fail with a hard compile error.
//!
//! The allowlist entry `cfg(feature, values("alef-meta"))` tells rustc 1.80+ that
//! `alef-meta` is a known cfg value, silencing `unexpected_cfg` warnings, while
//! keeping `alef-meta` out of `[features]` so `--all-features` never enables it.

use anyhow::Context as _;
use std::path::Path;

const CHECK_CFG_VALUE: &str = r#"cfg(feature, values("alef-meta"))"#;

/// Patch `[workspace.lints.rust]` in the root `Cargo.toml` to include
/// `unexpected_cfgs = { level = "warn", check-cfg = ['cfg(feature, values("alef-meta"))'] }`.
///
/// Reads from and writes to `./Cargo.toml` (the current working directory).
pub fn ensure_workspace_alef_meta_check_cfg() -> anyhow::Result<bool> {
    ensure_workspace_alef_meta_check_cfg_at(Path::new("Cargo.toml"))
}

/// Inner implementation that accepts an explicit path — used by tests to avoid
/// process-global `set_current_dir` races.
///
/// - Returns `true` when the file was modified.
/// - Returns `false` (without error) when:
///   - `cargo_toml` does not exist or cannot be read.
///   - The manifest has no `[workspace]` table (single-crate, not a workspace).
///   - `unexpected_cfgs` is already present in `[workspace.lints.rust]` (idempotent).
/// - Propagates errors only for parse or write failures.
fn ensure_workspace_alef_meta_check_cfg_at(cargo_toml: &Path) -> anyhow::Result<bool> {
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

    let content = match std::fs::read_to_string(cargo_toml) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    // Fast path: if "unexpected_cfgs" key is already anywhere in the file, skip parse.
    if content.contains("unexpected_cfgs") {
        return Ok(false);
    }

    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse {}", cargo_toml.display()))?;

    // Only workspace manifests need this patch.
    if !doc.contains_key("workspace") {
        return Ok(false);
    }

    let workspace_item = doc
        .get_mut("workspace")
        .context("[workspace] entry missing after containment check")?;
    // If [workspace] is not a table (inline table syntax), skip gracefully.
    let workspace_table = match workspace_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    // Create [workspace.lints] if absent; skip if it exists but is not a table (inline).
    let lints_item = workspace_table
        .entry("lints")
        .or_insert_with(|| Item::Table(Table::new()));
    let lints_table = match lints_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    // Create [workspace.lints.rust] if absent; skip if it exists but is not a table (inline).
    let rust_item = lints_table.entry("rust").or_insert_with(|| Item::Table(Table::new()));
    let rust_table = match rust_item.as_table_mut() {
        Some(t) => t,
        None => return Ok(false),
    };

    // Don't clobber an existing `unexpected_cfgs` entry — user may have customised it.
    if rust_table.contains_key("unexpected_cfgs") {
        return Ok(false);
    }

    // Build: unexpected_cfgs = { level = "warn", check-cfg = ['cfg(feature, values("alef-meta"))'] }
    let mut check_cfg_array = Array::new();
    check_cfg_array.push(CHECK_CFG_VALUE);

    let mut inline = InlineTable::new();
    inline.insert("level", Value::from("warn"));
    inline.insert("check-cfg", Value::Array(check_cfg_array));

    rust_table.insert("unexpected_cfgs", Item::Value(Value::InlineTable(inline)));

    std::fs::write(cargo_toml, doc.to_string()).with_context(|| format!("failed to write {}", cargo_toml.display()))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{CHECK_CFG_VALUE, ensure_workspace_alef_meta_check_cfg_at};
    use std::fs;
    use tempfile::TempDir;

    fn run(dir: &TempDir, content: &str) -> anyhow::Result<bool> {
        let path = dir.path().join("Cargo.toml");
        fs::write(&path, content).unwrap();
        ensure_workspace_alef_meta_check_cfg_at(&path)
    }

    fn read(dir: &TempDir) -> String {
        fs::read_to_string(dir.path().join("Cargo.toml")).unwrap()
    }

    #[test]
    fn skips_when_no_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("Cargo.toml");
        assert!(!ensure_workspace_alef_meta_check_cfg_at(&path).unwrap());
    }

    #[test]
    fn skips_single_crate_manifest() {
        let dir = TempDir::new().unwrap();
        let modified = run(&dir, "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n").unwrap();
        assert!(!modified, "single-crate manifest must not be modified");
    }

    #[test]
    fn patches_workspace_manifest_without_lints() {
        let dir = TempDir::new().unwrap();
        let modified = run(&dir, "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        assert!(modified, "should patch manifest that has no lints section");
        let written = read(&dir);
        assert!(
            written.contains(CHECK_CFG_VALUE),
            "must contain check-cfg value:\n{written}"
        );
        assert!(
            written.contains("unexpected_cfgs"),
            "must contain unexpected_cfgs key:\n{written}"
        );
    }

    #[test]
    fn idempotent_when_check_cfg_already_present() {
        let dir = TempDir::new().unwrap();
        let content = format!(
            "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunexpected_cfgs = {{ level = \"warn\", check-cfg = ['{CHECK_CFG_VALUE}'] }}\n"
        );
        let modified = run(&dir, &content).unwrap();
        assert!(!modified, "must not modify file that already has the check-cfg");
    }

    #[test]
    fn skips_when_unexpected_cfgs_key_exists_with_different_value() {
        let dir = TempDir::new().unwrap();
        let content = "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunexpected_cfgs = { level = \"deny\", check-cfg = ['cfg(frb_expand)'] }\n";
        let modified = run(&dir, content).unwrap();
        assert!(!modified, "must not touch existing unexpected_cfgs entry");
        assert!(read(&dir).contains("deny"), "existing entry must be preserved");
    }

    #[test]
    fn skips_gracefully_when_lints_is_inline_table() {
        let dir = TempDir::new().unwrap();
        // [workspace.lints] written as an inline table — as_table_mut() returns None.
        let content = "[workspace]\nmembers = []\nlints = { rust = { unsafe_code = \"forbid\" } }\n";
        let modified = run(&dir, content).unwrap();
        assert!(!modified, "must skip gracefully when lints is inline table");
        assert_eq!(read(&dir), content, "file must not be modified");
    }

    #[test]
    fn patches_workspace_with_existing_lints_rust_without_unexpected_cfgs() {
        let dir = TempDir::new().unwrap();
        let modified = run(
            &dir,
            "[workspace]\nmembers = []\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n",
        )
        .unwrap();
        assert!(modified, "should add unexpected_cfgs alongside existing lint entry");
        let written = read(&dir);
        assert!(
            written.contains(CHECK_CFG_VALUE),
            "must contain check-cfg value:\n{written}"
        );
        assert!(
            written.contains("unsafe_code"),
            "existing lint must be preserved:\n{written}"
        );
    }
}
