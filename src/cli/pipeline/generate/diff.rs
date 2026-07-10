use super::normalization::{format_rust_content, normalize_content, normalize_whitespace};
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::hash;
use rayon::prelude::*;
use std::path::Path;

/// Diff generated files against what's on disk.
///
/// For Rust files, both sides are formatted with rustfmt before comparison.
/// For all files, whitespace is normalized (trailing whitespace stripped,
/// trailing newline ensured) so that formatter-only diffs are ignored.
pub fn diff_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<Vec<String>> {
    let all_items: Vec<_> = files
        .iter()
        .flat_map(|(lang, lang_files)| lang_files.iter().map(move |f| (*lang, f)))
        .collect();

    let diffs: Vec<String> = all_items
        .par_iter()
        .filter_map(|(lang, file)| {
            let full_path = base_dir.join(&file.path);
            let existing = std::fs::read_to_string(&full_path).unwrap_or_default();
            let is_rust = file.path.extension().is_some_and(|ext| ext == "rs");
            let generated = normalize_content(&file.path, &file.content);
            let on_disk = if is_rust {
                format_rust_content(&full_path, &existing)
            } else {
                existing
            };
            let on_disk_body = hash::strip_hash_line(&on_disk);
            if normalize_whitespace(&on_disk_body) != normalize_whitespace(&generated) {
                Some(format!("[{lang}] {}", file.path.display()))
            } else {
                None
            }
        })
        .collect();

    Ok(diffs)
}
