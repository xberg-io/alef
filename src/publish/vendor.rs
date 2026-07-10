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

/// Maximum attempts for the per-member `cargo update` + final `cargo metadata`
/// validation block in `scrub_or_regenerate_lock`. Crates.io's sparse-registry
/// index can lag behind a fresh `cargo publish` by a few minutes; in strict
/// (CI/release) mode we sleep and retry that block on errors that look like
/// the lookup-not-yet-propagated case so the publish workflow does not have to
/// hand-author a wait loop in every per-language job.
const REGISTRY_PROPAGATION_ATTEMPTS: usize = 6;

/// Sleep between attempts on registry-propagation-lag retries. 30 s × 6
/// attempts ≈ 3 min total wall clock, which covers the worst observed sparse
/// index lag without holding the publish workflow open for unbounded retries
/// when the registry is genuinely missing the requested version.
const REGISTRY_PROPAGATION_SLEEP_SECS: u64 = 30;

/// Match `cargo update` / `cargo metadata` stderr against the patterns cargo
/// emits when the registry index does not yet know about a requested version.
/// Matching here keeps the retry tight — a genuine resolver / manifest error
/// still surfaces immediately rather than waiting through six 30 s sleeps.
fn looks_like_registry_propagation_lag(stderr: &str) -> bool {
    stderr.contains("failed to select a version for the requirement")
        || stderr.contains("no matching package named")
        || (stderr.contains("could not find") && stderr.contains("in registry"))
}

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

    let vendor_crate_dir = dest_dir.join(&*core_crate_name);
    if vendor_crate_dir.exists() {
        fs::remove_dir_all(&vendor_crate_dir).with_context(|| format!("removing {}", vendor_crate_dir.display()))?;
    }
    copy_dir_filtered(core_crate_dir, &vendor_crate_dir)?;

    let crate_manifest_path = vendor_crate_dir.join("Cargo.toml");
    let crate_toml = fs::read_to_string(&crate_manifest_path)
        .with_context(|| format!("reading {}", crate_manifest_path.display()))?;
    let mut crate_doc: DocumentMut = crate_toml
        .parse()
        .with_context(|| format!("parsing {}", crate_manifest_path.display()))?;

    if let Some(ws_pkg) = workspace_pkg {
        inline_workspace_package_fields(&mut crate_doc, ws_pkg)?;
    }

    if let Some(ws_deps) = workspace_deps {
        inline_workspace_deps(&mut crate_doc, "dependencies", ws_deps)?;
        inline_workspace_deps(&mut crate_doc, "dev-dependencies", ws_deps)?;
        inline_workspace_deps(&mut crate_doc, "build-dependencies", ws_deps)?;
    }

    remove_workspace_lints(&mut crate_doc);

    if !generate_workspace_manifest && !crate_doc.as_table().contains_key("workspace") {
        crate_doc.as_table_mut().insert("workspace", Item::Table(Table::new()));
    }

    fs::write(&crate_manifest_path, crate_doc.to_string())
        .with_context(|| format!("writing {}", crate_manifest_path.display()))?;

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

    let cargo_config_dir = dest_dir.join(".cargo");
    fs::create_dir_all(&cargo_config_dir)?;
    let config_content = format!(
        "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n\
         [source.vendored-sources]\ndirectory = \"{}\"\n",
        vendor_deps_dir.display()
    );
    fs::write(cargo_config_dir.join("config.toml"), config_content)?;

    clean_vendored_deps(&vendor_deps_dir)?;

    tracing::info!(dest = %dest_dir.display(), "full vendor complete");
    Ok(result)
}

/// Copy a directory recursively, skipping `target/`, `.git/`, and temp files.
fn copy_dir_filtered(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

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

    if let Some(version_str) = ws_spec.as_str() {
        if extras.is_empty() {
            return Item::Value(Value::String(toml_edit::Formatted::new(version_str.to_string())));
        }
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

    if let Some(ws_tbl) = ws_spec.as_table_like() {
        let tbl = inline_table_without_path(ws_tbl.iter(), extras.iter().map(|(k, v)| (k.as_str(), v)));
        return Item::Value(Value::InlineTable(tbl));
    }

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

    for section in DEP_SECTIONS {
        if let Some(table) = doc.get_mut(section).and_then(|d| d.as_table_mut()) {
            rewrite_dep_table(table, members, version);
        }
    }

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
    let keys: Vec<String> = table
        .iter()
        .filter_map(|(key, item)| {
            if !members.names.contains(key) {
                return None;
            }
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
            let extras: Vec<(&str, &Item)> = tbl.iter().filter(|(k, _)| *k != "version").collect();
            let merged = inline_table_without_path(base, extras.into_iter());
            Item::Value(Value::InlineTable(merged))
        }
        None => {
            let merged = inline_table_without_path(base, std::iter::empty());
            Item::Value(Value::InlineTable(merged))
        }
    }
}

/// Resolve the `Cargo.lock` for a shipped binding crate after path deps have been
/// rewritten to registry deps.
///
/// When `regenerate` is true and a `workspace_lock` is supplied, the workspace
/// lockfile is copied to `manifest_dir/Cargo.lock` first to seed the resolver,
/// then `cargo update --package <name>` is invoked **once per workspace member**.
/// This refreshes only the entries whose source was just rewritten from `path` to
/// a registry version — every other transitive dep stays pinned exactly as the
/// workspace lockfile had it. A member that this particular binding does not
/// depend on (cargo reports "did not match any packages") is silently skipped.
///
/// The previous implementation called `cargo generate-lockfile`, which (despite
/// the seed) rebuilds the lock from scratch with the latest semver-compatible
/// version of every package. That defeated the seed and let a fresh upstream
/// release that shipped between the workspace `cargo update` and this prepare
/// step quietly substitute itself into the binding's graph — exposing
/// `brotli-decompressor 5.0.1`'s broken dep graph to downstream macos-arm64
/// NIF/PHP builds, and `time 0.3.48`'s cookie E0119 to an earlier release.
///
/// On a per-member `cargo update -p` failure the behavior depends on `strict`:
/// - `strict == false` (lenient, the default for local/pre-release dev): the
///   failure is logged and any existing `Cargo.lock` in `manifest_dir` is deleted
///   so cargo regenerates it at consumer build time.
/// - `strict == true` (CI/release): the failure is a HARD error — the referenced
///   workspace-member version is likely not yet published to the registry, so the
///   core crates must be published before the language packages.
///
/// When `regenerate` is false the lock is simply deleted (the `strict` and
/// `members` parameters are only consulted on the regenerate path). The
/// `regenerate` flag lets tests force the offline delete path.
pub(crate) fn scrub_or_regenerate_lock(
    manifest_dir: &Path,
    regenerate: bool,
    strict: bool,
    workspace_lock: Option<&Path>,
    members: &crate::publish::workspace::WorkspaceMembers,
) -> Result<()> {
    let lock_path = manifest_dir.join("Cargo.lock");

    if regenerate {
        if let Some(ws_lock) = workspace_lock
            && ws_lock.exists()
        {
            fs::copy(ws_lock, &lock_path)
                .with_context(|| format!("seeding {} from {}", lock_path.display(), ws_lock.display()))?;
            if let Err(error) = strip_workspace_member_entries(&lock_path, &members.names) {
                tracing::warn!(%error, "could not strip workspace-member entries from seed lockfile");
            }
        }
        let manifest_dir: std::borrow::Cow<'_, Path> = match manifest_dir.canonicalize() {
            Ok(abs) => std::borrow::Cow::Owned(abs),
            Err(error) => {
                if strict {
                    return Err(error).with_context(|| {
                        format!(
                            "could not canonicalize binding manifest directory {} — \
                             ensure the path exists before calling scrub_or_regenerate_lock \
                             in strict (CI/release) mode",
                            manifest_dir.display()
                        )
                    });
                }
                tracing::warn!(
                    %error,
                    path = %manifest_dir.display(),
                    "could not canonicalize manifest_dir; falling back to cwd-relative absolute path"
                );
                let abs = std::env::current_dir()
                    .context("could not determine process working directory for manifest path fallback")?
                    .join(manifest_dir);
                std::borrow::Cow::Owned(abs)
            }
        };
        let manifest = manifest_dir.join("Cargo.toml");

        'retry: for attempt in 0..REGISTRY_PROPAGATION_ATTEMPTS {
            let mut last_failure: Option<(i32, String, String)> = None;
            for member in &members.names {
                let registry_spec = members
                    .versions
                    .get(member)
                    .map(|version| format!("registry+https://github.com/rust-lang/crates.io-index#{member}@{version}"));
                let pkg_arg: &str = registry_spec.as_deref().unwrap_or(member);
                let output = std::process::Command::new("cargo")
                    .env_remove("CARGO_BUILD_LOCKED")
                    .env_remove("RUSTC_WRAPPER")
                    .env_remove("RUSTC_WORKSPACE_WRAPPER")
                    .current_dir(manifest_dir.as_ref())
                    .arg("update")
                    .arg("--manifest-path")
                    .arg(&manifest)
                    .arg("--package")
                    .arg(pkg_arg)
                    .output();
                match output {
                    Ok(out) if out.status.success() => {}
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        if stderr.contains("package ID specification") && stderr.contains("did not match any packages")
                        {
                            continue;
                        }
                        last_failure = Some((out.status.code().unwrap_or(-1), member.clone(), stderr.to_string()));
                    }
                    Err(error) => {
                        if strict {
                            return Err(error).with_context(|| {
                                format!(
                                    "could not run cargo update -p {member} for {} — a referenced \
                                 workspace-member version is likely not yet published to the \
                                 registry. Publish the core crate(s) before the language packages, \
                                 then retry.",
                                    manifest.display()
                                )
                            });
                        }
                        tracing::warn!(%error, package = %member, "could not run cargo update; deleting Cargo.lock");
                        last_failure = Some((-1, member.clone(), error.to_string()));
                        break;
                    }
                }
            }

            if last_failure.is_none() {
                let validation = std::process::Command::new("cargo")
                    .env_remove("CARGO_BUILD_LOCKED")
                    .env_remove("RUSTC_WRAPPER")
                    .env_remove("RUSTC_WORKSPACE_WRAPPER")
                    .current_dir(manifest_dir.as_ref())
                    .arg("metadata")
                    .arg("--format-version")
                    .arg("1")
                    .arg("--manifest-path")
                    .arg(&manifest)
                    .output();
                match validation {
                    Ok(out) if out.status.success() => return Ok(()),
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        last_failure = Some((
                            out.status.code().unwrap_or(-1),
                            "<lockfile>".to_string(),
                            stderr.to_string(),
                        ));
                    }
                    Err(error) => {
                        if strict {
                            return Err(error).with_context(|| {
                                format!(
                                    "could not run cargo metadata for {} to validate and complete the \
                                 seeded lockfile",
                                    manifest.display()
                                )
                            });
                        }
                        tracing::warn!(%error, "could not run cargo metadata; deleting Cargo.lock");
                        last_failure = Some((-1, "<lockfile>".to_string(), error.to_string()));
                    }
                }
            }

            if let Some((_, _, ref stderr)) = last_failure
                && strict
                && attempt + 1 < REGISTRY_PROPAGATION_ATTEMPTS
                && looks_like_registry_propagation_lag(stderr)
            {
                tracing::warn!(
                    attempt = attempt + 1,
                    max_attempts = REGISTRY_PROPAGATION_ATTEMPTS,
                    sleep_secs = REGISTRY_PROPAGATION_SLEEP_SECS,
                    "cargo update / metadata failed with apparent crates.io registry-index \
                 propagation lag; sleeping and retrying",
                );
                std::thread::sleep(std::time::Duration::from_secs(REGISTRY_PROPAGATION_SLEEP_SECS));
                continue 'retry;
            }

            if let Some((code, member, stderr)) = last_failure {
                if strict {
                    if looks_like_registry_propagation_lag(&stderr) {
                        bail!(
                            "cargo update -p {member} (or final cargo metadata validation) failed \
                         (exit code {code}) for {} — the referenced workspace-member version is \
                         likely not yet published to the registry. Publish the core crate(s) before \
                         the language packages, then retry.\n{stderr}",
                            manifest.display()
                        );
                    }
                    bail!(
                        "cargo update -p {member} (or final cargo metadata validation) failed \
                     (exit code {code}) for {}; see the cargo stderr below.\n{stderr}",
                        manifest.display()
                    );
                }
                tracing::warn!(
                    code,
                    package = %member,
                    "cargo update -p / metadata validation failed; deleting Cargo.lock so it regenerates at build time"
                );
            } else {
                return Ok(());
            }
            break 'retry;
        }
    }

    if lock_path.exists() {
        fs::remove_file(&lock_path).with_context(|| format!("removing {}", lock_path.display()))?;
    }
    Ok(())
}

/// Remove every `[[package]]` entry from `Cargo.lock` whose `name` matches a
/// workspace-member name. In the source workspace lock, members are encoded as
/// `[[package]]` entries with NO `source` field (cargo's marker for path-based
/// workspace members). After `rewrite_path_deps_to_registry` rewrites the
/// binding manifest, cargo's resolver still sees the seed's path-source entry
/// AND adds a registry entry for the rewritten dep — `cargo update -p NAME`
/// then errors with `specification 'NAME' is ambiguous` because both entries
/// share the same name and version. Removing the path-source entries here lets
/// the subsequent `cargo update -p NAME` add a single registry-source entry
/// per member without collision.
///
/// Returns an error if the lockfile cannot be parsed or rewritten; callers
/// treat this as non-fatal (the per-member update step would surface the
/// underlying problem with a clearer message anyway).
fn strip_workspace_member_entries(lock_path: &Path, member_names: &std::collections::BTreeSet<String>) -> Result<()> {
    if member_names.is_empty() {
        return Ok(());
    }
    let content = fs::read_to_string(lock_path).with_context(|| format!("reading {}", lock_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("parsing {} as TOML", lock_path.display()))?;

    let Some(packages) = doc.get_mut("package").and_then(|p| p.as_array_of_tables_mut()) else {
        return Ok(());
    };
    let mut idx = packages.len();
    while idx > 0 {
        idx -= 1;
        let pkg = match packages.get(idx) {
            Some(t) => t,
            None => continue,
        };
        let Some(name) = pkg.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if !member_names.contains(name) {
            continue;
        }
        if pkg.contains_key("source") {
            continue;
        }
        packages.remove(idx);
    }

    fs::write(lock_path, doc.to_string()).with_context(|| format!("writing {}", lock_path.display()))?;
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

    if let Some(pkg) = ws_pkg {
        lines.push("[workspace.package]".to_string());
        for (key, value) in pkg.iter() {
            lines.push(format!("{key} = {value}"));
        }
        lines.push(String::new());
    }

    if let Some(deps) = ws_deps {
        lines.push("[workspace.dependencies]".to_string());
        for (key, value) in deps.iter() {
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
mod tests;
