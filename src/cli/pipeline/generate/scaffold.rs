use super::normalization::normalize_content;
use super::write::apply_shebang_chmod;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use base64::Engine;
use std::path::Path;
use tracing::debug;

/// Generate scaffold files for given languages.
///
/// After the built-in scaffold generators run, each registered extension gets a
/// chance to rewrite the scaffold file set per language via
/// [`crate::core::extension::Extension::transform_scaffold_files`] — for example
/// to wire an ergonomic entry module into a package `main`/wrapper or to add
/// runtime dependencies to a manifest. Extensions receive their
/// `[extensions.<name>]` config from `config_path` (`alef.toml`).
pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    config_path: &Path,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = crate::scaffold::scaffold(api, config, languages)?;
    crate::with_extensions(|exts| {
        let env = crate::core::template_env::TemplateEnv::new();
        for ext in exts {
            let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
            let cfg = ext
                .parse_config(raw.as_ref())
                .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
            for &language in languages {
                ext.transform_scaffold_files(api, &cfg, language, &mut files, &env)
                    .with_context(|| {
                        format!(
                            "extension `{}`: transform_scaffold_files({language}) failed",
                            ext.name()
                        )
                    })?;
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(files)
}

/// Generate README files for given languages.
pub fn readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    crate::readme::generate_readmes(api, config, languages)
}

/// Write standalone generated files (not grouped by language) to disk.
///
/// Scaffold files are create-only by default: if the target file already exists
/// on disk it is left untouched so that user customisations are preserved.
/// Pass `overwrite = true` (e.g. via `--clean`) to force-write all files.
///
/// Files that carry the alef header marker (regenerated bindings, READMEs)
/// will receive their `alef:hash:` line later via [`super::write::finalize_hashes`] —
/// scaffold files without the marker (Cargo.toml templates, composer.json,
/// gemspec) pass through unchanged.
pub fn write_scaffold_files(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<usize> {
    write_scaffold_files_with_overwrite(files, base_dir, false)
}

/// Like [`write_scaffold_files`] but with an explicit `overwrite` flag.
///
/// Files marked `generated_header: true` are always overwritten regardless of the
/// flag: these are fully alef-managed manifests (Cargo.toml, gemspec, composer.json)
/// whose dependency lists are derived from `[workspace.languages]`, `[crates.*]`,
/// and the active adapter set. Skipping them on regen means newly added streaming
/// adapters or trait bridges never get their conditional deps (futures-util,
/// futures, tokio sync features) appended, leaving the generated bindings
/// referencing crates that aren't in `[dependencies]`. Files with
/// `generated_header: false` are seeds (py.typed markers, sample test files,
/// README.md placeholders) and stay create-only so user edits survive.
pub fn write_scaffold_files_with_overwrite(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<usize> {
    let mut count = 0;
    for file in files {
        let full_path = base_dir.join(&file.path);
        let can_skip = !overwrite && !file.generated_header && full_path.exists();
        if can_skip {
            debug!("  skipped (already exists): {}", full_path.display());
            continue;
        }
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        // Binary file path: same as in `write_files`. Without this branch the
        // scaffold writer writes the base64 STRING into the .jar file, so
        // every `task <lang>:smoke` invocation hits "ClassNotFoundException:
        // GradleWrapperMain" because the jar isn't a real zip archive.
        let is_jar_file = full_path.extension().is_some_and(|ext| ext == "jar");
        if is_jar_file {
            let binary_content = base64::engine::general_purpose::STANDARD
                .decode(&file.content)
                .with_context(|| format!("failed to decode base64 for {}", full_path.display()))?;
            if let Ok(existing) = std::fs::read(&full_path) {
                if existing == binary_content {
                    debug!("  unchanged: {}", full_path.display());
                    continue;
                }
            }
            std::fs::write(&full_path, &binary_content)
                .with_context(|| format!("failed to write binary file {}", full_path.display()))?;
            count += 1;
            debug!("  wrote (binary): {}", full_path.display());
            continue;
        }
        let normalized = normalize_content(&full_path, &file.content);
        // Skip the write when on-disk bytes already match the normalized output.
        // `std::fs::write` is unconditional truncate+write, which updates mtime
        // even for identical content; pre-commit/prek hooks then report the file
        // as "modified by this hook" and fail the run, breaking the
        // alef-sync-versions hook for downstream repos on every commit.
        //
        // The on-disk file may carry an `alef:hash:` line injected by
        // `finalize_hashes` after the original write, while the freshly
        // generated `normalized` does not — so strip the hash line from both
        // before comparing. `finalize_hashes` runs after this function and
        // re-injects the hash idempotently, so skipping the rewrite here does
        // not lose information.
        if let Ok(existing) = std::fs::read_to_string(&full_path) {
            let existing_body = crate::core::hash::strip_hash_line(&existing);
            let normalized_body = crate::core::hash::strip_hash_line(&normalized);
            if existing_body == normalized_body {
                apply_shebang_chmod(&full_path, &normalized)?;
                debug!("  unchanged: {}", full_path.display());
                continue;
            }
        }
        std::fs::write(&full_path, &normalized)
            .with_context(|| format!("failed to write generated file {}", full_path.display()))?;
        apply_shebang_chmod(&full_path, &normalized)?;
        count += 1;
        debug!("  wrote: {}", full_path.display());
    }
    Ok(count)
}
