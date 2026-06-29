//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod context;
mod descriptions;
pub mod doc_cleaning;
mod examples;
mod formatting;
mod language_pages;
mod naming;
mod render;
mod rust_static;
mod shared_pages;
mod signatures;
mod sorting;
mod template_env;
#[cfg(test)]
mod tests;
mod type_mapping;
mod version_labels;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use doc_cleaning::clean_doc;
pub use type_mapping::doc_type;

pub use context::{CliSurface, DocsRenderContext, McpSurface};

/// Generate API reference documentation for the given languages.
///
/// Produces one `api-{lang}.md` per language, plus shared `configuration.md`,
/// `types.md`, and `errors.md` files written into `output_dir`.
pub fn generate_docs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_dir: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();
    let ffi_prefix = &config.ffi_prefix().to_pascal_case();

    for &lang in languages {
        files.push(language_pages::generate_lang_doc(
            api, config, lang, output_dir, ffi_prefix,
        )?);
    }

    files.push(shared_pages::generate_configuration_doc(api, config, output_dir)?);
    files.push(shared_pages::generate_types_doc(api, output_dir)?);
    files.push(shared_pages::generate_errors_doc(api, output_dir)?);

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

/// Generate the complete docs stage: API reference, optional CLI/MCP reference,
/// optional template-rendered llms.txt and skills, and configured snippet checks.
pub fn generate_docs_stage(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let reference_output = output_override
        .map(PathBuf::from)
        .or_else(|| config.docs.as_ref().and_then(|docs| docs.reference_output.clone()))
        .unwrap_or_else(|| PathBuf::from("docs/reference"));
    let reference_output_str = reference_output.to_string_lossy().to_string();

    let mut files = generate_docs(api, config, languages, &reference_output_str)?;
    for file in &mut files {
        file.content = with_markdown_alef_header(&file.content);
        file.generated_header = true;
    }

    let mut context = build_base_context(api, config, languages, &files);
    let snippet_dirs = build_snippet_context(config, workspace_root, &mut context)?;

    if let Some(docs_cfg) = &config.docs {
        if let Some(cli_cfg) = &docs_cfg.cli
            && cli_cfg.is_enabled()
        {
            let explicit_sources = !cli_cfg.sources.is_empty();
            let sources = docs_sources(config, &cli_cfg.sources, workspace_root);
            warn_missing_explicit_sources("CLI", &cli_cfg.sources, workspace_root);
            let surface = rust_static::extract_cli_surface(&sources)?;
            if surface.commands.is_empty() {
                if explicit_sources {
                    tracing::warn!("docs.cli was configured but no clap commands were discovered");
                }
            } else {
                let path = cli_cfg
                    .output
                    .clone()
                    .unwrap_or_else(|| reference_output.join("cli.md"));
                render::ensure_managed_or_adopted(workspace_root, &path, cli_cfg.adopt_existing)?;
                files.push(render::generate_cli_doc(&surface, path.clone()));
                context.references.push(context::ReferenceDoc {
                    kind: "cli".to_string(),
                    title: "CLI Reference".to_string(),
                    path: path.to_string_lossy().to_string(),
                });
                context.cli = surface;
            }
        }

        if let Some(mcp_cfg) = &docs_cfg.mcp
            && mcp_cfg.is_enabled()
        {
            let explicit_sources = !mcp_cfg.sources.is_empty();
            let sources = docs_sources(config, &mcp_cfg.sources, workspace_root);
            warn_missing_explicit_sources("MCP", &mcp_cfg.sources, workspace_root);
            let surface = rust_static::extract_mcp_surface(&sources)?;
            if surface.tools.is_empty() && surface.prompts.is_empty() && surface.resources.is_empty() {
                if explicit_sources {
                    tracing::warn!("docs.mcp was configured but no rmcp tools, prompts, or resources were discovered");
                }
            } else {
                let path = mcp_cfg
                    .output
                    .clone()
                    .unwrap_or_else(|| reference_output.join("mcp.md"));
                render::ensure_managed_or_adopted(workspace_root, &path, mcp_cfg.adopt_existing)?;
                files.push(render::generate_mcp_doc(&surface, path.clone()));
                context.references.push(context::ReferenceDoc {
                    kind: "mcp".to_string(),
                    title: "MCP Reference".to_string(),
                    path: path.to_string_lossy().to_string(),
                });
                context.mcp = surface;
            }
        }

        if let Some(llms_cfg) = &docs_cfg.llms {
            files.push(render::render_llms(llms_cfg, &context, workspace_root, &snippet_dirs)?);
        }

        if let Some(skills_cfg) = &docs_cfg.skills {
            files.extend(render::render_skills(
                skills_cfg,
                &context,
                workspace_root,
                &snippet_dirs,
            )?);
        }
    }

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

fn build_base_context(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    api_files: &[GeneratedFile],
) -> DocsRenderContext {
    let description = config
        .scaffold
        .as_ref()
        .and_then(|scaffold| scaffold.description.clone())
        .unwrap_or_else(|| format!("Bindings for {}", config.name));
    let license = config
        .scaffold
        .as_ref()
        .and_then(|scaffold| scaffold.license.clone())
        .unwrap_or_else(|| "MIT".to_string());
    let api_references = api_files
        .iter()
        .map(|file| {
            let path = file.path.to_string_lossy().to_string();
            context::ReferenceDoc {
                kind: "api".to_string(),
                title: path
                    .rsplit('/')
                    .next()
                    .unwrap_or(path.as_str())
                    .trim_end_matches(".md")
                    .replace('-', " "),
                path,
            }
        })
        .collect::<Vec<_>>();

    DocsRenderContext {
        krate: context::CrateDocsContext {
            name: config.name.clone(),
            version: api.version.clone(),
            description,
            repository: config.github_repo(),
            license,
        },
        languages: languages.iter().map(ToString::to_string).collect(),
        references: api_references.clone(),
        api_references,
        ..DocsRenderContext::default()
    }
}

fn build_snippet_context(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    context: &mut DocsRenderContext,
) -> anyhow::Result<Vec<PathBuf>> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Ok(Vec::new());
    };

    for dir in &snippet_cfg.dirs {
        if !workspace_root.join(dir).exists() {
            tracing::warn!("docs.snippets.dirs entry does not exist, skipping: {}", dir.display());
        }
    }
    let snippet_dirs = snippet_cfg
        .dirs
        .iter()
        .filter(|dir| workspace_root.join(dir).exists())
        .cloned()
        .collect::<Vec<_>>();
    if snippet_dirs.is_empty() {
        if snippet_cfg.validation_level.is_some() || !snippet_cfg.required_languages.is_empty() {
            tracing::warn!("docs.snippets is configured for validation but no snippet directories exist");
        }
        return Ok(Vec::new());
    }

    let absolute_snippet_dirs = snippet_dirs
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();
    let snippets = crate::snippets::discovery::discover_snippets(&absolute_snippet_dirs, None)?;
    let mut counts_by_language = BTreeMap::new();
    for snippet in &snippets {
        *counts_by_language.entry(snippet.language.to_string()).or_insert(0) += 1;
    }
    context.snippets = context::SnippetIndexContext {
        dirs: snippet_dirs
            .iter()
            .map(|dir| dir.to_string_lossy().to_string())
            .collect(),
        snippets: snippets
            .iter()
            .map(|snippet| context::SnippetContext {
                id: snippet.id.clone(),
                path: snippet.path.to_string_lossy().to_string(),
                language: snippet.language.to_string(),
                title: snippet.title.clone(),
                tags: snippet.metadata.tags.clone(),
            })
            .collect(),
        counts_by_language,
    };

    validate_snippets(config, workspace_root, snippet_cfg, &absolute_snippet_dirs, &snippets)?;
    Ok(snippet_dirs)
}

fn validate_snippets(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    snippet_cfg: &crate::core::config::DocsSnippetsConfig,
    absolute_snippet_dirs: &[PathBuf],
    snippets: &[crate::snippets::types::Snippet],
) -> anyhow::Result<()> {
    let docs_dirs = if snippet_cfg.docs_dirs.is_empty() {
        Vec::new()
    } else {
        snippet_cfg
            .docs_dirs
            .iter()
            .map(|dir| workspace_root.join(dir))
            .collect::<Vec<_>>()
    };
    let include_base_paths = snippet_cfg
        .include_base_paths
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();

    if !docs_dirs.is_empty() {
        let audit_report = crate::snippets::audit::audit(&crate::snippets::audit::AuditConfig {
            docs_dirs: docs_dirs.clone(),
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            require_frontmatter: snippet_cfg.require_frontmatter,
        });
        if audit_report.has_errors() {
            let summary = audit_report
                .issues
                .iter()
                .take(8)
                .map(|issue| format!("{}:{}: {}", issue.path.display(), issue.line, issue.message))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!("snippet audit failed for crate `{}`:\n{summary}", config.name);
        }
    }

    let required_languages = snippet_cfg
        .required_languages
        .iter()
        .map(|lang| lang.parse::<crate::snippets::types::Language>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| anyhow::anyhow!("invalid docs.snippets.required_languages entry: {err}"))?;

    if !docs_dirs.is_empty() || !required_languages.is_empty() {
        let report = crate::snippets::gaps::detect_gaps(&crate::snippets::gaps::GapConfig {
            docs_dirs,
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            required_languages,
            include_base_paths,
        })?;
        if !report.unreferenced_snippets.is_empty() {
            tracing::warn!(
                "docs.snippets found {} unreferenced snippet file(s); not failing because extra examples can be intentional",
                report.unreferenced_snippets.len()
            );
        }
        if !report.missing_references.is_empty()
            || !report.missing_language_variants.is_empty()
            || !report.skips_without_reason.is_empty()
            || !report.unknown_languages.is_empty()
        {
            anyhow::bail!("snippet gap validation failed for crate `{}`", config.name);
        }
    }

    if let Some(level) = &snippet_cfg.validation_level {
        let level = level
            .parse::<crate::snippets::types::ValidationLevel>()
            .map_err(|err| anyhow::anyhow!("invalid docs.snippets.validation_level: {err}"))?;
        let mut runner_cfg = crate::snippets::runner::RunnerConfig {
            level,
            fail_fast: snippet_cfg.fail_fast,
            ..crate::snippets::runner::RunnerConfig::default()
        };
        if let Some(timeout_secs) = snippet_cfg.timeout_secs {
            runner_cfg.timeout_secs = timeout_secs;
        }
        let registry = crate::snippets::validators::ValidatorRegistry::default();
        let summary = crate::snippets::runner::run_validation(snippets, &registry, &runner_cfg)?;
        if summary.unavailable > 0 {
            tracing::warn!(
                "docs.snippets skipped {} snippet validation(s) because required toolchains were unavailable",
                summary.unavailable
            );
        }
        if summary.has_failures() {
            anyhow::bail!(
                "snippet validation failed for crate `{}`: {} failed, {} errors",
                config.name,
                summary.failed,
                summary.errors
            );
        }
    }

    Ok(())
}

fn docs_sources(config: &ResolvedCrateConfig, configured_sources: &[PathBuf], workspace_root: &Path) -> Vec<PathBuf> {
    let sources = if configured_sources.is_empty() {
        config.source_hash_paths()
    } else {
        configured_sources.to_vec()
    };
    sources
        .into_iter()
        .map(|source| {
            if source.is_absolute() {
                source
            } else {
                workspace_root.join(source)
            }
        })
        .collect()
}

fn warn_missing_explicit_sources(kind: &str, sources: &[PathBuf], workspace_root: &Path) {
    let kind = kind.to_ascii_lowercase();
    for source in sources {
        if !workspace_root.join(source).exists() {
            tracing::warn!("docs.{kind} source does not exist, skipping: {}", source.display());
        }
    }
}

fn with_markdown_alef_header(content: &str) -> String {
    render::with_html_header(content.to_string())
}
