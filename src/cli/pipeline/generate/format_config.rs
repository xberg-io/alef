//! Pre-writes the format-config scaffold (`rustfmt.toml`, `.clang-format`) ahead of the very
//! first generated `.rs` byte, so [`super::normalization::format_rust_content`]'s bounded
//! rustfmt-config lookup finds a real, committed `rustfmt.toml` on a project's first
//! `alef all`/`alef generate` run instead of silently falling back to rustfmt's built-in
//! defaults for that one run -- see alef #465.
//!
//! `poly.toml` is deliberately excluded from what this pre-pass writes, even though
//! [`crate::scaffold::languages::scaffold_poly_config`] returns it alongside `rustfmt.toml`:
//! the real scaffold stage treats `poly.toml` as a *merge* target
//! (`super::scaffold::record_poly_merge_baseline`), and pre-writing it here would record that
//! merge baseline twice in the same run -- once here, once when the real scaffold stage reaches
//! it a few lines later. Nothing on the `.rs` write path this pre-pass exists to unblock ever
//! reads `poly.toml`, so leaving it for the real scaffold stage costs this pre-pass nothing.

use super::write::WriteReport;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::Path;

/// Whether `path` is one of the format-config files this pre-pass is allowed to write ahead of
/// the real scaffold stage. Both are single-component, repo-root-relative paths -- comparing
/// the path's own text is simpler than juggling `Path`/`PathBuf` equality here and cannot
/// misfire on a nested file of the same name, since `scaffold_poly_config` never emits either
/// name anywhere but the repo root. ~keep
fn is_prepass_retained_path(path: &Path) -> bool {
    matches!(path.to_str(), Some("rustfmt.toml") | Some(".clang-format"))
}

/// Write `rustfmt.toml`/`.clang-format` ahead of the bindings stage, so the first `.rs` file
/// this run generates can already resolve a real, project-bounded rustfmt config instead of
/// rustfmt's built-in defaults.
///
/// Reuses [`crate::scaffold::languages::scaffold_poly_config`] (it takes only `config` and
/// `languages`, no `ApiSurface`, so it can run this early in the pipeline) and writes through
/// the same byte-diff scaffold writer the real scaffold stage uses
/// ([`super::write_scaffold_files_report`]), so that later stage's write of the identical
/// content is a silent no-op rather than a second, redundant write.
///
/// Callers: `alef all` (`bin_cli::all_commands`, immediately before the bindings stage) and
/// `alef generate` (`bin_cli::core_commands::generate`, immediately before its own bindings
/// stage). Deliberately NOT called from [`super::generate`] itself, which `alef verify` also
/// calls read-only -- `alef verify` must never write to the tree it is checking.
pub fn write_format_config_prepass(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    base_dir: &Path,
) -> anyhow::Result<WriteReport> {
    let candidates = crate::scaffold::languages::scaffold_poly_config(config, languages);
    let files: Vec<GeneratedFile> = candidates
        .into_iter()
        .filter(|file| is_prepass_retained_path(&file.path))
        .collect();
    super::write_scaffold_files_report(&files, base_dir, false)
}

#[cfg(test)]
#[path = "format_config_tests.rs"]
mod tests;
