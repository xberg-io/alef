//! README generator for alef.

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

use std::path::PathBuf;

mod fallback;
mod paths;
mod template;
mod template_env;
#[cfg(test)]
mod tests;

/// Generate README files for the given languages.
pub fn generate_readmes(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        if let Some(file) = generate_readme(api, config, lang)? {
            files.push(file);
        }
    }
    Ok(files)
}

fn generate_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<Option<GeneratedFile>> {
    // Rust is the source crate, not a binding. The canonical Rust README lives at
    // the workspace crate (e.g. `crates/<name>/README.md`) and is hand-written or
    // managed by the consumer repo. Only emit a Rust README when the user has
    // explicitly opted in via `[readme.languages.rust]` with an `output_path`.
    if matches!(lang, Language::Rust) && !rust_readme_explicitly_configured(config) {
        return Ok(None);
    }

    // Language::Jni and Language::C are FFI shim glue layers, not publishable
    // bindings — they share their public surface with the host language (kotlin-android,
    // ffi). The hardcoded fallback used to emit them at `packages/zig/README.md`,
    // which collided with the actual Zig README. Skip silently — consumers that
    // want a C/JNI README can opt in via `[readme.languages.c]` / `.jni`.
    if matches!(lang, Language::C | Language::Jni) {
        return Ok(None);
    }

    // Try template-based generation first when readme config is present
    if let Some(readme_cfg) = &config.readme {
        if let Some(template_dir) = &readme_cfg.template_dir {
            let workspace_root = config.workspace_root.clone().unwrap_or_else(|| PathBuf::from("."));
            let abs_template_dir = workspace_root.join(template_dir);
            if abs_template_dir.exists() {
                if let Some(file) =
                    template::try_template_readme(api, config, lang, readme_cfg, &workspace_root, &abs_template_dir)?
                {
                    return Ok(Some(file));
                }
            }
        }
    }

    // Fall back to hardcoded generation
    Ok(Some(fallback::generate_readme_hardcoded(api, config, lang)?))
}

/// Returns true when the user has explicitly configured a Rust README in
/// `[readme.languages.rust]` with an `output_path` (or `output`). The default
/// behavior is to skip Rust because the Rust crate's README is the source-of-
/// truth `crates/<name>/README.md`, not a `packages/rust/` stub.
fn rust_readme_explicitly_configured(config: &ResolvedCrateConfig) -> bool {
    let Some(readme_cfg) = &config.readme else {
        return false;
    };
    let Some(rust_cfg) = readme_cfg.languages.get("rust") else {
        return false;
    };
    rust_cfg
        .get("output_path")
        .or_else(|| rust_cfg.get("output"))
        .and_then(|v| v.as_str())
        .is_some()
}
