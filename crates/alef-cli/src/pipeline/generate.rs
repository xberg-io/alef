use alef_core::backend::GeneratedFile;
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::hash;
use alef_core::ir::ApiSurface;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info};

use crate::cache;
use crate::registry;

/// Generate bindings for given languages using a per-crate resolved config.
pub fn generate(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
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
    let config_toml =
        toml::to_string(config).with_context(|| "failed to serialize resolved crate config for cache key")?;

    let to_generate: Vec<_> = languages
        .par_iter()
        .filter_map(|&lang| {
            let lang_str = lang.to_string();
            let lang_hash = cache::compute_lang_hash(&ir_json, &lang_str, &config_toml);

            if !clean && cache::is_lang_cached(&config.name, &lang_str, &lang_hash) {
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
            cache::write_lang_hash(&config.name, lang_str, lang_hash, &output_paths)
                .with_context(|| format!("failed to write language hash for {lang_str}"))?;
            Ok((*lang, files))
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(results)
}

/// Generate type stubs for given languages.
pub fn generate_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
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
    config: &ResolvedCrateConfig,
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
///
/// For `.rs` files, `rustfmt` is applied (via [`normalize_content`]) before the
/// hash is computed. This guarantees the embedded hash always reflects
/// cargo-fmt-clean content, even when the file on disk was generated by an
/// older alef version or written from a warm cache without going through
/// `write_files`. Without this step a subsequent `cargo fmt` in CI would
/// reformat the file and break `alef verify`.
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

        // Strip the existing hash line so the content is in its "raw" state
        // before we compute the new hash.
        let stripped = hash::strip_hash_line(&content);

        // For Rust files, normalize (rustfmt + whitespace) before hashing so
        // the embedded hash always reflects cargo-fmt-clean content. This
        // handles two cases:
        //   1. Files written via write_files are already normalized, so this
        //      is a no-op (idempotent).
        //   2. Files that were not re-written this run (cache hit, or generated
        //      by an older alef that did not apply rustfmt) get reformatted
        //      here, so the recorded hash matches what `cargo fmt` would
        //      produce — preventing CI from seeing a fmt diff that breaks
        //      `alef verify`.
        let normalized = normalize_content(path, &stripped);

        let file_hash = hash::compute_file_hash(sources_hash, &normalized);
        let final_content = hash::inject_hash_line(&normalized, &file_hash);

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
///
/// Rust files go through rustfmt for canonical formatting, then through
/// `normalize_whitespace` so trailing-whitespace and trailing-newline rules
/// hold even when rustfmt could not parse the file (e.g. cextendr `lib.rs`
/// with non-standard `parameter: T = "default"` syntax that rustfmt rejects;
/// without the second pass, the raw codegen output retains trailing
/// whitespace on blank lines, and prek's `trailing-whitespace` hook then
/// rewrites the file post-finalisation, breaking `alef verify`).
///
/// Non-rust files skip rustfmt and go straight to whitespace normalization.
pub fn normalize_content(path: &Path, content: &str) -> String {
    let pre = if path.extension().is_some_and(|ext| ext == "rs") {
        format_rust_content(content)
    } else {
        content.to_string()
    };
    normalize_whitespace(&pre)
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
pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    alef_scaffold::scaffold(api, config, languages)
}

/// Generate README files for given languages.
pub fn readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
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
        let normalized = normalize_content(&full_path, &file.content);
        std::fs::write(&full_path, &normalized)
            .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
        count += 1;
        debug!("  wrote: {}", full_path.display());
    }
    Ok(count)
}

/// Delete alef-generated files under `roots` whose absolute path is not
/// present in `keep`. A file is considered alef-owned only when its first
/// 10 lines contain the literal `auto-generated by alef` marker — every
/// non-alef file (user code, fixtures, scaffolded manifests, lockfiles)
/// is left untouched.
///
/// This sweeps orphans left behind when categories or fixtures are removed
/// from the generation set (e.g. a category that produced 0 test functions
/// for the current binding surface). Without this pass, those files linger
/// on disk with stale `alef:hash:` headers and `alef verify` reports them
/// as stale forever.
///
/// Empty parent directories left behind after deletion are removed in a
/// best-effort second pass.
pub fn sweep_orphans(
    roots: &[std::path::PathBuf],
    keep: &std::collections::HashSet<std::path::PathBuf>,
) -> anyhow::Result<usize> {
    fn is_alef_owned(path: &std::path::Path) -> bool {
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(file);
        for (idx, line) in reader.lines().enumerate() {
            if idx >= 10 {
                break;
            }
            if let Ok(line) = line
                && line.contains("auto-generated by alef")
            {
                return true;
            }
        }
        false
    }

    let mut removed = 0usize;
    let mut touched_dirs: std::collections::BTreeSet<std::path::PathBuf> = std::collections::BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // Skip dependency / build directories
                    if matches!(
                        name,
                        ".git"
                            | "target"
                            | "node_modules"
                            | "vendor"
                            | "_build"
                            | "deps"
                            | ".venv"
                            | "venv"
                            | "build"
                            | "dist"
                            | "Pods"
                    ) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if keep.contains(&path) {
                    continue;
                }
                if !is_alef_owned(&path) {
                    continue;
                }
                if let Err(err) = std::fs::remove_file(&path) {
                    debug!("  sweep skip (remove failed): {} ({err})", path.display());
                    continue;
                }
                debug!("  swept orphan: {}", path.display());
                if let Some(parent) = path.parent() {
                    touched_dirs.insert(parent.to_path_buf());
                }
                removed += 1;
            }
        }
    }
    // Best-effort empty-dir cleanup: remove deepest-first so nested empties collapse.
    let mut dirs: Vec<_> = touched_dirs.into_iter().collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let _ = std::fs::remove_dir(&dir);
    }
    if removed > 0 {
        info!("Swept {removed} orphan generated file(s)");
    }
    Ok(removed)
}

/// Collect every alef-headered file under `root` (recursively), skipping
/// dependency / build directories.
///
/// Used by the `all` pipeline to gather existing registry-mode e2e files
/// (`test_apps/`) so their `alef:hash:` lines can be re-stamped after the
/// sources hash changes — without regenerating their content.
pub fn collect_alef_headered_paths(root: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    fn is_alef_owned(path: &std::path::Path) -> bool {
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(file);
        for (idx, line) in reader.lines().enumerate() {
            if idx >= 10 {
                break;
            }
            if let Ok(line) = line
                && line.contains("auto-generated by alef")
            {
                return true;
            }
        }
        false
    }

    let mut paths = std::collections::HashSet::new();
    if !root.exists() {
        return paths;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(
                    name,
                    ".git"
                        | "target"
                        | "node_modules"
                        | "vendor"
                        | "_build"
                        | "deps"
                        | ".venv"
                        | "venv"
                        | "build"
                        | "dist"
                        | "Pods"
                ) {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() && is_alef_owned(&path) {
                paths.insert(path);
            }
        }
    }
    paths
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

#[cfg(test)]
mod write_scaffold_normalize_tests {
    use super::*;
    use alef_core::backend::GeneratedFile;
    use std::path::PathBuf;

    fn make_file(name: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(name),
            content: content.to_owned(),
            generated_header: false,
        }
    }

    /// `write_scaffold_files_with_overwrite` must strip trailing whitespace and
    /// ensure a single trailing newline — matching what prek's
    /// `end-of-file-fixer` and `trailing-whitespace` hooks would do.
    #[test]
    fn test_scaffold_write_normalizes_trailing_whitespace_and_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content = "line one   \nline two\n\n";
        let files = vec![make_file("out.py", content)];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.py")).expect("read ok");
        assert_eq!(
            written, "line one\nline two\n",
            "trailing whitespace must be stripped and single newline ensured"
        );
    }

    #[test]
    fn test_scaffold_write_adds_missing_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.gleam", "pub fn main() {}")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.gleam")).expect("read ok");
        assert!(
            written.ends_with('\n'),
            "file must end with newline, got: {:?}",
            written
        );
    }

    #[test]
    fn test_scaffold_write_does_not_add_double_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.zig", "const x = 1;\n")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.zig")).expect("read ok");
        assert!(!written.ends_with("\n\n"), "must not have double trailing newline");
        assert!(written.ends_with('\n'));
    }

    /// `normalize_content` must strip trailing whitespace from `.rs` files even
    /// when rustfmt rejects them — e.g. cextendr `lib.rs` files use the
    /// `name: T = "default"` parameter-default syntax that rustfmt cannot
    /// parse, so it falls back to the raw codegen output. Without a final
    /// whitespace pass, the raw output's trailing-whitespace blank lines
    /// (e.g. `    \n` between `#[must_use]` and `pub fn …`) survive into the
    /// finalised `alef:hash`, and prek's `trailing-whitespace` hook then
    /// rewrites the file post-hash, breaking `alef verify`.
    #[test]
    fn test_normalize_content_strips_trailing_whitespace_when_rustfmt_fails() {
        // This rust-shaped content uses cextendr's parameter-default syntax,
        // which rustfmt rejects with `parameter defaults are not supported`.
        // The trailing whitespace on the `    ` line must be stripped.
        let path = PathBuf::from("packages/r/src/rust/src/lib.rs");
        let content = "extendr_module! {\n    fn convert(\n    \n        title: String = \"\",\n    );\n}\n";
        let normalized = normalize_content(&path, content);
        for (i, line) in normalized.lines().enumerate() {
            assert_eq!(
                line.trim_end(),
                line,
                "line {i} has trailing whitespace after normalize: {line:?}"
            );
        }
        assert!(normalized.ends_with('\n'), "must end with newline");
    }

    /// `sweep_orphans` must delete alef-marked files that aren't in the keep set,
    /// preserve user-owned files (no marker), and preserve files that are in the
    /// keep set even if they have the marker.
    #[test]
    fn test_sweep_orphans_removes_only_alef_marked_files_outside_keep_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let nested = base.join("e2e/elixir/test");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc\n";
        let kept = nested.join("keep_test.exs");
        let orphan = nested.join("orphan_test.exs");
        let user_owned = nested.join("user_helper.exs");

        std::fs::write(&kept, format!("{alef_marker}defmodule Keep do\nend\n")).unwrap();
        std::fs::write(&orphan, format!("{alef_marker}defmodule Orphan do\nend\n")).unwrap();
        std::fs::write(&user_owned, "defmodule UserHelper do\nend\n").unwrap();

        let mut keep = std::collections::HashSet::new();
        keep.insert(kept.clone());

        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 1, "should remove exactly one orphan");
        assert!(kept.exists(), "kept alef-marked file must remain");
        assert!(!orphan.exists(), "orphan alef-marked file must be removed");
        assert!(user_owned.exists(), "user-owned (no marker) file must remain");
    }

    /// `sweep_orphans` must skip dependency / build directories (target, node_modules,
    /// _build, deps, vendor, build, dist, .git, .venv) so it never deletes anything
    /// inside a vendored or compiled tree.
    #[test]
    fn test_sweep_orphans_skips_dependency_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let alef_marker = "// auto-generated by alef\n// alef:hash:def\n";
        for skip_dir in ["target", "node_modules", "_build", "vendor"] {
            let nested = base.join(skip_dir).join("nested");
            std::fs::create_dir_all(&nested).expect("mkdir");
            std::fs::write(nested.join("orphan.rs"), alef_marker).unwrap();
        }
        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 0, "must not descend into dependency directories");
    }

    /// `collect_alef_headered_paths` must return all alef-headered files under
    /// the given root and skip user-owned (no marker) files.
    #[test]
    fn test_collect_alef_headered_paths_finds_headered_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let lang_dir = base.join("python");
        std::fs::create_dir_all(&lang_dir).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc123\nprint('hello')\n";
        let user_file = "print('user code')\n";

        let headered = lang_dir.join("test_chat.py");
        let plain = lang_dir.join("conftest.py");
        std::fs::write(&headered, alef_marker).unwrap();
        std::fs::write(&plain, user_file).unwrap();

        let collected = collect_alef_headered_paths(base);
        assert!(collected.contains(&headered), "alef-headered file must be collected");
        assert!(!collected.contains(&plain), "user-owned file must not be collected");
    }

    /// `collect_alef_headered_paths` on a non-existent root must return an
    /// empty set without panicking.
    #[test]
    fn test_collect_alef_headered_paths_missing_root_returns_empty() {
        let paths = collect_alef_headered_paths(std::path::Path::new("/nonexistent/test_apps"));
        assert!(paths.is_empty(), "missing root must yield empty set");
    }

    /// Invariant: after `write` + simulated format-pass + `finalize_hashes`, the
    /// embedded `alef:hash:` must match what `compute_file_hash` produces from
    /// the on-disk content. This guards against the ordering bug where hashes
    /// were finalised before formatters ran (format would then mutate the file,
    /// leaving a stale hash that `alef verify` reported as stale).
    #[test]
    fn test_finalize_hashes_matches_on_disk_content_after_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        // Simulate a generated file with an alef header but no hash line yet.
        let content_before_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n";
        let file_path = base.join("lib.rs");
        std::fs::write(&file_path, content_before_format).expect("write pre-format content");

        // Simulate a formatter modifying the file (e.g. rustfmt adding newlines).
        let content_after_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n\n";
        std::fs::write(&file_path, content_after_format).expect("write post-format content");

        // Finalize hashes AFTER the format pass (correct ordering).
        let sources_hash = "deadbeef";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        finalize_hashes(&paths, sources_hash).expect("finalize ok");

        // Read the finalised file and verify the embedded hash matches.
        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");
        let embedded = alef_core::hash::extract_hash(&finalised).expect("hash must be present");
        let expected = alef_core::hash::compute_file_hash(sources_hash, &finalised);
        assert_eq!(
            embedded, expected,
            "embedded hash must match compute_file_hash of the post-format on-disk content"
        );
    }
}
