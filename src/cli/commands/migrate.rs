//! Migrate legacy single-crate alef.toml to new multi-crate `[workspace]` / `[[crates]]` schema.
//!
//! This module converts old-style configs (with a single `[crate]` table and flat top-level
//! language sections) to the new 0.13 schema where crate-specific settings live under
//! `[[crates]]` entries and workspace defaults live under `[workspace]`.

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use toml_edit::{ArrayOfTables, DocumentMut, Item, table};

/// Options for the migrate subcommand.
///
/// `write == false` is the default (dry-run mode prints a diff to stdout).
/// `write == true` is the explicit `--write` flag; only this mode mutates
/// the alef.toml on disk.
pub struct MigrateOptions {
    /// Path to the alef.toml file to migrate.
    pub path: PathBuf,
    /// When true, rewrite `path` in place. When false (the default), print
    /// a unified diff to stdout and leave the file untouched.
    pub write: bool,
}

/// Top-level keys that must move under [[crates]]
const CRATE_SCOPED_KEYS: &[&str] = &[
    "python",
    "node",
    "ruby",
    "php",
    "elixir",
    "wasm",
    "ffi",
    "gleam",
    "go",
    "java",
    "dart",
    "kotlin",
    "swift",
    "csharp",
    "r",
    "zig",
    "output",
    "exclude",
    "include",
    "lint",
    "update",
    "test",
    "setup",
    "clean",
    "build_commands",
    "publish",
    "e2e",
    "scaffold",
    "readme",
    "custom_files",
    "custom_modules",
    "custom_registrations",
];

/// Array-of-tables keys that must move under [[crates]]
const CRATE_SCOPED_ARRAY_KEYS: &[&str] = &["adapters", "trait_bridges"];

/// Top-level keys that must move under [workspace]
const WORKSPACE_SCOPED_KEYS: &[&str] = &[
    "tools",
    "dto",
    "format",
    "format_overrides",
    "generate",
    "generate_overrides",
    "output_template",
    "opaque_types",
    "sync",
];

pub fn run(options: MigrateOptions) -> Result<()> {
    let content =
        std::fs::read_to_string(&options.path).with_context(|| format!("Failed to read {}", options.path.display()))?;

    let mut doc = content.parse::<DocumentMut>().with_context(|| "Failed to parse TOML")?;

    if doc.get("workspace").is_some() || doc.get("crates").is_some() {
        return Err(anyhow!(
            "Config already uses new schema (found [workspace] or [[crates]]). Skipping migration."
        ));
    }

    let mut workspace_table = table();

    let legacy_crate = doc.remove("crate");

    if let Some(version) = doc.remove("version") {
        if let Some(ws_tbl) = workspace_table.as_table_mut() {
            ws_tbl["alef_version"] = version;
        }
    }

    if let Some(languages) = doc.remove("languages") {
        if let Some(ws_tbl) = workspace_table.as_table_mut() {
            ws_tbl["languages"] = languages;
        }
    }

    for key in WORKSPACE_SCOPED_KEYS {
        if let Some(value) = doc.remove(key) {
            if let Some(ws_tbl) = workspace_table.as_table_mut() {
                ws_tbl[key] = value;
            }
        }
    }

    let had_legacy_crate = legacy_crate.is_some();
    let mut crate_table = table();
    if let Some(legacy_item) = &legacy_crate {
        if let Some(legacy_tbl) = legacy_item.as_table() {
            if let Some(cr_tbl) = crate_table.as_table_mut() {
                copy_table_into(legacy_tbl, cr_tbl);
            }
        }
    }

    for key in CRATE_SCOPED_KEYS {
        if let Some(value) = doc.remove(key) {
            if let Some(cr_tbl) = crate_table.as_table_mut() {
                cr_tbl.insert(key, strip_position(value));
            }
        }
    }

    for key in CRATE_SCOPED_ARRAY_KEYS {
        if let Some(value) = doc.remove(key) {
            if let Some(cr_tbl) = crate_table.as_table_mut() {
                cr_tbl.insert(key, strip_position(value));
            }
        }
    }

    let mut workspace_count = 0;
    let mut crate_count = 0;

    let ws_inner = workspace_table.as_table().expect("workspace_table is a Table");
    if ws_inner.contains_key("alef_version") {
        workspace_count += 1;
    }
    if ws_inner.contains_key("languages") {
        workspace_count += 1;
    }
    for key in WORKSPACE_SCOPED_KEYS {
        if ws_inner.contains_key(key) {
            workspace_count += 1;
        }
    }

    let cr_inner = crate_table.as_table().expect("crate_table is a Table");
    if had_legacy_crate {
        crate_count += 1;
    }
    for key in CRATE_SCOPED_KEYS {
        if cr_inner.contains_key(key) {
            crate_count += 1;
        }
    }
    for key in CRATE_SCOPED_ARRAY_KEYS {
        if cr_inner.contains_key(key) {
            crate_count += 1;
        }
    }

    if let Some(ws_tbl) = workspace_table.as_table() {
        if !ws_tbl.is_empty() {
            doc["workspace"] = workspace_table;
        }
    }

    let mut crates_array = ArrayOfTables::new();
    let crate_inner = crate_table
        .into_table()
        .map_err(|_| anyhow!("internal: crate_table was not a table"))?;
    crates_array.push(crate_inner);
    doc["crates"] = Item::ArrayOfTables(crates_array);

    let migrated_content = doc.to_string();

    if options.write {
        atomic_write(&options.path, &migrated_content)?;
        eprintln!("Migrated {} ✓", options.path.display());
    } else {
        print_diff(&content, &migrated_content)?;
    }
    eprintln!("Moved {workspace_count} key(s) to [workspace], {crate_count} key(s) to [[crates]]");

    Ok(())
}

/// Cap on the number of diff lines streamed in dry-run mode. Past this point
/// we print a truncation marker rather than flooding the terminal — `--write`
/// is the right tool for inspecting the full output.
const MAX_DIFF_LINES: usize = 200;

/// Copy every entry from `src` into `dst`, in iteration order, with all
/// position metadata cleared. Position metadata is what toml_edit uses to
/// preserve a value's location when re-serializing — copying it across into
/// a new table layout would cause the resulting TOML to be reordered to match
/// the *original* document (which can produce malformed output when scalar
/// fields end up after sub-tables in the new layout).
fn copy_table_into(src: &toml_edit::Table, dst: &mut toml_edit::Table) {
    for (k, v) in src.iter() {
        dst.insert(k, strip_position(v.clone()));
    }
}

/// Recursively clear position AND decor metadata on a TOML item so toml_edit
/// serializes it in insertion order rather than reproducing its position from
/// the source document. Walks into nested tables and arrays of tables.
fn strip_position(mut item: toml_edit::Item) -> toml_edit::Item {
    match &mut item {
        toml_edit::Item::Value(v) => {
            v.decor_mut().clear();
        }
        toml_edit::Item::Table(t) => {
            t.set_position(None);
            t.decor_mut().clear();
            let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
            for k in keys {
                if let Some(child) = t.remove(&k) {
                    t.insert(&k, strip_position(child));
                }
            }
        }
        toml_edit::Item::ArrayOfTables(arr) => {
            for sub in arr.iter_mut() {
                sub.set_position(None);
                sub.decor_mut().clear();
                let keys: Vec<String> = sub.iter().map(|(k, _)| k.to_string()).collect();
                for k in keys {
                    if let Some(child) = sub.remove(&k) {
                        sub.insert(&k, strip_position(child));
                    }
                }
            }
        }
        _ => {}
    }
    item
}

/// Write `content` to `dest` atomically, rejecting symlink targets.
///
/// Writes to a sibling temp file then renames it into place so a crash
/// mid-write cannot corrupt the original. Symlinks are rejected because
/// a rename over a symlink would silently redirect to the link's target.
fn atomic_write(dest: &std::path::Path, content: &str) -> Result<()> {
    let meta = dest.symlink_metadata();
    if let Ok(m) = meta {
        if m.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to overwrite symlink at {}; resolve the symlink first",
                dest.display()
            ));
        }
    }

    let parent = dest
        .parent()
        .ok_or_else(|| anyhow!("cannot determine parent directory of {}", dest.display()))?;

    let tmp_path = parent.join(format!(
        ".{}.migrate.tmp.{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("alef.toml"),
        std::process::id()
    ));

    let write_result =
        std::fs::write(&tmp_path, content).with_context(|| format!("failed to write temp file {}", tmp_path.display()));

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) =
        std::fs::rename(&tmp_path, dest).with_context(|| format!("failed to rename temp file to {}", dest.display()))
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}

fn print_diff(original: &str, migrated: &str) -> Result<()> {
    let diff = similar::TextDiff::from_lines(original, migrated);

    println!("--- alef.toml (original)");
    println!("+++ alef.toml (migrated)");

    for (idx, change) in diff.iter_all_changes().enumerate() {
        if idx >= MAX_DIFF_LINES {
            println!("... (diff truncated after {MAX_DIFF_LINES} lines; rerun with --write to apply) ...");
            break;
        }
        let prefix = match change.tag() {
            similar::ChangeTag::Delete => '-',
            similar::ChangeTag::Insert => '+',
            similar::ChangeTag::Equal => ' ',
        };
        print!("{prefix}{change}", change = change.value());
    }

    println!();
    println!("Run with --write to apply this migration.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn migrate_toml(input: &str, write: bool) -> Result<String> {
        let dir = TempDir::new()?;
        let path = dir.path().join("alef.toml");
        fs::write(&path, input)?;

        let options = MigrateOptions {
            path: path.clone(),
            write,
        };
        run(options)?;

        Ok(fs::read_to_string(path)?)
    }

    #[test]
    fn test_migrate_promotes_crate_to_array() -> Result<()> {
        let input = r#"
[crate]
name = "foo"
sources = []
"#;

        let output = migrate_toml(input, true)?;
        assert!(
            output.contains("[[crates]]"),
            "expected `[[crates]]` (array-of-tables), got:\n{output}"
        );
        assert!(output.contains("name = \"foo\""));
        assert!(!output.contains("[crate]\n"), "leftover singular [crate] section");
        let parsed: toml::Value = toml::from_str(&output)?;
        let crates = parsed
            .get("crates")
            .and_then(|v| v.as_array())
            .expect("`crates` must be an array of tables");
        assert_eq!(crates.len(), 1, "expected exactly one crate");
        assert_eq!(crates[0].get("name").and_then(|v| v.as_str()), Some("foo"));
        Ok(())
    }

    #[test]
    fn test_migrate_moves_python_under_crate() -> Result<()> {
        let input = r#"
[crate]
name = "sample_router"
sources = []

[python]
module_name = "_sample_router"
"#;

        let output = migrate_toml(input, true)?;
        assert!(output.contains("[crates") && output.contains("python"));
        assert!(!output.contains("[python]") || output.contains("crates"));
        assert!(output.contains("module_name = \"_sample_router\""));
        Ok(())
    }

    #[test]
    fn test_migrate_moves_lint_under_crate() -> Result<()> {
        let input = r#"
[crate]
name = "sample_router"
sources = []

[lint.python]
check = "ruff"
"#;

        let output = migrate_toml(input, true)?;
        assert!(output.contains("[crates") || output.contains("crates.lint"));
        Ok(())
    }

    #[test]
    fn test_migrate_moves_tools_under_workspace() -> Result<()> {
        let input = r#"
[crate]
name = "sample_router"
sources = []

[tools]
python_pkg_manager = "uv"
"#;

        let output = migrate_toml(input, true)?;
        assert!(output.contains("[workspace]"));
        assert!(output.contains("tools") || output.contains("workspace.tools"));
        assert!(output.contains("python_pkg_manager = \"uv\""));
        Ok(())
    }

    #[test]
    fn test_migrate_renames_version_to_alef_version() -> Result<()> {
        let input = r#"
version = "0.13.0"

[crate]
name = "sample_router"
sources = []
"#;

        let output = migrate_toml(input, true)?;
        assert!(output.contains("alef_version = \"0.13.0\""));
        for line in output.lines() {
            let trimmed = line.trim_start();
            assert!(
                !trimmed.starts_with("version =") && !trimmed.starts_with("version="),
                "leftover top-level version line: {line:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_migrate_moves_adapters_array_under_crates() -> Result<()> {
        let input = r#"
[crate]
name = "sample_router"
sources = []

[[adapters]]
core_path = "sample_router::handle_request"

[[adapters]]
core_path = "sample_router::shutdown"
"#;
        let output = migrate_toml(input, true)?;
        let parsed: toml::Value = toml::from_str(&output)?;
        let crates = parsed
            .get("crates")
            .and_then(|v| v.as_array())
            .expect("`crates` must be an array of tables");
        let adapters = crates[0]
            .get("adapters")
            .and_then(|v| v.as_array())
            .expect("`crates[0].adapters` must be an array");
        assert_eq!(adapters.len(), 2);
        assert_eq!(
            adapters[0].get("core_path").and_then(|v| v.as_str()),
            Some("sample_router::handle_request")
        );
        assert!(parsed.get("adapters").is_none(), "leftover top-level [[adapters]]");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_rejects_symlink_target() -> Result<()> {
        let dir = TempDir::new()?;
        let real_file = dir.path().join("real.toml");
        let link = dir.path().join("alef.toml");
        fs::write(&real_file, "original")?;
        std::os::unix::fs::symlink(&real_file, &link)?;

        let result = atomic_write(&link, "new content");
        assert!(result.is_err(), "atomic_write should refuse to overwrite a symlink");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("symlink"),
            "error message should mention symlink, got: {err}"
        );
        assert_eq!(fs::read_to_string(&real_file)?, "original");
        Ok(())
    }

    #[test]
    fn test_migrate_rejects_already_migrated() -> Result<()> {
        let input = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "foo"
sources = []
"#;

        let dir = TempDir::new()?;
        let path = dir.path().join("alef.toml");
        fs::write(&path, input)?;

        let options = MigrateOptions { path, write: false };
        let result = run(options);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already uses new schema"));
        Ok(())
    }
}
