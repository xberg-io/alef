//! README generator for alef.

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

use std::collections::HashSet;
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
    let mut seen_paths = HashSet::new();
    for &lang in languages {
        if let Some(file) = generate_readme(api, config, lang)? {
            push_unique_readme(&mut files, &mut seen_paths, file)?;
        }
    }
    for file in generate_readme_targets(api, config)? {
        push_unique_readme(&mut files, &mut seen_paths, file)?;
    }
    Ok(files)
}

/// Expand a binding-language list with any extra language README targets that
/// are configured but not part of the binding language matrix.
pub fn expand_configured_readme_languages(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<Language> {
    let mut expanded = languages.to_vec();
    if rust_readme_explicitly_configured(config) && !expanded.contains(&Language::Rust) {
        expanded.push(Language::Rust);
    }
    expanded
}

fn generate_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<Option<GeneratedFile>> {
    if matches!(lang, Language::Rust) && !rust_readme_explicitly_configured(config) {
        return Ok(None);
    }

    if matches!(lang, Language::C | Language::Jni) {
        return Ok(None);
    }

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

    Ok(Some(fallback::generate_readme_hardcoded(api, config, lang)?))
}

fn generate_readme_targets(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let Some(readme_cfg) = &config.readme else {
        return Ok(Vec::new());
    };
    if readme_cfg.targets.is_empty() {
        return Ok(Vec::new());
    }
    let workspace_root = config.workspace_root.clone().unwrap_or_else(|| PathBuf::from("."));
    let Some(template_dir) = &readme_cfg.template_dir else {
        anyhow::bail!("README targets require `crates.readme.template_dir`");
    };
    let abs_template_dir = workspace_root.join(template_dir);
    if !abs_template_dir.exists() {
        anyhow::bail!(
            "README template directory '{}' does not exist",
            abs_template_dir.display()
        );
    }

    let mut target_names = readme_cfg.targets.keys().cloned().collect::<Vec<_>>();
    target_names.sort();
    target_names
        .into_iter()
        .map(|target_name| {
            let target_json = readme_cfg
                .targets
                .get(&target_name)
                .ok_or_else(|| anyhow::anyhow!("README target '{target_name}' disappeared during generation"))?;
            template::render_target_readme(
                api,
                config,
                &target_name,
                target_json,
                readme_cfg,
                &workspace_root,
                &abs_template_dir,
            )
        })
        .collect()
}

fn push_unique_readme(
    files: &mut Vec<GeneratedFile>,
    seen_paths: &mut HashSet<PathBuf>,
    file: GeneratedFile,
) -> anyhow::Result<()> {
    if !seen_paths.insert(file.path.clone()) {
        anyhow::bail!(
            "duplicate README output path '{}'; configure unique `output_path` values",
            file.path.display()
        );
    }
    files.push(file);
    Ok(())
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
