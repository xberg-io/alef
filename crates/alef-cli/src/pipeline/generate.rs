use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::hash;
use alef_core::ir::ApiSurface;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info};

use crate::cache;
use crate::registry;

/// Generate bindings for given languages.
pub fn generate(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
    clean: bool,
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    // Validate that Go/Java/C# have FFI in the languages list
    let has_ffi = languages.contains(&Language::Ffi);
    for &lang in languages {
        if (lang == Language::Go || lang == Language::Java || lang == Language::Csharp) && !has_ffi {
            tracing::warn!(
                "Language {:?} requires FFI to be in the languages list for proper code generation",
                lang
            );
        }
    }

    let ir_json = serde_json::to_string(api)?;
    let config_toml = toml::to_string(config).unwrap_or_default();

    let to_generate: Vec<_> = languages
        .par_iter()
        .filter_map(|&lang| {
            let lang_str = lang.to_string();
            let lang_hash = cache::compute_lang_hash(&ir_json, &lang_str, &config_toml);

            if !clean && cache::is_lang_cached(&lang_str, &lang_hash) {
                debug!("  {}: cached, skipping", lang_str);
                return None;
            }

            Some((lang, lang_str, lang_hash))
        })
        .collect();

    let results: Vec<(Language, Vec<GeneratedFile>)> = to_generate
        .par_iter()
        .map(|(lang, lang_str, lang_hash)| {
            let backend = registry::get_backend(*lang);
            info!("  {}: generating...", lang_str);

            let files = backend
                .generate_bindings(api, config)
                .with_context(|| format!("failed to generate bindings for {lang_str}"))?;
            let base_dir = std::env::current_dir().unwrap_or_default();
            let output_paths: Vec<std::path::PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_lang_hash(lang_str, lang_hash, &output_paths)
                .with_context(|| format!("failed to write language hash for {lang_str}"))?;
            Ok((*lang, files))
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(results)
}

/// Generate type stubs for given languages.
pub fn generate_stubs(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .map(|&lang| {
            let backend = registry::get_backend(lang);
            let files = backend.generate_type_stubs(api, config)?;
            Ok((lang, files))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect();
    Ok(results)
}

/// Generate public API wrappers for given languages.
pub fn generate_public_api(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let results: Vec<(Language, Vec<GeneratedFile>)> = languages
        .par_iter()
        .map(|&lang| {
            let backend = registry::get_backend(lang);
            let files = backend.generate_public_api(api, config)?;
            Ok((lang, files))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .collect();
    Ok(results)
}

/// Write generated files to disk.
///
/// Rust files are formatted with `rustfmt` before writing so prek's `cargo fmt`
/// hook is a no-op on regenerated content. The embedded `alef:hash:<hex>`
/// value is a **per-file source+output** hash from
/// [`hash::compute_file_hash`]: `blake3(sources_hash || file_content_without_hash_line)`.
///
/// Hashes are written in two passes by the caller:
/// 1. `write_files` writes content with the header but **no hash line** (the
///    header marker is left in place so [`finalize_hashes`] can find it later).
/// 2. After every formatter has run, the caller invokes [`finalize_hashes`]
///    to inject the per-file hash. This means the embedded hash always
///    reflects the actual on-disk byte content and `alef verify` is a
///    pure read+strip+rehash+compare with no regeneration.
pub fn write_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<usize> {
    // First pass: create all needed directories (sequential, deduped)
    let dirs: std::collections::BTreeSet<_> = files
        .iter()
        .flat_map(|(_, lang_files)| lang_files.iter())
        .filter_map(|f| base_dir.join(&f.path).parent().map(|p| p.to_path_buf()))
        .collect();
    for dir in &dirs {
        std::fs::create_dir_all(dir).with_context(|| format!("failed to create directory {}", dir.display()))?;
    }

    // Second pass: format and write files in parallel. The embedded hash is
    // injected later by `finalize_hashes` once all formatters are done.
    let all_files: Vec<_> = files.iter().flat_map(|(_, lang_files)| lang_files.iter()).collect();

    all_files.par_iter().try_for_each(|file| -> anyhow::Result<()> {
        let full_path = base_dir.join(&file.path);
        let normalized = normalize_content(&file.path, &file.content);
        std::fs::write(&full_path, &normalized)
            .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
        debug!("  wrote: {}", full_path.display());
        Ok(())
    })?;

    Ok(all_files.len())
}

/// Inject the per-file `alef:hash:` line into every alef-headered file in
/// `paths`. Run *after* every formatter (`format_generated`, `fmt_post_generate`)
/// so the embedded hash describes the final on-disk byte content.
///
/// Files that don't carry the alef header marker (scaffold-once Cargo.toml,
/// composer.json, gemspec, package.json, lockfiles) are skipped — alef has
/// no claim on them.
pub fn finalize_hashes(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    sources_hash: &str,
) -> anyhow::Result<usize> {
    let updated: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    paths.par_iter().try_for_each(|path| -> anyhow::Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        // Only touch files alef stamped with the header marker. Anything else
        // (scaffold-once manifest, lockfile) is user-owned.
        let has_marker = content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef"));
        if !has_marker {
            return Ok(());
        }

        // Compute the canonical hash from the current (post-format) on-disk
        // content. `compute_file_hash` strips any pre-existing hash line so
        // re-runs are idempotent.
        let stripped = hash::strip_hash_line(&content);
        let file_hash = hash::compute_file_hash(sources_hash, &stripped);
        let final_content = hash::inject_hash_line(&stripped, &file_hash);

        // Skip the write when the file already carries the right hash —
        // avoids invalidating mtime-based caches when nothing changed.
        if final_content == content {
            return Ok(());
        }

        std::fs::write(path, &final_content)
            .with_context(|| format!("failed to finalize hash for {}", path.display()))?;
        updated.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    })?;
    Ok(updated.into_inner())
}

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
                format_rust_content(&existing)
            } else {
                existing
            };
            // Compare bodies modulo the alef:hash: line (it is finalised post-format
            // and isn't part of the codegen output) and modulo trivial whitespace.
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

/// Normalize content the same way `write_files` does before hashing.
/// Rust files go through rustfmt; everything else gets whitespace normalization.
pub fn normalize_content(path: &Path, content: &str) -> String {
    if path.extension().is_some_and(|ext| ext == "rs") {
        format_rust_content(content)
    } else {
        normalize_whitespace(content)
    }
}

/// Normalize whitespace for comparison: strip trailing whitespace per line,
/// collapse runs of 3+ blank lines to 2, and ensure a single trailing newline.
fn normalize_whitespace(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut blank_count = 0;
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    // Ensure exactly one trailing newline
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Generate scaffold files for given languages.
pub fn scaffold(api: &ApiSurface, config: &AlefConfig, languages: &[Language]) -> anyhow::Result<Vec<GeneratedFile>> {
    alef_scaffold::scaffold(api, config, languages)
}

/// Generate README files for given languages.
pub fn readme(api: &ApiSurface, config: &AlefConfig, languages: &[Language]) -> anyhow::Result<Vec<GeneratedFile>> {
    alef_readme::generate_readmes(api, config, languages)
}

/// Write standalone generated files (not grouped by language) to disk.
///
/// Scaffold files are create-only by default: if the target file already exists
/// on disk it is left untouched so that user customisations are preserved.
/// Pass `overwrite = true` (e.g. via `--clean`) to force-write all files.
///
/// Files that carry the alef header marker (regenerated bindings, READMEs)
/// will receive their `alef:hash:` line later via [`finalize_hashes`] —
/// scaffold files without the marker (Cargo.toml templates, composer.json,
/// gemspec) pass through unchanged.
pub fn write_scaffold_files(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<usize> {
    write_scaffold_files_with_overwrite(files, base_dir, false)
}

/// Like [`write_scaffold_files`] but with an explicit `overwrite` flag.
pub fn write_scaffold_files_with_overwrite(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<usize> {
    let mut count = 0;
    for file in files {
        let full_path = base_dir.join(&file.path);
        if !overwrite && full_path.exists() {
            debug!("  skipped (already exists): {}", full_path.display());
            continue;
        }
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        std::fs::write(&full_path, &file.content)
            .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
        count += 1;
        debug!("  wrote: {}", full_path.display());
    }
    Ok(count)
}

/// Format a Rust source string by piping through `rustfmt`.
///
/// Reads from stdin and writes to stdout, avoiding temp files.  `rustfmt`
/// discovers the project's `rustfmt.toml` from the working directory.
///
/// Returns the formatted content on success, or the original content if
/// rustfmt is unavailable or fails (best-effort).
pub fn format_rust_content(content: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let config_dir = std::env::current_dir().unwrap_or_default();

    let mut child = match Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg("--config-path")
        .arg(&config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            debug!("rustfmt not available: {e}");
            return content.to_string();
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(content.as_bytes());
    }

    match child.wait_with_output() {
        Ok(output) if output.status.success() => {
            String::from_utf8(output.stdout).unwrap_or_else(|_| content.to_string())
        }
        Ok(output) => {
            debug!("rustfmt failed: {}", String::from_utf8_lossy(&output.stderr));
            content.to_string()
        }
        Err(e) => {
            debug!("rustfmt process error: {e}");
            content.to_string()
        }
    }
}
