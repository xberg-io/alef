//! Shared utilities for the package sub-modules.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Recursively copy a directory tree, skipping hidden dirs and common build artifacts.
///
/// Skips:
/// - Any entry whose name starts with `.`
/// - `target`, `node_modules`, `__pycache__`, `.git`, `zig-cache`
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src).context("reading source directory")? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            let name = file_name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            if matches!(
                name.as_ref(),
                "target" | "node_modules" | "__pycache__" | ".git" | "zig-cache"
            ) {
                continue;
            }
            fs::create_dir_all(&dest_path)?;
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

/// Copy an optional top-level file (README, CHANGELOG, LICENSE) from workspace_root into dst.
///
/// Silently skips the copy if the file does not exist.
pub fn copy_optional_file(workspace_root: &Path, filename: &str, dst: &Path) -> Result<()> {
    let src = workspace_root.join(filename);
    if src.exists() {
        fs::copy(&src, dst.join(filename)).with_context(|| format!("copying {filename}"))?;
    }
    Ok(())
}
