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
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

/// The dependency-table section names that can carry path dependencies.
const DEP_SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

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

    // Workspace spec is a table — merge with extras (extras override), dropping
    // `path`/`workspace` keys (path deps don't resolve in a vendored context).
    if let Some(ws_tbl) = ws_spec.as_table_like() {
        let tbl = inline_table_without_path(ws_tbl.iter(), extras.iter().map(|(k, v)| (k.as_str(), v)));
        return Item::Value(Value::InlineTable(tbl));
    }

    // Fallback — shouldn't happen.
    ws_spec.clone()
}

/// Build an inline dependency table from a base set of key/value pairs plus
/// overlay extras, stripping `path` and `workspace` keys.
///
/// This is the single implementation of "produce an inline table with the
/// preserved keys minus `path`/`workspace`". Both [`build_inlined_dep`] (which
/// merges a workspace spec) and [`rewrite_path_deps_to_registry`] (which sets a
/// registry `version`) reuse it. Later inserts win over earlier ones, so the
/// caller orders `base` then `overlay` so that overlay values override.
fn inline_table_without_path<'a, B, O>(base: B, overlay: O) -> InlineTable
where
    B: Iterator<Item = (&'a str, &'a Item)>,
    O: Iterator<Item = (&'a str, &'a Item)>,
{
    let mut tbl = InlineTable::new();
    for (k, v) in base.chain(overlay) {
        if k == "path" || k == "workspace" {
            continue;
        }
        if let Some(val) = v.as_value() {
            tbl.insert(k, val.clone());
        }
    }
    tbl
}

/// Rewrite workspace-member path dependencies in a shipped binding manifest to
/// registry version-dependencies.
///
/// For every dependency table — `[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]`, and each `[target.<cfg>.dependencies]` subtable — any
/// dependency whose key names a workspace member (per `members.names`) AND whose
/// value is a table containing a `path` key is replaced with an inline table of
/// the form `{ version = "<version>", <preserved extras> }`. Both `path` and any
/// `workspace` key are removed; other keys (`features`, `optional`,
/// `default-features`, …) are preserved.
///
/// Dependencies that are plain version strings, or that are not workspace
/// members, are left untouched. The operation is **idempotent**: re-running it on
/// already-rewritten output produces identical output (an inline table with a
/// `version` and no `path` is left unchanged).
pub fn rewrite_path_deps_to_registry(
    manifest_path: &Path,
    members: &crate::publish::workspace::WorkspaceMembers,
    version: &str,
) -> Result<()> {
    let content = fs::read_to_string(manifest_path).with_context(|| format!("reading {}", manifest_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    // Plain top-level dependency sections.
    for section in DEP_SECTIONS {
        if let Some(table) = doc.get_mut(section).and_then(|d| d.as_table_mut()) {
            rewrite_dep_table(table, members, version);
        }
    }

    // `[target.<cfg>.dependencies]` (and dev/build variants) for every target.
    if let Some(target_tbl) = doc.get_mut("target").and_then(|t| t.as_table_mut()) {
        for (_cfg, cfg_item) in target_tbl.iter_mut() {
            let Some(cfg_tbl) = cfg_item.as_table_mut() else {
                continue;
            };
            for section in DEP_SECTIONS {
                if let Some(table) = cfg_tbl.get_mut(section).and_then(|d| d.as_table_mut()) {
                    rewrite_dep_table(table, members, version);
                }
            }
        }
    }

    fs::write(manifest_path, doc.to_string()).with_context(|| format!("writing {}", manifest_path.display()))?;
    Ok(())
}

/// Rewrite a single dependency table in place: replace each workspace-member
/// path dependency with a registry version-dependency.
fn rewrite_dep_table(table: &mut Table, members: &crate::publish::workspace::WorkspaceMembers, version: &str) {
    // Collect keys to rewrite first (can't mutate while iterating).
    let keys: Vec<String> = table
        .iter()
        .filter_map(|(key, item)| {
            if !members.names.contains(key) {
                return None;
            }
            // Only table-form deps that carry a `path` are rewritten. Plain
            // version strings and version-only tables are left untouched (which
            // also makes the operation idempotent).
            let has_path = item.as_table_like().is_some_and(|t| t.contains_key("path"));
            if has_path { Some(key.to_string()) } else { None }
        })
        .collect();

    for key in keys {
        let Some(existing) = table.get(&key) else {
            continue;
        };
        let rewritten = strip_path_set_version(existing, version);
        table.insert(&key, rewritten);
    }
}

/// Produce an inline dependency table from an existing table-form spec by
/// stripping `path`/`workspace` and setting `version` to `version`.
///
/// Preserved extras (`features`, `optional`, `default-features`, …) are kept.
/// This is the registry-rewrite counterpart to [`build_inlined_dep`]; both share
/// [`inline_table_without_path`] so there is one implementation of the
/// path-strip/merge logic.
fn strip_path_set_version(existing: &Item, version: &str) -> Item {
    let version_item = Item::Value(Value::String(toml_edit::Formatted::new(version.to_string())));
    let base = std::iter::once(("version", &version_item));
    match existing.as_table_like() {
        Some(tbl) => {
            // Base = forced version; overlay = existing keys. The `*k != "version"`
            // filter is LOAD-BEARING: `inline_table_without_path` lets later (overlay)
            // inserts win, so an existing `version` key must be dropped from the
            // overlay or it would override the forced registry version.
            let extras: Vec<(&str, &Item)> = tbl.iter().filter(|(k, _)| *k != "version").collect();
            let merged = inline_table_without_path(base, extras.into_iter());
            Item::Value(Value::InlineTable(merged))
        }
        None => {
            // Not a table — produce a version-only inline table.
            let merged = inline_table_without_path(base, std::iter::empty());
            Item::Value(Value::InlineTable(merged))
        }
    }
}

/// Resolve the `Cargo.lock` for a shipped binding crate after path deps have been
/// rewritten to registry deps.
///
/// When `regenerate` is true, runs `cargo generate-lockfile` so the lock resolves
/// the (now registry-only) graph immediately.
///
/// On a `cargo generate-lockfile` failure the behavior depends on `strict`:
/// - `strict == false` (lenient, the default for local/pre-release dev): the
///   failure is logged and any existing `Cargo.lock` in `manifest_dir` is deleted
///   so cargo regenerates it at consumer build time.
/// - `strict == true` (CI/release): the failure is a HARD error — a referenced
///   workspace-member version is likely not yet published to the registry, so the
///   core crates must be published before the language packages.
///
/// When `regenerate` is false the lock is simply deleted (the `strict` flag is
/// only consulted on the regenerate path). The `regenerate` flag lets tests force
/// the offline delete path.
pub(crate) fn scrub_or_regenerate_lock(manifest_dir: &Path, regenerate: bool, strict: bool) -> Result<()> {
    let lock_path = manifest_dir.join("Cargo.lock");

    if regenerate {
        let manifest = manifest_dir.join("Cargo.toml");
        let status = std::process::Command::new("cargo")
            .arg("generate-lockfile")
            .arg("--manifest-path")
            .arg(&manifest)
            .status();
        match status {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => {
                if strict {
                    bail!(
                        "cargo generate-lockfile failed (exit code {}) for {} — a referenced \
                         workspace-member version is likely not yet published to the registry. \
                         Publish the core crate(s) before the language packages, then retry.",
                        s.code().unwrap_or(-1),
                        manifest.display()
                    );
                }
                tracing::warn!(
                    code = s.code().unwrap_or(-1),
                    "cargo generate-lockfile failed; deleting Cargo.lock so it regenerates at build time"
                );
            }
            Err(error) => {
                if strict {
                    return Err(error).with_context(|| {
                        format!(
                            "could not run cargo generate-lockfile for {} — a referenced \
                             workspace-member version is likely not yet published to the registry. \
                             Publish the core crate(s) before the language packages, then retry.",
                            manifest.display()
                        )
                    });
                }
                tracing::warn!(%error, "could not run cargo generate-lockfile; deleting Cargo.lock");
            }
        }
    }

    // Delete the lock (best-effort) so cargo regenerates it from the registry.
    if lock_path.exists() {
        fs::remove_file(&lock_path).with_context(|| format!("removing {}", lock_path.display()))?;
    }
    Ok(())
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
            // Skip any dep carrying a `path` — the path won't exist at the
            // vendored destination, so even a dual `{ version, path }` dep must
            // be dropped from the generated workspace manifest.
            let has_path = value.as_table_like().is_some_and(|t| t.contains_key("path"));
            if !has_path {
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

    // ---------------------------------------------------------------------
    // S3: rewrite_path_deps_to_registry + Cargo.lock
    // ---------------------------------------------------------------------

    use crate::publish::workspace::WorkspaceMembers;
    use std::collections::{BTreeMap, BTreeSet};

    fn members_with(names: &[&str]) -> WorkspaceMembers {
        WorkspaceMembers {
            names: names.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
            versions: BTreeMap::new(),
        }
    }

    /// Write a binding manifest with four workspace-member path-deps (one with
    /// `features`, one in `[dev-dependencies]`, one under a `[target.*]` table,
    /// one plain table) plus a normal registry dep.
    fn write_binding_manifest(path: &Path) {
        fs::write(
            path,
            r#"
[package]
name = "my-lib-py"
version = "0.1.0"
edition = "2024"

[dependencies]
my-lib = { path = "../my-lib", features = ["serde"] }
my-lib-core = { path = "../my-lib-core" }
anyhow = "1"

[dev-dependencies]
my-lib-testkit = { path = "../my-lib-testkit", default-features = false }

[target.'cfg(unix)'.dependencies]
my-lib-unix = { path = "../my-lib-unix", optional = true }
"#,
        )
        .unwrap();
    }

    #[test]
    fn rewrite_path_deps_replaces_members_keeps_others() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("Cargo.toml");
        write_binding_manifest(&manifest);

        let members = members_with(&["my-lib", "my-lib-core", "my-lib-testkit", "my-lib-unix"]);
        rewrite_path_deps_to_registry(&manifest, &members, "1.2.3").unwrap();

        let doc: DocumentMut = fs::read_to_string(&manifest).unwrap().parse().unwrap();

        // [dependencies]: my-lib rewritten with version + features, no path.
        let deps = doc["dependencies"].as_table().unwrap();
        let my_lib = deps["my-lib"].as_inline_table().unwrap();
        assert_eq!(my_lib.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(my_lib.get("path").is_none(), "path must be stripped");
        assert!(my_lib.get("features").is_some(), "features preserved");

        // my-lib-core: table without extras → version-only table.
        let core = deps["my-lib-core"].as_inline_table().unwrap();
        assert_eq!(core.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(core.get("path").is_none());

        // Normal registry dep untouched (still a plain string).
        assert_eq!(deps["anyhow"].as_str(), Some("1"));

        // [dev-dependencies]: default-features preserved.
        let dev = doc["dev-dependencies"].as_table().unwrap();
        let testkit = dev["my-lib-testkit"].as_inline_table().unwrap();
        assert_eq!(testkit.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(testkit.get("path").is_none());
        assert_eq!(testkit.get("default-features").and_then(|v| v.as_bool()), Some(false));

        // [target.'cfg(unix)'.dependencies]: optional preserved.
        let target = doc["target"].as_table().unwrap();
        let cfg = target["cfg(unix)"].as_table().unwrap();
        let unix_deps = cfg["dependencies"].as_table().unwrap();
        let unix = unix_deps["my-lib-unix"].as_inline_table().unwrap();
        assert_eq!(unix.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert!(unix.get("path").is_none());
        assert_eq!(unix.get("optional").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn rewrite_path_deps_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("Cargo.toml");
        write_binding_manifest(&manifest);

        let members = members_with(&["my-lib", "my-lib-core", "my-lib-testkit", "my-lib-unix"]);
        rewrite_path_deps_to_registry(&manifest, &members, "1.2.3").unwrap();
        let once = fs::read_to_string(&manifest).unwrap();
        rewrite_path_deps_to_registry(&manifest, &members, "1.2.3").unwrap();
        let twice = fs::read_to_string(&manifest).unwrap();

        assert_eq!(once, twice, "second rewrite must be a no-op");
    }

    #[test]
    fn rewrite_path_deps_dual_form_strips_path_and_sets_version() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("Cargo.toml");
        // Dual-form: has BOTH version and path — version must be overwritten
        // with the passed value and path removed.
        fs::write(
            &manifest,
            r#"
[package]
name = "b"
version = "0.1.0"

[dependencies]
my-lib = { version = "0.1", path = "../my-lib" }
"#,
        )
        .unwrap();

        let members = members_with(&["my-lib"]);
        rewrite_path_deps_to_registry(&manifest, &members, "9.9.9").unwrap();

        let doc: DocumentMut = fs::read_to_string(&manifest).unwrap().parse().unwrap();
        let my_lib = doc["dependencies"]["my-lib"].as_inline_table().unwrap();
        assert_eq!(my_lib.get("version").and_then(|v| v.as_str()), Some("9.9.9"));
        assert!(my_lib.get("path").is_none());
    }

    #[test]
    fn rewrite_path_deps_leaves_non_members_and_plain_strings() {
        let tmp = TempDir::new().unwrap();
        let manifest = tmp.path().join("Cargo.toml");
        fs::write(
            &manifest,
            r#"
[package]
name = "b"
version = "0.1.0"

[dependencies]
# A path dep that is NOT a workspace member — left untouched.
external = { path = "../external" }
# A plain version string member — left untouched (no path key).
my-lib = "1"
"#,
        )
        .unwrap();

        let members = members_with(&["my-lib"]);
        rewrite_path_deps_to_registry(&manifest, &members, "2.0.0").unwrap();

        let doc: DocumentMut = fs::read_to_string(&manifest).unwrap().parse().unwrap();
        let deps = doc["dependencies"].as_table().unwrap();
        // Non-member path dep retains its path.
        assert_eq!(
            deps["external"]
                .as_inline_table()
                .unwrap()
                .get("path")
                .and_then(|v| v.as_str()),
            Some("../external")
        );
        // Plain-string member is unchanged.
        assert_eq!(deps["my-lib"].as_str(), Some("1"));
    }

    #[test]
    fn scrub_lock_deletes_existing_when_not_regenerating() {
        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join("Cargo.lock");
        fs::write(&lock, "# lock").unwrap();

        scrub_or_regenerate_lock(tmp.path(), false, false).unwrap();
        assert!(!lock.exists(), "Cargo.lock must be deleted on the offline path");
    }

    #[test]
    fn scrub_lock_no_lock_is_noop() {
        let tmp = TempDir::new().unwrap();
        // No Cargo.lock present — must not error.
        scrub_or_regenerate_lock(tmp.path(), false, false).unwrap();
    }

    // ---------------------------------------------------------------------
    // S7: clean-room source-install smoke test
    //
    // Proves a rewritten binding manifest (member deps now registry
    // version-deps) resolves with NO workspace present. The positive control
    // touches the network (cargo generate-lockfile fetches the index) and is
    // gated behind `ALEF_SMOKE_REGISTRY=1`. The negative control (a bad path
    // dep) fails fully offline and always runs.
    // ---------------------------------------------------------------------

    fn write_clean_room_crate(dir: &Path, deps_body: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "pub fn smoke() {}\n").unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"clean-room\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps_body}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn clean_room_registry_dep_resolves_without_workspace() {
        // Network-touching positive control — gated so it doesn't run on every
        // `cargo test`. Enable with `ALEF_SMOKE_REGISTRY=1`.
        if std::env::var("ALEF_SMOKE_REGISTRY").is_err() {
            eprintln!("skipping clean_room_registry_dep_resolves_without_workspace (set ALEF_SMOKE_REGISTRY=1)");
            return;
        }
        let tmp = TempDir::new().unwrap();
        let crate_dir = tmp.path().join("clean-room");
        // A tiny well-known registry crate stands in for a (post-rewrite) member
        // version-dep: it proves resolution works with no workspace path.
        write_clean_room_crate(&crate_dir, "anyhow = \"1\"");

        let status = std::process::Command::new("cargo")
            .arg("generate-lockfile")
            .arg("--manifest-path")
            .arg(crate_dir.join("Cargo.toml"))
            .status()
            .expect("running cargo generate-lockfile");
        assert!(
            status.success(),
            "registry version-dep must resolve without a workspace"
        );
        assert!(crate_dir.join("Cargo.lock").exists(), "lockfile should be produced");
    }

    #[test]
    fn clean_room_bad_path_dep_fails_offline() {
        // Negative control: a path dep pointing at a nonexistent crate fails
        // resolution without any network access — always runs.
        let tmp = TempDir::new().unwrap();
        let crate_dir = tmp.path().join("clean-room");
        write_clean_room_crate(&crate_dir, "ghost = { path = \"../does-not-exist\" }");

        // NOTE: must NOT pass `--no-deps` — that skips dependency-graph
        // resolution and would mask the broken path dep.
        let output = std::process::Command::new("cargo")
            .arg("metadata")
            .arg("--format-version")
            .arg("1")
            .arg("--manifest-path")
            .arg(crate_dir.join("Cargo.toml"))
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .expect("running cargo metadata");
        assert!(
            !output.status.success(),
            "a path dep to a nonexistent crate must fail resolution"
        );
    }

    // ---------------------------------------------------------------------
    // S6: strict release-ordering precondition on lock regeneration
    //
    // Reuse the bad-path setup: a manifest with a path dep to a nonexistent
    // crate makes `cargo generate-lockfile` fail locally (missing path, no
    // network). In strict mode that failure is a hard, actionable error; in
    // lenient mode it falls back to deleting the lock and returns Ok.
    // ---------------------------------------------------------------------

    /// Build a crate whose single dependency is an unresolvable path dep, so
    /// `cargo generate-lockfile` fails offline.
    fn write_unresolvable_crate(dir: &Path) {
        write_clean_room_crate(dir, "ghost = { path = \"../does-not-exist\" }");
    }

    /// Run `scrub_or_regenerate_lock` in regenerate mode. The crate's single
    /// dependency is an unresolvable path dep, so `cargo generate-lockfile`
    /// fails locally on the missing path with no network access — no process-wide
    /// `CARGO_NET_OFFLINE` mutation (which is racy under parallel tests) is needed.
    fn scrub_regenerate(crate_dir: &Path, strict: bool) -> Result<()> {
        scrub_or_regenerate_lock(crate_dir, true, strict)
    }

    #[test]
    fn scrub_lock_strict_errors_when_lockfile_cannot_resolve() {
        let tmp = TempDir::new().unwrap();
        let crate_dir = tmp.path().join("clean-room");
        write_unresolvable_crate(&crate_dir);

        let err = scrub_regenerate(&crate_dir, true)
            .expect_err("strict mode must return Err when the lockfile cannot resolve");
        let msg = err.to_string();
        assert!(
            msg.contains("not yet published to the registry"),
            "error must be actionable about unpublished member versions; got: {msg}"
        );
        assert!(
            msg.contains(&crate_dir.join("Cargo.toml").display().to_string()),
            "error must name the manifest; got: {msg}"
        );
    }

    #[test]
    fn scrub_lock_lenient_falls_back_to_delete_when_lockfile_cannot_resolve() {
        let tmp = TempDir::new().unwrap();
        let crate_dir = tmp.path().join("clean-room");
        write_unresolvable_crate(&crate_dir);

        // Pre-seed a lock — lenient mode deletes it on regenerate failure.
        let lock = crate_dir.join("Cargo.lock");
        fs::write(&lock, "# stale lock").unwrap();

        scrub_regenerate(&crate_dir, false).expect("lenient mode must return Ok and fall back to deleting the lock");
        assert!(!lock.exists(), "lenient fallback must delete the unresolved Cargo.lock");
    }
}
