//! Digest of the files of a path package.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::{hex, render_toml};
use crate::component::artifact::sha256_file;

/// Digest of the files of a path package.
///
/// Inside a git repository this is the SHA-256 over the sorted `(blob id, relative path)` pairs of
/// the tracked files (`git ls-files -s`) and the untracked-but-not-ignored ones
/// (`git ls-files -o --exclude-standard`, skipping any `target` directory, as the walk mode does).
/// Files whose working-tree content differs from the index (`git ls-files -m`) and untracked files
/// are hashed from the working tree (`git hash-object`), so an uncommitted edit or a new source file
/// still changes it. Blob ids make the digest independent of line-ending conversion and checkout
/// location. Outside git (or when git reports no file) it is the SHA-256 over the sorted
/// `(SHA-256 of content, relative path)` pairs of every regular file, skipping `target` and `.git`
/// directories. The two modes are prefixed differently, so the same tree hashes differently with
/// and without git; that only costs a rebuild.
///
/// In both modes the package's own `Cargo.toml` is hashed without its `[package] version`, so a
/// version bump alone does not change the digest.
///
/// In both modes a symlink to a file is hashed by the content of the file it points to, so
/// retargeting it or editing the target changes the digest. A symlink to a directory is not
/// followed (git mode hashes its link text; walk mode skips it). Files outside `dir`, reached by
/// `include_str!("../..")` or a build script, are not covered.
///
/// # Errors
///
/// Fails when a file cannot be read.
pub fn tree_digest(dir: &Path) -> Result<String> {
    let (mode, mut entries) = match git_entries(dir)? {
        Some(entries) => ("git", entries),
        None => ("walk", walk_entries(dir)?),
    };
    if let Some(entry) = entries.iter_mut().find(|(_, path)| path == "Cargo.toml") {
        entry.0 = manifest_digest(&dir.join("Cargo.toml"))?;
    }
    entries.sort_by(|left, right| left.1.cmp(&right.1));
    let mut digest = Sha256::new();
    digest.update(mode.as_bytes());
    digest.update(b"\n");
    for (id, path) in entries {
        digest.update(format!("{id} {path}\n").as_bytes());
    }
    Ok(hex(&digest.finalize()))
}

/// Digest of a package manifest with `[package] version` removed and keys sorted, so a
/// lock-step version bump (which rewrites a literal `version = "..."`) leaves the package digest
/// unchanged; `env!("CARGO_PKG_VERSION")` in a reused artifact keeps the version it was built at.
fn manifest_digest(manifest: &Path) -> Result<String> {
    let text = std::fs::read_to_string(manifest).with_context(|| format!("failed to read {}", manifest.display()))?;
    let mut table = text
        .parse::<toml::Table>()
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    if let Some(toml::Value::Table(package)) = table.get_mut("package") {
        package.remove("version");
    }
    let mut rendered = String::new();
    render_toml(&toml::Value::Table(table), &mut rendered);
    Ok(format!("manifest:{}", hex(&Sha256::digest(rendered.as_bytes()))))
}

fn git_output(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git").args(args).current_dir(dir).output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// Whether `dir` or one of its ancestors holds a `.git` entry.
pub(super) fn has_git_ancestor(dir: &Path) -> bool {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    dir.ancestors().any(|ancestor| ancestor.join(".git").exists())
}

fn nul_separated(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
}

/// The content digest of a symlink that points at a file, `None` for anything else.
fn followed_symlink_digest(path: &Path) -> Result<Option<String>> {
    let is_symlink = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if is_symlink && path.is_file() {
        return Ok(Some(format!("sha256:{}", sha256_file(path)?)));
    }
    Ok(None)
}

fn hash_working_file(dir: &Path, path: &str) -> Result<String> {
    if std::fs::symlink_metadata(dir.join(path)).is_err() {
        return Ok("deleted".to_string());
    }
    git_output(dir, &["hash-object", "--", path])
        .map(|out| String::from_utf8_lossy(&out).trim().to_string())
        .with_context(|| format!("`git hash-object` failed for {path}"))
}

/// `(blob id, path)` for the files under `dir` that git knows or would add, or `None` when `dir` is
/// not inside a usable git repository.
fn git_entries(dir: &Path) -> Result<Option<Vec<(String, String)>>> {
    let Some(listing) = git_output(dir, &["ls-files", "-s", "-z", "--", "."]) else {
        if has_git_ancestor(dir) {
            tracing::warn!(
                "`git ls-files` failed in {}, which lies inside a git checkout; hashing it by walking the directory instead",
                dir.display()
            );
        }
        return Ok(None);
    };
    let mut entries = BTreeMap::new();
    for entry in nul_separated(&listing) {
        let (meta, path) = entry.split_once('\t').context("unexpected `git ls-files -s` output")?;
        let blob = meta
            .split_whitespace()
            .nth(1)
            .context("unexpected `git ls-files -s` output")?;
        entries.insert(path.to_string(), blob.to_string());
    }
    let modified = git_output(dir, &["ls-files", "-m", "-z", "--", "."]).unwrap_or_default();
    for path in nul_separated(&modified) {
        let id = hash_working_file(dir, &path)?;
        entries.insert(path, id);
    }
    let untracked = git_output(dir, &["ls-files", "-o", "--exclude-standard", "-z", "--", "."]).unwrap_or_default();
    for path in nul_separated(&untracked).filter(|path| !path.split('/').any(|part| part == "target")) {
        let id = hash_working_file(dir, &path)?;
        entries.insert(path, id);
    }
    if entries.is_empty() {
        return Ok(None);
    }
    for (path, id) in &mut entries {
        if let Some(content) = followed_symlink_digest(&dir.join(path))? {
            *id = content;
        }
    }
    Ok(Some(entries.into_iter().map(|(path, id)| (id, path)).collect()))
}

fn walk_entries(dir: &Path) -> Result<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !(entry.file_type().is_dir() && matches!(entry.file_name().to_str(), Some("target" | ".git")))
        });
    for entry in walker {
        let entry = entry.with_context(|| format!("failed to walk {}", dir.display()))?;
        let is_file = entry.file_type().is_file() || (entry.path_is_symlink() && entry.path().is_file());
        if !is_file {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(dir)
            .context("walked path escaped its directory")?
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        entries.push((sha256_file(entry.path())?, relative));
    }
    Ok(entries)
}
