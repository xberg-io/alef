//! Migration for crates that became components-only.
//!
//! A crate declaring components and no binding languages used to be scaffolded like a bindings
//! project. It no longer owns those files, so the ones alef previously wrote would otherwise sit on
//! disk forever, reported as orphans by `alef verify`.

use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::content_has_alef_marker;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Workspace-root files the scaffold stage used to emit for such a crate.
const FORMER_SCAFFOLD_FILES: &[&str] = &[
    "rust-toolchain.toml",
    "poly.toml",
    "rustfmt.toml",
    ".clang-format",
    ".gitattributes",
    ".cargo/config.toml",
];

fn collect_markdown(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect_markdown(&path, found),
            Ok(kind) if kind.is_file() && path.extension().is_some_and(|ext| ext == "md") => found.push(path),
            _ => {}
        }
    }
}

/// Delete the scaffold and docs-reference files a components-only crate no longer owns, but only
/// those that still carry an alef provenance marker. An unmarked file is user-owned and is never
/// touched. Returns the number of files removed.
pub fn reclaim_components_only_leftovers(config: &ResolvedCrateConfig, base_dir: &Path) -> anyhow::Result<usize> {
    if !config.is_components_only() {
        return Ok(0);
    }
    let mut candidates: Vec<PathBuf> = FORMER_SCAFFOLD_FILES.iter().map(|file| base_dir.join(file)).collect();
    collect_markdown(
        &base_dir.join(crate::docs::reference_output_dir(config)),
        &mut candidates,
    );

    let mut removed = 0;
    for path in candidates {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content_has_alef_marker(&content) {
            debug!("components-only: keeping unmarked {}", path.display());
            continue;
        }
        std::fs::remove_file(&path)?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        removed += 1;
    }
    if removed > 0 {
        info!("Removed {removed} scaffold/docs file(s) a components-only crate no longer owns");
    }
    Ok(removed)
}
