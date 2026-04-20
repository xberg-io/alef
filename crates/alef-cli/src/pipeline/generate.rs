use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use anyhow::Context as _;
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
    let mut results = vec![];

    for &lang in languages {
        let lang_str = lang.to_string();
        let lang_hash = cache::compute_lang_hash(&ir_json, &lang_str, &config_toml);

        if !clean && cache::is_lang_cached(&lang_str, &lang_hash) {
            debug!("  {}: cached, skipping", lang_str);
            continue;
        }

        let backend = registry::get_backend(lang);
        info!("  {}: generating...", lang_str);

        let files = backend
            .generate_bindings(api, config)
            .with_context(|| format!("failed to generate bindings for {lang_str}"))?;
        let base_dir = std::env::current_dir().unwrap_or_default();
        let output_paths: Vec<std::path::PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
        cache::write_lang_hash(&lang_str, &lang_hash, &output_paths)
            .with_context(|| format!("failed to write language hash for {lang_str}"))?;
        results.push((lang, files));
    }

    Ok(results)
}

/// Generate type stubs for given languages.
pub fn generate_stubs(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let mut results = vec![];
    for &lang in languages {
        let backend = registry::get_backend(lang);
        let files = backend.generate_type_stubs(api, config)?;
        if !files.is_empty() {
            results.push((lang, files));
        }
    }
    Ok(results)
}

/// Generate public API wrappers for given languages.
pub fn generate_public_api(
    api: &ApiSurface,
    config: &AlefConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<(Language, Vec<GeneratedFile>)>> {
    let mut results = vec![];
    for &lang in languages {
        let backend = registry::get_backend(lang);
        let files = backend.generate_public_api(api, config)?;
        if !files.is_empty() {
            results.push((lang, files));
        }
    }
    Ok(results)
}

/// Write generated files to disk.
///
/// Rust files are formatted with `rustfmt` before writing so the on-disk
/// content matches what [`diff_files`] produces during `alef verify`.
/// This also means `cargo fmt` (run by prek) becomes a no-op for generated
/// files, making `alef all` idempotent.
pub fn write_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<usize> {
    let mut count = 0;
    for (_lang, lang_files) in files {
        for file in lang_files {
            let full_path = base_dir.join(&file.path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            let content = if file.path.extension().is_some_and(|ext| ext == "rs") {
                format_rust_content(&file.content)
            } else {
                normalize_whitespace(&file.content)
            };
            std::fs::write(&full_path, &content)
                .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
            count += 1;
            debug!("  wrote: {}", full_path.display());
        }
    }
    Ok(count)
}

/// Diff generated files against what's on disk.
///
/// For Rust files, both sides are formatted with rustfmt before comparison.
/// For all files, whitespace is normalized (trailing whitespace stripped,
/// trailing newline ensured) so that formatter-only diffs are ignored.
pub fn diff_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut diffs = vec![];
    for (lang, lang_files) in files {
        for file in lang_files {
            let full_path = base_dir.join(&file.path);
            let existing = std::fs::read_to_string(&full_path).unwrap_or_default();
            let is_rust = file.path.extension().is_some_and(|ext| ext == "rs");
            let generated = if is_rust {
                format_rust_content(&file.content)
            } else {
                file.content.clone()
            };
            let on_disk = if is_rust {
                format_rust_content(&existing)
            } else {
                existing
            };
            if normalize_whitespace(&on_disk) != normalize_whitespace(&generated) {
                diffs.push(format!("[{lang}] {}", file.path.display()));
            }
        }
    }
    Ok(diffs)
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

/// Format a Rust source string using `rustfmt` via a temporary file.
///
/// Uses a temp file in the current directory so that `rustfmt` discovers the
/// project's `rustfmt.toml` (e.g. `max_width = 120`).  This produces output
/// identical to `cargo fmt` / `prek`, ensuring `alef verify` matches.
///
/// Returns the formatted content on success, or the original content if
/// rustfmt is unavailable or fails (best-effort).
pub fn format_rust_content(content: &str) -> String {
    use std::process::Command;

    // Write to a temp file in cwd so rustfmt picks up rustfmt.toml.
    let tmp_name = format!(".alef_fmt_{}.rs", std::process::id());
    let tmp_path = std::env::current_dir().unwrap_or_default().join(&tmp_name);

    if std::fs::write(&tmp_path, content).is_err() {
        return content.to_string();
    }

    let result = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg(&tmp_path)
        .output();

    let formatted = match result {
        Ok(output) if output.status.success() => {
            std::fs::read_to_string(&tmp_path).unwrap_or_else(|_| content.to_string())
        }
        Ok(output) => {
            debug!("rustfmt failed: {}", String::from_utf8_lossy(&output.stderr));
            content.to_string()
        }
        Err(e) => {
            debug!("rustfmt process error: {e}");
            content.to_string()
        }
    };

    let _ = std::fs::remove_file(&tmp_path);
    formatted
}

/// Auto-format generated Rust files using `rustfmt` (best-effort, doesn't fail on error).
pub fn format_rust_files(files: &[(Language, Vec<GeneratedFile>)], base_dir: &Path) {
    let rs_files: Vec<_> = files
        .iter()
        .flat_map(|(_, lang_files)| lang_files.iter())
        .filter(|f| f.path.extension().is_some_and(|ext| ext == "rs"))
        .map(|f| base_dir.join(&f.path))
        .collect();

    if rs_files.is_empty() {
        return;
    }

    // Run rustfmt on each file individually (more reliable than cargo fmt for specific files)
    for path in &rs_files {
        let result = std::process::Command::new("rustfmt")
            .arg("--edition")
            .arg("2024")
            .arg(path)
            .output();
        match result {
            Ok(output) if !output.status.success() => {
                debug!(
                    "rustfmt warning on {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                debug!("rustfmt not available: {e}");
                return; // Don't try other files if rustfmt isn't installed
            }
            _ => {}
        }
    }
}
