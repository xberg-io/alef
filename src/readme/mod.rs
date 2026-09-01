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
    validate_readme_snippets_dir(config)?;

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

/// A configured-but-unrendered `template` is fatal rather than falling back: substituting the
/// generic placeholder README discarded the configured content while `alef readme` still reported
/// success with 0 files skipped (#555). ~keep
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

    if let Some(file) = try_render_configured_readme(api, config, lang)? {
        return Ok(Some(file));
    }

    let lang_code = paths::lang_code(lang);
    if readme_language_declares_a_template(config, lang_code) {
        anyhow::bail!(
            "crates.readme.languages.{lang_code} names a `template`, but README generation did not render it. \
             Fix the template, `crates.readme.template_dir`, or the `crates.readme.languages.{lang_code}` entry \
             so it renders, or drop the entry's `template` key to accept the generic placeholder README."
        );
    }

    Ok(Some(fallback::generate_readme_hardcoded(api, config, lang)?))
}

/// Returns true when `crates.readme.languages.<lang_code>` names a `template`.
///
/// The discriminator is the `template` key, not the mere existence of the entry.
/// An entry that only relocates output (`{ "output_path": "..." }`) is asking for
/// the generic placeholder README *at a different path* — a legitimate, long-standing
/// configuration, and one the caller cannot express any other way, since deleting the
/// entry would also lose the path. Treating any entry as "must render a template"
/// makes that configuration impossible to write. Honouring the path is the hardcoded
/// generator's job, since that entry never reaches a template; it derived its own path
/// and discarded the configured one until `fallback::configured_output_path`.
///
/// An entry that does name a template is the #555 case: rendering it is the only
/// reason the entry exists, so failing to render must fail loudly rather than silently
/// ship the placeholder in place of the configured badges, sections, and snippets.
fn readme_language_declares_a_template(config: &ResolvedCrateConfig, lang_code: &str) -> bool {
    config.readme.as_ref().is_some_and(|readme_cfg| {
        readme_cfg
            .languages
            .get(lang_code)
            .and_then(|entry| entry.get("template"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|template| !template.trim().is_empty())
    })
}

/// Attempt to render a language's README from `crates.readme.template_dir`.
///
/// Returns `Ok(None)` whenever no template-based render was possible (no `readme`
/// config, no `template_dir`, the directory doesn't exist, or the language has no
/// entry and no legacy YAML `config` fallback), signalling the caller to either fall
/// back to the hardcoded generator (unconfigured languages) or fail loudly
/// (explicitly configured languages — see [`readme_language_explicitly_configured`]).
///
/// `None` is about the *content*, never the *path*: a configured `output_path` is not a
/// property of the template and survives every one of these cases, so the fallback must
/// still honour it (`paths::configured_output_path`). ~keep
fn try_render_configured_readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<Option<GeneratedFile>> {
    let Some(readme_cfg) = &config.readme else {
        return Ok(None);
    };
    let Some(template_dir) = &readme_cfg.template_dir else {
        return Ok(None);
    };
    let workspace_root = config.workspace_root.clone().unwrap_or_else(|| PathBuf::from("."));
    let abs_template_dir = workspace_root.join(template_dir);
    if !abs_template_dir.exists() {
        return Ok(None);
    }
    template::try_template_readme(api, config, lang, readme_cfg, &workspace_root, &abs_template_dir)
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
    // Warn, don't fail: this is a docs-tidiness signal (a dangling TOC entry), not a
    // correctness break, and alef is consumed by several repos mid-release -- a template
    // shape this scanner misjudges, or a section a consumer deliberately leaves empty, must
    // not turn into an unrelated hard build failure. ~keep
    let empty_headings = find_empty_headings(&file.content);
    if !empty_headings.is_empty() {
        tracing::warn!(
            path = %file.path.display(),
            headings = %empty_headings.join(", "),
            "README emits heading(s) with no body before the next same-or-shallower heading; \
             this renders a dangling entry in the page and its TOC -- populate the section, or \
             make the template omit the heading when the section has no content"
        );
    }
    files.push(file);
    Ok(())
}

/// Returns `Some(level)` (the number of leading `#`s, 1-6) when `line` is a Markdown ATX
/// heading, i.e. a run of 1-6 `#` characters followed by a space/tab or end of line.
fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then_some(level)
}

/// Find headings with no body before the next heading at the same or a shallower level.
///
/// A heading immediately followed by a *deeper* heading (e.g. `## Examples` grouping
/// `### Streaming Responses`) is a legitimate section grouper, not an empty section, so only
/// same-or-shallower next headings count as "no body". Lines inside fenced code blocks are
/// never treated as headings -- shell/Python/C comments (`# ...`, `#include ...`) are common
/// in the code examples these READMEs embed and must not be misread as Markdown structure --
/// but a fenced code block itself always counts as body content. ~keep
fn find_empty_headings(content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_code_block = false;
    let is_code: Vec<bool> = lines
        .iter()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_code_block = !in_code_block;
                true
            } else {
                in_code_block
            }
        })
        .collect();

    let mut empties = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_code[i] {
            continue;
        }
        let Some(level) = heading_level(line) else {
            continue;
        };
        let mut has_content = false;
        for (offset, next_line) in lines.iter().enumerate().skip(i + 1) {
            if !is_code[offset]
                && let Some(next_level) = heading_level(next_line)
            {
                has_content = next_level > level;
                break;
            }
            if !next_line.trim().is_empty() {
                has_content = true;
                break;
            }
        }
        if !has_content {
            empties.push(line.trim().to_string());
        }
    }
    empties
}

/// Validate that a configured `crates.readme.snippets_dir` actually exists.
///
/// Checked unconditionally, before any template renders, regardless of
/// whether the language templates currently reference the `include_snippet`
/// filter: a stale `snippets_dir` must fail the build even for languages
/// whose README template does not (yet, or ever) call the filter. A
/// misconfigured path here previously resolved every `include_snippet` call
/// to a silent `<!-- snippet not found -->` placeholder that shipped into
/// published READMEs while `alef readme` reported success. ~keep
fn validate_readme_snippets_dir(config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let Some(readme_cfg) = &config.readme else {
        return Ok(());
    };
    let workspace_root = config.workspace_root.clone().unwrap_or_else(|| PathBuf::from("."));
    let mut roots = readme_cfg
        .snippets_dir
        .iter()
        .map(|root| ("crates.readme.snippets_dir".to_string(), root.clone()))
        .collect::<Vec<_>>();
    roots.extend(readme_cfg.languages.iter().filter_map(|(language, entry)| {
        entry
            .get("snippets_dir")
            .and_then(serde_json::Value::as_str)
            .map(|root| {
                (
                    format!("crates.readme.languages.{language}.snippets_dir"),
                    PathBuf::from(root),
                )
            })
    }));
    for (key, snippets_dir) in roots {
        let abs_snippets_dir = workspace_root.join(&snippets_dir);
        if !abs_snippets_dir.exists() {
            anyhow::bail!(
                "config key `{key}` is set to '{}' (resolved to '{}'), which does not exist",
                snippets_dir.display(),
                abs_snippets_dir.display()
            );
        }
    }
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
