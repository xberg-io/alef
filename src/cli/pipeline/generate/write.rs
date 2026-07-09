use super::normalization::normalize_content;
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use crate::core::hash;
use anyhow::Context as _;
use base64::Engine;
use rayon::prelude::*;
use std::path::Path;
use tracing::debug;

/// Apply `0o755` permissions to a file whose content begins with a shebang line.
///
/// Called immediately after every `fs::write` in both [`write_files`] and
/// [`write_scaffold_files_with_overwrite`] so that generated shell scripts
/// (e.g. `download_ffi.sh`, `run_tests.sh`, `mvnw`) are executable on Unix
/// without a manual `chmod` step by the consumer.
///
/// On non-Unix platforms this is a no-op — POSIX permission bits do not exist.
#[cfg(unix)]
pub(crate) fn apply_shebang_chmod(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if content.starts_with("#!") {
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, perms).with_context(|| format!("failed to chmod 755 {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn apply_shebang_chmod(_path: &std::path::Path, _content: &str) -> anyhow::Result<()> {
    Ok(())
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

        // Check if this is a binary file that needs base64 decoding.
        let is_jar_file = full_path.extension().is_some_and(|ext| ext == "jar");

        if is_jar_file {
            // Decode base64 content to binary.
            let binary_content = base64::engine::general_purpose::STANDARD
                .decode(&file.content)
                .with_context(|| format!("failed to decode base64 for {}", full_path.display()))?;

            // Skip the write when on-disk bytes already match.
            if let Ok(existing) = std::fs::read(&full_path) {
                if existing == binary_content {
                    debug!("  unchanged: {}", full_path.display());
                    return Ok(());
                }
            }

            std::fs::write(&full_path, &binary_content)
                .with_context(|| format!("failed to write binary file {}", full_path.display()))?;
            debug!("  wrote: {}", full_path.display());
        } else {
            // Text file handling (existing logic).
            let normalized = normalize_content(&full_path, &file.content);
            // Skip the write when on-disk bytes already match (modulo the
            // post-write `alef:hash:` line injected by `finalize_hashes`).
            // `std::fs::write` is unconditional truncate+write; updating mtime
            // on identical content trips pre-commit/prek's modification check
            // and breaks every alef-driven hook for downstream repos.
            if let Ok(existing) = std::fs::read_to_string(&full_path) {
                let existing_body = crate::core::hash::strip_hash_line(&existing);
                let normalized_body = crate::core::hash::strip_hash_line(&normalized);
                if existing_body == normalized_body {
                    apply_shebang_chmod(&full_path, &normalized)?;
                    debug!("  unchanged: {}", full_path.display());
                    return Ok(());
                }
            }
            std::fs::write(&full_path, &normalized)
                .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
            apply_shebang_chmod(&full_path, &normalized)?;
            debug!("  wrote: {}", full_path.display());
        }
        Ok(())
    })?;

    Ok(all_files.len())
}

/// Inject the per-file `alef:hash:` line into every alef-headered file in
/// `paths`. Run *after* every formatter (`format_generated`, `fmt_post_generate`).
///
/// The embedded hash is a **generation-inputs fingerprint** computed by
/// [`hash::compute_inputs_hash`] from the alef revision, the Rust source
/// fingerprint (`sources_hash`), and the raw `alef.toml` bytes. It does **not**
/// depend on the emitted file content, so post-generation formatter rewrites
/// (rustfmt, ruff, rumdl-fmt, oxfmt, …) never invalidate it.
///
/// Files that don't carry the alef header marker (scaffold-once Cargo.toml,
/// composer.json, gemspec, package.json, lockfiles) are skipped — alef has
/// no claim on them.
pub fn finalize_hashes(
    paths: &std::collections::HashSet<std::path::PathBuf>,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
) -> anyhow::Result<usize> {
    // Compute the inputs hash once for the whole run — it is the same for every
    // file generated from the same (alef_rev, sources, alef.toml) tuple.
    let inputs_hash = hash::compute_inputs_hash(sources_hash, alef_toml_bytes);

    let updated: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    paths.par_iter().try_for_each(|path| -> anyhow::Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };
        // Only touch files alef stamped with the header marker. Anything else
        // (scaffold-once manifest, lockfile) is user-owned.
        // Recognize both "auto-generated by alef" (standard header) and
        // "Generated by alef" (custom headers in Swift, Kotlin, Dart, Gleam, Zig, JNI).
        let has_marker = content
            .lines()
            .take(10)
            .any(|line| line.contains("auto-generated by alef") || line.contains("Generated by alef"));
        if !has_marker {
            return Ok(());
        }

        // Strip the existing hash line before injecting the new one. The
        // inputs hash is independent of file content, so no normalization
        // step is needed — formatters cannot affect it.
        let stripped = hash::strip_hash_line(&content);
        let final_content = hash::inject_hash_line(&stripped, &inputs_hash);

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
