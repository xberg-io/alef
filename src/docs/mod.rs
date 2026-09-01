//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

mod context;
mod descriptions;
pub mod doc_cleaning;
mod enum_variant_ref;
mod examples;
mod formatting;
pub(crate) mod language_pages;
pub(crate) mod naming;
mod render;
mod rust_static;
pub(crate) mod rust_types;
mod shared_pages;
mod signatures;
pub(crate) mod snippet_summary;
mod sorting;
pub(crate) mod template_env;
#[cfg(test)]
mod tests;
mod type_mapping;
mod version_labels;

pub(crate) use snippet_summary::enforce_snippet_summary;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use doc_cleaning::clean_doc;
pub use type_mapping::doc_type;

pub use context::{CliSurface, DocsRenderContext, McpSurface};
/// Shared with `crate::readme` so README output gets the same self-embedded
/// HTML-comment marker as generated docs pages -- see `render::with_html_header`'s
/// own doc for why. ~keep
pub(crate) use render::with_html_header;

/// The cfg-feature set the shared, language-neutral pages (`configuration.md`, `types.md`,
/// `errors.md`) are filtered against: the union of every language `config` CONFIGURES
/// (`config.languages`), never the `languages` a single `generate_docs` call was asked to
/// RENDER. Those are different questions -- `alef docs --lang python` renders only the Python
/// page for that one invocation, but the shared pages describe the surface across every
/// binding the project configures, so a CLI filter that narrows what gets rendered must not
/// also narrow what the shared pages describe. Deriving this from the rendered `languages`
/// argument previously meant `alef docs --lang python` could silently drop a cfg item that only
/// a *different* configured binding (e.g. `wasm`) enables. Reuses
/// [`language_pages::effective_docs_features`], the same per-language derivation
/// `generate_lang_doc` filters its own page with, so the two surfaces can never independently
/// drift on what one configured language makes reachable. ~keep
fn canonical_docs_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
    let mut canonical_features: HashSet<String> = HashSet::new();
    let mut has_configured_language = false;

    for &lang in &config.languages {
        // Mirrors the `Language::C | Language::Jni` skip in `generate_docs`'s own render loop
        // (see that loop's comment for why neither owns a reference page): neither is ever a
        // real entry an operator configures either, but skip explicitly rather than assume. ~keep
        if matches!(lang, Language::C | Language::Jni) {
            continue;
        }
        has_configured_language = true;
        canonical_features.extend(language_pages::effective_docs_features(api, config, lang));
    }

    // No configured language means there is nothing to prove any feature reachable through, so
    // the union above cannot be trusted to mean "nothing is enabled" -- fall back to the
    // unfiltered surface rather than let an empty set filter every cfg-gated item out. ~keep
    if !has_configured_language {
        return api.clone();
    }
    let enabled_features: HashSet<&str> = canonical_features.iter().map(String::as_str).collect();
    api.with_cfg_filtered_deep(&enabled_features)
}

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
        // `Language::C` is an e2e consumer target, not a generated binding, and
        // `Language::Jni` is an internal Rust shim crate paired with `kotlin_android`
        // (see the `Language` variant docs in core/config/extras.rs) -- neither owns a
        // public API reference page of its own. `readme::generate_readme` already skips
        // both for the same reason. Both slug to `"c"` in `naming::lang_slug`, so
        // rendering a page for them here raced `Language::Ffi` -- the actual generated C
        // binding -- for `api-c.md`, and for `Jni` specifically the content was never
        // even the right ABI to begin with (see the long comment on
        // `examples::sample_param_value`'s Jni arm). ~keep
        if matches!(lang, Language::C | Language::Jni) {
            continue;
        }
        files.push(language_pages::generate_lang_doc(
            api, config, lang, output_dir, ffi_prefix,
        )?);
    }

    let canonical_api = &canonical_docs_api(api, config);
    files.push(shared_pages::generate_configuration_doc(
        canonical_api,
        config,
        output_dir,
    )?);
    files.push(shared_pages::generate_types_doc(canonical_api, config, output_dir)?);
    files.push(shared_pages::generate_errors_doc(canonical_api, output_dir)?);

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}

/// The reference-docs output directory for `config` — `[docs].reference_output`
/// or the `docs/reference` default. Relative to the workspace root; callers join
/// it under the base directory.
///
/// Exposed so the generate pipeline can protect committed reference pages from
/// orphan cleanup: the page set `generate_docs_stage` emits depends on host
/// state (CLI/MCP source presence, doc languages), so a host that produces fewer
/// pages must not delete the committed ones it simply did not regenerate (#184). ~keep
pub fn reference_output_dir(config: &ResolvedCrateConfig) -> PathBuf {
    config
        .docs
        .as_ref()
        .and_then(|docs| docs.reference_output.clone())
        .unwrap_or_else(|| PathBuf::from("docs/reference"))
}

/// Generate the complete docs stage and hand back everything rendered, paired with whatever
/// error (if any) stopped it going further.
///
/// The `Vec<GeneratedFile>` is never dropped on failure: the 15+ API reference pages plus
/// `configuration.md`/`types.md`/`errors.md` are fully rendered before snippet discovery,
/// snippet validation, CLI/MCP adoption checks, or llms/skills rendering ever run, and every one
/// of those can fail for reasons that have nothing to do with the reference pages themselves (a
/// strict snippet-validation bail, an unmanaged `llms.txt`, a missing `docs.snippets.dirs` root).
/// Discarding the whole `Vec` on any of those used to mean a single strict-mode snippet failure
/// silently froze the *entire* published API reference at whatever version last validated
/// cleanly — worse than the failure it was trying to report, and with no signal to the caller
/// that anything was skipped. Callers must write `.0` unconditionally and only then propagate
/// `.1`.
///
/// That guarantee is not limited to the API reference pages: `generate_docs_stage_extras` orders
/// its steps so `cli.md`, `mcp.md`, `llms.txt` and every `SKILL.md` are emitted *before* snippet
/// validation runs, precisely so a strict bail cannot amputate the returned set. See that
/// function's doc for the false-orphan residue that ordering fixes. ~keep
pub fn generate_docs_stage(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    generate_docs_stage_impl(api, config, languages, output_override, workspace_root, true)
}

/// Same as [`generate_docs_stage`], except it never spawns a compiler, interpreter, or
/// type-checker for a snippet.
///
/// The only caller is `bin_cli::helpers::collect_managed_surface`, which feeds both `alef
/// adopt`'s candidate set and `alef verify`'s frozen-file report. Neither asks whether a
/// snippet compiles, runs, or type-checks -- they ask "what file surface does alef's
/// configuration own", and `validate_snippets`'s compile/type-check/run step (behind
/// `docs.snippets.validation_level`) produces zero [`GeneratedFile`]s; it only decides
/// whether `generate_docs_stage_extras` returns `Ok` or `Err`, and `collect_managed_surface`
/// already downgrades that `Err` to a debug log and keeps every page regardless (see this
/// module's other doc comments) -- so the compile step's outcome was already discarded
/// there, at the cost of spawning one subprocess per snippet per configured backend. On a
/// real repo that is thousands of `zig`/`go`/`javac`/`dotnet`/... invocations to answer an
/// ownership question about a single file, which is what made `alef adopt` on one `Cargo.toml`
/// take 90 minutes instead of seconds. Snippet discovery, the reference audit, and gap
/// detection are unaffected: they are pure in-memory/filesystem checks with nothing to spawn,
/// so they still run and their failures are still tolerated exactly as before. ~keep
pub fn generate_docs_stage_without_snippet_compile_validation(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    generate_docs_stage_impl(api, config, languages, output_override, workspace_root, false)
}

fn generate_docs_stage_impl(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_override: Option<&str>,
    workspace_root: &Path,
    run_snippet_compile_validation: bool,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    let reference_output = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| reference_output_dir(config));
    let reference_output_str = reference_output.to_string_lossy().to_string();

    let mut files = match generate_docs(api, config, languages, &reference_output_str) {
        Ok(files) => files,
        Err(err) => return (Vec::new(), Err(err)),
    };
    for file in &mut files {
        file.content = with_markdown_alef_header(&file.content);
        file.generated_header = true;
    }

    let result = generate_docs_stage_extras(
        api,
        config,
        languages,
        workspace_root,
        &reference_output,
        &mut files,
        run_snippet_compile_validation,
    );

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    (files, result)
}

/// The one way a page enters the docs stage's emitted set.
///
/// `files` is not a convenience buffer, it is the record every downstream consumer of this stage
/// reads to decide what alef owns: `write_scaffold_files_report` writes it and records each
/// unmarkable path in `.alef-ownership.toml`, `alef all`/`alef docs` derive the orphan sweep's
/// `keep` set from it, and `bin_cli::helpers::collect_managed_surface` hands it to `alef verify`'s
/// frozen report and `alef adopt`'s candidate set. A page missing from it is indistinguishable, to
/// all of them, from a file alef has stopped emitting — an orphan.
///
/// Pairing the two appends in one function, instead of leaving each caller to remember a separate
/// `context.references.push`, is the point: the separate call is exactly the shape that left
/// `llms.txt` and every `SKILL.md` out of `references` while `cli.md`/`mcp.md` were in it. Passing
/// `None` is now a visible decision at the call site rather than a forgotten line. ~keep
fn emit_page(
    files: &mut Vec<GeneratedFile>,
    context: &mut DocsRenderContext,
    file: GeneratedFile,
    reference: Option<(&str, &str)>,
) {
    if let Some((kind, title)) = reference {
        context.references.push(context::ReferenceDoc {
            kind: kind.to_string(),
            title: title.to_string(),
            path: file.path.to_string_lossy().to_string(),
        });
    }
    files.push(file);
}

/// Everything past the API reference pages: CLI/MCP extraction and adoption checks, snippet
/// discovery, llms/skills rendering, and snippet validation. Takes `files` by mutable reference
/// specifically so an early `?` return here only unwinds this function — `generate_docs_stage`'s
/// `files` keeps every page pushed onto it before the failure. ~keep
///
/// THE ORDER OF THE STEPS BELOW IS LOAD-BEARING, and is the whole reason this function reads the
/// way it does: every step that *emits* a page runs before the fallible step that emits none.
/// Snippet validation used to run first (it was fused into `build_snippet_context`), so any
/// strict bail, gap failure or audit failure — none of which say anything about the CLI or MCP
/// surface — returned before `cli.md`, `mcp.md`, `llms.txt` and every `SKILL.md` were ever pushed.
/// `generate_docs_stage`'s "the Vec is never dropped on failure" guarantee then covered only the
/// API reference pages, and the pages that vanished were *silently* absent rather than reported:
/// downstream, absence from this Vec is read as "alef no longer emits this", so a validation
/// failure in one part of the docs stage manufactured false orphans in another. Two consumer
/// repos show exactly that residue — `cli.md`/`mcp.md`/`SKILL.md` present on disk, alef-marked,
/// and absent from `.alef-ownership.toml` while every version-bearing API page is listed.
///
/// Anything fallible added here must go after the last `emit_page`, or be an emitter itself. ~keep
fn generate_docs_stage_extras(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    workspace_root: &Path,
    reference_output: &Path,
    files: &mut Vec<GeneratedFile>,
    run_snippet_compile_validation: bool,
) -> anyhow::Result<()> {
    let mut context = build_base_context(api, config, languages, files.as_slice());
    let Some(docs_cfg) = &config.docs else {
        return Ok(());
    };

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
            let page = render::generate_cli_doc(&surface, path);
            emit_page(files, &mut context, page, Some(("cli", "CLI Reference")));
            context.cli = surface;
        }
    }

    if let Some(mcp_cfg) = &docs_cfg.mcp
        && mcp_cfg.is_enabled()
    {
        let explicit_sources = !mcp_cfg.sources.is_empty();
        let sources = docs_sources(config, &mcp_cfg.sources, workspace_root);
        warn_missing_explicit_sources("MCP", &mcp_cfg.sources, workspace_root);
        let surface = rust_static::extract_mcp_surface(&sources, &mcp_cfg.declared)?;
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
            let page = render::generate_mcp_doc(&surface, path);
            emit_page(files, &mut context, page, Some(("mcp", "MCP Reference")));
            context.mcp = surface;
        }
    }

    let snippets = build_snippet_context(config, workspace_root, &mut context)?;
    let snippet_dirs: &[PathBuf] = snippets.as_ref().map_or(&[], |stage| stage.dirs.as_slice());

    if let Some(llms_cfg) = &docs_cfg.llms {
        let page = render::render_llms(llms_cfg, &context, workspace_root, snippet_dirs)?;
        emit_page(files, &mut context, page, None);
    }

    if let Some(skills_cfg) = &docs_cfg.skills {
        let pages = render::render_skills(skills_cfg, &context, workspace_root, snippet_dirs)?;
        for page in pages {
            emit_page(files, &mut context, page, None);
        }
    }

    if let Some(stage) = &snippets {
        validate_snippets(
            config,
            workspace_root,
            stage.config,
            &stage.absolute_dirs,
            &stage.snippets,
            run_snippet_compile_validation,
        )?;
    }

    Ok(())
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

/// The discovered snippet corpus, carried from discovery to validation.
///
/// Discovery has to happen before `llms.txt`/`SKILL.md` render (it populates `context.snippets`
/// and supplies the roots the `include_snippet` filter searches); validation must happen after
/// they have been emitted. Holding the corpus in one value is what lets the two sit on opposite
/// sides of the emitting steps without discovering twice. ~keep
struct SnippetStage<'cfg> {
    config: &'cfg crate::core::config::DocsSnippetsConfig,
    dirs: Vec<PathBuf>,
    absolute_dirs: Vec<PathBuf>,
    snippets: Vec<crate::snippets::types::Snippet>,
}

/// Discover the configured snippet corpus and record it in `context`.
///
/// `Ok(None)` means there is nothing to validate later either — no `docs.snippets` section, or a
/// section with no discovery roots at all. Both cases previously returned an empty dir list and
/// skipped validation; returning `None` keeps that coupling explicit instead of leaving the
/// deferred `validate_snippets` call to re-derive it from an empty vector. ~keep
fn build_snippet_context<'cfg>(
    config: &'cfg ResolvedCrateConfig,
    workspace_root: &Path,
    context: &mut DocsRenderContext,
) -> anyhow::Result<Option<SnippetStage<'cfg>>> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Ok(None);
    };

    for dir in snippet_cfg.dirs.iter().chain(&snippet_cfg.inline_dirs) {
        let abs_dir = workspace_root.join(dir);
        if !abs_dir.exists() {
            anyhow::bail!(
                "configured docs.snippets.dirs root '{}' (resolved to '{}') does not exist",
                dir.display(),
                abs_dir.display()
            );
        }
    }
    let snippet_dirs = snippet_cfg.dirs.clone();
    let discovery_dirs = snippet_cfg
        .dirs
        .iter()
        .chain(&snippet_cfg.inline_dirs)
        .cloned()
        .collect::<Vec<_>>();
    if discovery_dirs.is_empty() {
        if snippet_cfg.validation_level.is_some() || !snippet_cfg.required_languages.is_empty() {
            tracing::warn!("docs.snippets is configured for validation but docs.snippets.dirs is empty");
        }
        return Ok(None);
    }

    let absolute_snippet_dirs = snippet_dirs
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();
    let absolute_discovery_dirs = discovery_dirs
        .iter()
        .map(|dir| workspace_root.join(dir))
        .collect::<Vec<_>>();
    let excluded = snippet_cfg
        .exclude
        .iter()
        .map(|path| workspace_root.join(path))
        .collect::<Vec<_>>();
    let snippets = crate::snippets::discovery::discover_snippets(&absolute_discovery_dirs, None)?
        .into_iter()
        .filter(|snippet| !excluded.iter().any(|prefix| snippet.path.starts_with(prefix)))
        .collect::<Vec<_>>();
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

    Ok(Some(SnippetStage {
        config: snippet_cfg,
        dirs: snippet_dirs,
        absolute_dirs: absolute_snippet_dirs,
        snippets,
    }))
}

fn validate_snippets(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    snippet_cfg: &crate::core::config::DocsSnippetsConfig,
    absolute_snippet_dirs: &[PathBuf],
    snippets: &[crate::snippets::types::Snippet],
    run_snippet_compile_validation: bool,
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
    let include_base_paths = if snippet_cfg.include_base_paths.is_empty() {
        docs_dirs.clone()
    } else {
        snippet_cfg
            .include_base_paths
            .iter()
            .map(|dir| workspace_root.join(dir))
            .collect::<Vec<_>>()
    };
    let exclude = snippet_cfg
        .exclude
        .iter()
        .map(|path| workspace_root.join(path))
        .collect::<Vec<_>>();
    let mut configured_references =
        crate::snippets::gaps::readme_snippet_references(workspace_root, config.readme.as_ref());
    configured_references.extend(crate::snippets::gaps::coverage_ledger_references(
        absolute_snippet_dirs,
    )?);
    let content_collections = snippet_cfg
        .content_collections
        .iter()
        .map(|(name, root)| (name.clone(), workspace_root.join(root)))
        .collect();
    configured_references.extend(crate::snippets::gaps::astro_collection_references(
        &docs_dirs,
        &content_collections,
    )?);

    if !docs_dirs.is_empty() {
        let audit_report = crate::snippets::audit::audit(&crate::snippets::audit::AuditConfig {
            docs_dirs: docs_dirs.clone(),
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            include_base_paths: include_base_paths.clone(),
            configured_references: configured_references.clone(),
            exclude: exclude.clone(),
            require_frontmatter: snippet_cfg.require_frontmatter,
            // `alef validate`'s snippet gate is a structural check; curated accounting belongs
            // to `alef snippets audit --config`, which reads the declaration directly. ~keep
            accounting: crate::snippets::audit::SnippetAccounting::default(),
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
        .map(|lang| crate::snippets::types::resolve_required_language(lang))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| anyhow::anyhow!("invalid docs.snippets.required_languages entry: {err}"))?;

    if !docs_dirs.is_empty() || !required_languages.is_empty() {
        let report = crate::snippets::gaps::detect_gaps(&crate::snippets::gaps::GapConfig {
            docs_dirs,
            snippet_dirs: absolute_snippet_dirs.to_vec(),
            required_languages,
            include_base_paths,
            configured_references,
            exclude,
        })?;
        if !report.unreferenced_snippets.is_empty() && snippet_cfg.strict {
            anyhow::bail!(
                "strict snippet coverage failed for crate `{}`: {} unreferenced snippet file(s)",
                config.name,
                report.unreferenced_snippets.len()
            );
        }
        if !report.unreferenced_snippets.is_empty() {
            // Warn, not fail, outside strict mode: extra examples can be intentional. ~keep
            tracing::warn!(
                "docs.snippets found {} unreferenced snippet file(s)",
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

    // Gated separately from discovery, audit, and gap detection above: this block is the
    // one that spawns a toolchain subprocess per snippet per configured backend, and it is
    // the only reason to ever skip it. See
    // `generate_docs_stage_without_snippet_compile_validation`'s doc for why a caller would
    // need `run_snippet_compile_validation = false`. ~keep
    if run_snippet_compile_validation && let Some(level) = &snippet_cfg.validation_level {
        let level = level
            .parse::<crate::snippets::types::ValidationLevel>()
            .map_err(|err| anyhow::anyhow!("invalid docs.snippets.validation_level: {err}"))?;
        // The one place this phase boundary is announced at runtime, not just in `--help` text:
        // a consumer reaching for `alef all --clean` reasonably expects "generate, no build", and
        // this block is the one that quietly spawns a real compiler/type-checker/interpreter per
        // session instead. `--skip-snippet-validation` and this block's own doc comment above
        // already explain the boundary to anyone who reads `--help` first; this line is for
        // everyone else, at the one moment the cost is about to be paid. ~keep
        tracing::info!(
            crate_name = %config.name,
            snippet_count = snippets.len(),
            level = %level,
            "starting snippet compile validation: spawns a real toolchain per session/snippet; pass \
             --skip-snippet-validation for a generate-only run"
        );
        let mut runner_cfg = crate::snippets::runner::RunnerConfig {
            level,
            fail_fast: snippet_cfg.fail_fast,
            deny_unclassified: snippet_cfg.deny_unclassified,
            allowed_side_effects: parse_allowed_side_effects(&snippet_cfg.allowed_side_effects)?,
            cache_dir: Some(workspace_root.join(snippet_cfg.cache_dir())),
            // Paired with `cache_dir` deliberately: `RunnerConfig::default()` leaves `changed_only`
            // off, so this path wrote a cache entry for every snippet of every run and never read
            // one back — a guaranteed 100% miss on the stage that dominates `alef all`. Cache
            // entries are keyed on the snippet's own content, its annotation, the session
            // fingerprint, and this same side-effect policy (see
            // `ValidationCache::invalidation_key`), so a regenerated binding, an edited snippet or
            // annotation, or an `allowed_side_effects`/`deny_unclassified` edit all still
            // revalidate. ~keep
            changed_only: true,
            sessions: snippet_cfg
                .sessions
                .iter()
                .map(|(target, session)| {
                    let normalized = crate::snippets::types::Language::normalize_session_target(target);
                    let language = crate::snippets::types::Language::from_session_target(&normalized);
                    if language == crate::snippets::types::Language::Unknown {
                        anyhow::bail!("unknown docs.snippets session target `{target}`");
                    }
                    let mut rust_features = session.rust_features.clone();
                    if language == crate::snippets::types::Language::Rust {
                        rust_features.extend(config.features.iter().cloned());
                        rust_features.sort();
                        rust_features.dedup();
                    }
                    Ok((
                        normalized,
                        crate::snippets::session::SessionSpec {
                            language,
                            working_directory: workspace_root.join(&session.cwd),
                            manifest: session.manifest.as_ref().map(|path| workspace_root.join(path)),
                            before: session.before.clone(),
                            env: session.env.clone(),
                            include_paths: session
                                .include_paths
                                .iter()
                                .map(|path| workspace_root.join(path))
                                .collect(),
                            rust_features,
                            rust_dependencies: session.rust_dependencies.clone(),
                        },
                    ))
                })
                .collect::<anyhow::Result<_>>()?,
            ..crate::snippets::runner::RunnerConfig::default()
        };
        if let Some(timeout_secs) = snippet_cfg.timeout_secs {
            runner_cfg.timeout_secs = timeout_secs;
        }
        runner_cfg.before_timeout_secs = snippet_cfg.before_timeout_secs;
        let registry = crate::snippets::validators::ValidatorRegistry::default();
        let summary = crate::snippets::runner::run_validation(snippets, &registry, &runner_cfg)?;
        // Write the report before any strict bail. A run that fails strict mode is precisely the
        // run whose report a consumer needs, and emitting it afterwards meant the artifact was
        // never produced in that case. ~keep
        if let Some(path) = &snippet_cfg.report_output {
            let report_path = workspace_root.join(path);
            crate::snippets::output::write_report(&summary, &report_path, false).map_err(|err| {
                anyhow::anyhow!(
                    "writing snippet validation report to '{}': {err}",
                    report_path.display()
                )
            })?;
        }
        enforce_snippet_summary(&config.name, snippet_cfg.strict, &summary)?;
    }

    Ok(())
}

fn parse_allowed_side_effects(configured: &[String]) -> anyhow::Result<Vec<crate::snippets::types::SideEffectClass>> {
    configured
        .iter()
        .map(|value| match value.as_str() {
            "safe" => Ok(crate::snippets::types::SideEffectClass::Safe),
            "network" => Ok(crate::snippets::types::SideEffectClass::Network),
            "process" => Ok(crate::snippets::types::SideEffectClass::Process),
            "install" => Ok(crate::snippets::types::SideEffectClass::Install),
            "server" => Ok(crate::snippets::types::SideEffectClass::Server),
            _ => anyhow::bail!("invalid docs.snippets.allowed_side_effects entry: `{value}`"),
        })
        .collect()
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
    render::with_html_header(content.to_string(), "alef docs")
}
