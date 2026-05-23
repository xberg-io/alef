//! Core crate vendoring — copies a Rust crate into a language package and
//! rewrites `Cargo.toml` to remove workspace inheritance.
//!
//! Two modes:
//! - **Core-only** (`VendorMode::CoreOnly`): copies only the core crate,
//!   inlines workspace fields and dependency specs. Used by Ruby and Elixir.
//! - **Full** (`VendorMode::Full`): core-only + `cargo vendor` of all
//!   transitive deps. Used by R/CRAN packages.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

/// Result of a vendoring operation.
pub struct VendorResult {
    /// Path to the vendored crate directory.
    pub vendor_dir: PathBuf,
    /// Path to the generated workspace Cargo.toml (if created).
    pub workspace_manifest: Option<PathBuf>,
}

/// Vendor just the core crate: copy source, rewrite Cargo.toml, generate workspace manifest.
pub fn vendor_core_only(
    workspace_root: &Path,
    core_crate_dir: &Path,
    dest_dir: &Path,
    generate_workspace_manifest: bool,
) -> Result<VendorResult> {
    let core_crate_name = core_crate_dir
        .file_name()
        .context("core crate dir has no name")?
        .to_string_lossy();

    // 1. Read workspace metadata from the root Cargo.toml.
    let workspace_manifest_path = workspace_root.join("Cargo.toml");
    let workspace_toml = fs::read_to_string(&workspace_manifest_path)
        .with_context(|| format!("reading {}", workspace_manifest_path.display()))?;
    let workspace_doc: DocumentMut = workspace_toml
        .parse()
        .with_context(|| format!("parsing {}", workspace_manifest_path.display()))?;

    let workspace_table = workspace_doc
        .get("workspace")
        .and_then(|w| w.as_table())
        .context("no [workspace] in root Cargo.toml")?;

    let workspace_pkg = workspace_table.get("package").and_then(|p| p.as_table());
    let workspace_deps = workspace_table.get("dependencies").and_then(|d| d.as_table());

    // 2. Clean and copy the crate.
    let vendor_crate_dir = dest_dir.join(&*core_crate_name);
    if vendor_crate_dir.exists() {
        fs::remove_dir_all(&vendor_crate_dir).with_context(|| format!("removing {}", vendor_crate_dir.display()))?;
    }
    copy_dir_filtered(core_crate_dir, &vendor_crate_dir)?;

    // 3. Rewrite the vendored Cargo.toml.
    let crate_manifest_path = vendor_crate_dir.join("Cargo.toml");
    let crate_toml = fs::read_to_string(&crate_manifest_path)
        .with_context(|| format!("reading {}", crate_manifest_path.display()))?;
    let mut crate_doc: DocumentMut = crate_toml
        .parse()
        .with_context(|| format!("parsing {}", crate_manifest_path.display()))?;

    // 3a. Inline [package] fields that use workspace inheritance.
    if let Some(ws_pkg) = workspace_pkg {
        inline_workspace_package_fields(&mut crate_doc, ws_pkg)?;
    }

    // 3b. Inline workspace dependencies.
    if let Some(ws_deps) = workspace_deps {
        inline_workspace_deps(&mut crate_doc, "dependencies", ws_deps)?;
        inline_workspace_deps(&mut crate_doc, "dev-dependencies", ws_deps)?;
        inline_workspace_deps(&mut crate_doc, "build-dependencies", ws_deps)?;
    }

    // 3c. Remove [lints] workspace = true.
    remove_workspace_lints(&mut crate_doc);

    fs::write(&crate_manifest_path, crate_doc.to_string())
        .with_context(|| format!("writing {}", crate_manifest_path.display()))?;

    // 4. Optionally generate a workspace Cargo.toml.
    let ws_manifest_path = if generate_workspace_manifest {
        let path = dest_dir.join("Cargo.toml");
        let content = generate_vendor_workspace_manifest(&core_crate_name, workspace_pkg, workspace_deps);
        fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    tracing::info!(
        crate_name = %core_crate_name,
        dest = %vendor_crate_dir.display(),
        "vendored core crate"
    );

    Ok(VendorResult {
        vendor_dir: vendor_crate_dir,
        workspace_manifest: ws_manifest_path,
    })
}

/// Vendor with full transitive dependencies: core-only + cargo vendor.
pub fn vendor_full(workspace_root: &Path, core_crate_dir: &Path, dest_dir: &Path) -> Result<VendorResult> {
    let result = vendor_core_only(workspace_root, core_crate_dir, dest_dir, true)?;

    // Run `cargo vendor` in the vendor workspace to download all transitive deps.
    let vendor_deps_dir = dest_dir.join("vendor");
    let status = std::process::Command::new("cargo")
        .arg("vendor")
        .arg(&vendor_deps_dir)
        .current_dir(dest_dir)
        .status()
        .context("running cargo vendor")?;

    if !status.success() {
        bail!("cargo vendor failed with exit code {}", status.code().unwrap_or(-1));
    }

    // Generate .cargo/config.toml to use vendored deps.
    let cargo_config_dir = dest_dir.join(".cargo");
    fs::create_dir_all(&cargo_config_dir)?;
    let config_content = format!(
        "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n\
         [source.vendored-sources]\ndirectory = \"{}\"\n",
        vendor_deps_dir.display()
    );
    fs::write(cargo_config_dir.join("config.toml"), config_content)?;

    // Clean up test/bench/doc files from vendored deps to reduce size.
    clean_vendored_deps(&vendor_deps_dir)?;

    tracing::info!(dest = %dest_dir.display(), "full vendor complete");
    Ok(result)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Copy a directory recursively, skipping `target/`, `.git/`, and temp files.
fn copy_dir_filtered(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip build artifacts, VCS, and temp files.
        if matches!(name_str.as_ref(), "target" | ".git" | ".gitignore" | ".fastembed_cache") {
            continue;
        }
        if name_str.ends_with(".swp")
            || name_str.ends_with(".bak")
            || name_str.ends_with(".tmp")
            || name_str.ends_with('~')
        {
            continue;
        }

        let src_path = entry.path();
        let dest_path = dest.join(&name);

        if entry.file_type()?.is_dir() {
            copy_dir_filtered(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path)
                .with_context(|| format!("copying {} to {}", src_path.display(), dest_path.display()))?;
        }
    }
    Ok(())
}

/// Inline `field.workspace = true` patterns in `[package]` with actual workspace values.
fn inline_workspace_package_fields(doc: &mut DocumentMut, ws_pkg: &Table) -> Result<()> {
    let pkg = match doc.get_mut("package").and_then(|p| p.as_table_mut()) {
        Some(t) => t,
        None => return Ok(()),
    };

    let inheritable_fields = [
        "version",
        "edition",
        "rust-version",
        "authors",
        "license",
        "repository",
        "homepage",
        "documentation",
        "description",
        "readme",
        "keywords",
        "categories",
    ];

    for field in &inheritable_fields {
        if is_workspace_inherited(pkg, field) {
            if let Some(ws_value) = ws_pkg.get(field) {
                pkg.insert(field, ws_value.clone());
            } else {
                pkg.remove(field);
            }
        }
    }

    Ok(())
}

/// Check if a field is `field.workspace = true` (either dotted key or table form).
fn is_workspace_inherited(table: &Table, key: &str) -> bool {
    if let Some(item) = table.get(key) {
        if let Some(tbl) = item.as_table_like() {
            if let Some(ws) = tbl.get("workspace") {
                return ws.as_value().and_then(|v| v.as_bool()) == Some(true);
            }
        }
    }
    false
}

/// Inline workspace dependencies in a `[dependencies]` (or `[dev-dependencies]`, etc.) section.
fn inline_workspace_deps(doc: &mut DocumentMut, section: &str, ws_deps: &Table) -> Result<()> {
    let deps = match doc.get_mut(section).and_then(|d| d.as_table_mut()) {
        Some(t) => t,
        None => return Ok(()),
    };

    // Collect keys that need inlining (can't mutate while iterating).
    let keys_to_inline: Vec<String> = deps
        .iter()
        .filter_map(|(key, item)| {
            if let Some(tbl) = item.as_table_like() {
                if tbl
                    .get("workspace")
                    .and_then(|w| w.as_value())
                    .and_then(|v| v.as_bool())
                    == Some(true)
                {
                    return Some(key.to_string());
                }
            }
            None
        })
        .collect();

    for key in keys_to_inline {
        let crate_extras = extract_non_workspace_fields(deps.get(&key));
        let ws_spec = ws_deps.get(&key);

        let inlined = build_inlined_dep(ws_spec, &crate_extras);
        deps.insert(&key, inlined);
    }

    Ok(())
}

/// Extract non-workspace fields from a dependency spec (e.g. `features`, `optional`, `default-features`).
fn extract_non_workspace_fields(item: Option<&Item>) -> BTreeMap<String, Item> {
    let mut extras = BTreeMap::new();
    if let Some(tbl) = item.and_then(|i| i.as_table_like()) {
        for (k, v) in tbl.iter() {
            if k != "workspace" {
                extras.insert(k.to_string(), v.clone());
            }
        }
    }
    extras
}

/// Build the inlined dependency item by merging workspace spec with crate-level extras.
fn build_inlined_dep(ws_spec: Option<&Item>, extras: &BTreeMap<String, Item>) -> Item {
    // If no workspace spec found, just drop the workspace key and keep extras.
    let ws_spec = match ws_spec {
        Some(spec) => spec,
        None => {
            if extras.is_empty() {
                return Item::None;
            }
            let mut tbl = toml_edit::InlineTable::new();
            for (k, v) in extras {
                if let Some(val) = v.as_value() {
                    tbl.insert(k, val.clone());
                }
            }
            return Item::Value(Value::InlineTable(tbl));
        }
    };

    // If workspace spec is a simple string version, and there are no extras, use that directly.
    if let Some(version_str) = ws_spec.as_str() {
        if extras.is_empty() {
            return Item::Value(Value::String(toml_edit::Formatted::new(version_str.to_string())));
        }
        // Has extras — build inline table with version + extras.
        let mut tbl = toml_edit::InlineTable::new();
        tbl.insert(
            "version",
            Value::String(toml_edit::Formatted::new(version_str.to_string())),
        );
        for (k, v) in extras {
            if let Some(val) = v.as_value() {
                tbl.insert(k, val.clone());
            }
        }
        return Item::Value(Value::InlineTable(tbl));
    }

    // Workspace spec is a table — merge with extras (extras override).
    if let Some(ws_tbl) = ws_spec.as_table_like() {
        let mut tbl = toml_edit::InlineTable::new();
        // Copy all workspace fields except `path` (path deps don't resolve in vendored context).
        for (k, v) in ws_tbl.iter() {
            if k == "path" {
                continue;
            }
            if let Some(val) = v.as_value() {
                tbl.insert(k, val.clone());
            }
        }
        // Overlay crate-level extras.
        for (k, v) in extras {
            if let Some(val) = v.as_value() {
                tbl.insert(k, val.clone());
            }
        }
        return Item::Value(Value::InlineTable(tbl));
    }

    // Fallback — shouldn't happen.
    ws_spec.clone()
}

/// Remove `[lints] workspace = true` section.
fn remove_workspace_lints(doc: &mut DocumentMut) {
    if let Some(lints) = doc.get("lints").and_then(|l| l.as_table_like()) {
        if lints
            .get("workspace")
            .and_then(|w| w.as_value())
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            doc.remove("lints");
        }
    }
}

/// Generate a minimal workspace Cargo.toml for the vendor directory.
fn generate_vendor_workspace_manifest(crate_name: &str, ws_pkg: Option<&Table>, ws_deps: Option<&Table>) -> String {
    let mut lines = Vec::new();
    lines.push("[workspace]".to_string());
    lines.push(format!("members = [\"{crate_name}\"]"));
    lines.push("resolver = \"2\"".to_string());
    lines.push(String::new());

    // Add workspace.package fields.
    if let Some(pkg) = ws_pkg {
        lines.push("[workspace.package]".to_string());
        for (key, value) in pkg.iter() {
            lines.push(format!("{key} = {value}"));
        }
        lines.push(String::new());
    }

    // Add workspace.dependencies (skip path-only deps — those are internal crates).
    if let Some(deps) = ws_deps {
        lines.push("[workspace.dependencies]".to_string());
        for (key, value) in deps.iter() {
            let is_path_only = value
                .as_table_like()
                .is_some_and(|t| t.contains_key("path") && !t.contains_key("version"));
            if !is_path_only {
                lines.push(format!("{key} = {value}"));
            }
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Clean up vendored dependencies to reduce package size.
fn clean_vendored_deps(vendor_dir: &Path) -> Result<()> {
    if !vendor_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(vendor_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let crate_dir = entry.path();

        // Remove test/bench/example dirs.
        for dir_name in &["tests", "benches", "examples", "test", "bench"] {
            let dir = crate_dir.join(dir_name);
            if dir.exists() {
                fs::remove_dir_all(&dir).ok();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_workspace(root: &Path) {
        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
resolver = "2"
members = ["crates/my-lib", "crates/my-lib-py"]

[workspace.package]
version = "1.2.3"
edition = "2024"
rust-version = "1.85"
license = "MIT"
authors = ["Test Author"]
repository = "https://github.com/test/my-lib"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
anyhow = "1"
my-lib = { version = "1.2.3", path = "crates/my-lib" }
tokio = { version = "1", features = ["full"] }
"#,
        )
        .unwrap();

        let core_dir = root.join("crates/my-lib/src");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("lib.rs"), "pub fn hello() {}").unwrap();
        fs::write(
            root.join("crates/my-lib/Cargo.toml"),
            r#"
[package]
name = "my-lib"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
serde = { workspace = true }
anyhow = { workspace = true, optional = true }

[dev-dependencies]
tokio = { workspace = true }

[lints]
workspace = true
"#,
        )
        .unwrap();
    }

    #[test]
    fn vendor_core_only_copies_and_rewrites() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        let dest = root.join("vendor");
        fs::create_dir_all(&dest).unwrap();

        let result = vendor_core_only(root, &root.join("crates/my-lib"), &dest, true).unwrap();

        // Check crate was copied.
        assert!(result.vendor_dir.join("src/lib.rs").exists());

        // Check vendored Cargo.toml was rewritten.
        let vendored = fs::read_to_string(result.vendor_dir.join("Cargo.toml")).unwrap();
        let doc: DocumentMut = vendored.parse().unwrap();

        // Package fields should be inlined.
        let pkg = doc["package"].as_table().unwrap();
        assert_eq!(pkg["version"].as_str(), Some("1.2.3"));
        assert_eq!(pkg["edition"].as_str(), Some("2024"));
        assert_eq!(pkg["license"].as_str(), Some("MIT"));

        // Dependencies should be inlined.
        let deps = doc["dependencies"].as_table().unwrap();
        let serde = deps["serde"].as_inline_table().unwrap();
        assert_eq!(serde.get("version").and_then(|v| v.as_str()), Some("1"));
        assert!(serde.get("features").is_some());
        assert!(serde.get("workspace").is_none()); // workspace key removed

        // anyhow should have version + optional.
        let anyhow = deps["anyhow"].as_inline_table().unwrap();
        assert_eq!(anyhow.get("version").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(anyhow.get("optional").and_then(|v| v.as_bool()), Some(true));

        // [lints] should be removed.
        assert!(doc.get("lints").is_none());

        // Workspace manifest should be generated.
        let ws_manifest = result.workspace_manifest.unwrap();
        assert!(ws_manifest.exists());
        let ws_content = fs::read_to_string(&ws_manifest).unwrap();
        assert!(ws_content.contains("[workspace]"));
        assert!(ws_content.contains("\"my-lib\""));
        // Path-only dep (my-lib) should be excluded.
        assert!(!ws_content.contains("my-lib = { version"));
        // Non-path deps should be included.
        assert!(ws_content.contains("serde"));
        assert!(ws_content.contains("anyhow"));
    }

    #[test]
    fn vendor_core_only_cleans_target_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        // Create a fake target/ dir in the core crate.
        let target = root.join("crates/my-lib/target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("debug.txt"), "fake").unwrap();

        let dest = root.join("vendor");
        fs::create_dir_all(&dest).unwrap();
        let result = vendor_core_only(root, &root.join("crates/my-lib"), &dest, false).unwrap();

        // target/ should NOT be copied.
        assert!(!result.vendor_dir.join("target").exists());
    }

    #[test]
    fn vendor_core_only_without_workspace_manifest() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        let dest = root.join("vendor");
        fs::create_dir_all(&dest).unwrap();
        let result = vendor_core_only(root, &root.join("crates/my-lib"), &dest, false).unwrap();

        assert!(result.workspace_manifest.is_none());
        assert!(!dest.join("Cargo.toml").exists());
    }

    #[test]
    fn vendor_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        let dest = root.join("vendor");
        fs::create_dir_all(&dest).unwrap();

        // Vendor twice — second call should succeed and overwrite.
        vendor_core_only(root, &root.join("crates/my-lib"), &dest, true).unwrap();
        let result = vendor_core_only(root, &root.join("crates/my-lib"), &dest, true).unwrap();
        assert!(result.vendor_dir.join("src/lib.rs").exists());
    }
}
