//! `alef docs` handling, split out of `core_commands.rs` to keep that already
//! over-cap file from growing (`file-modularization`).

use anyhow::Result;
use std::path::PathBuf;

use crate::bin_cli::all_commands::stage_failures::StageFailures;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::bin_cli::helpers::{format_languages, load_config, resolve_doc_languages};
use crate::cli::{cache, dispatch, pipeline};

/// Write every rendered page, then stamp what was written, as one step.
///
/// The two halves are fused into a single function specifically so no caller can order them
/// wrongly or put a fallible step between them: writing a page and failing to stamp it is the
/// one combination that produces a permanently bad state. `alef verify` claims a file by the
/// marker in its leading lines and holds it to its embedded `alef:hash:`, while the scaffold
/// writer no-ops on unchanged content -- so a page written bare is reported drifted forever and
/// no later run re-enters the path that would stamp it (alef#475).
///
/// Returns the write report's changed count. ~keep
fn write_and_stamp(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
) -> Result<usize> {
    let report = pipeline::write_scaffold_files_report(files, base_dir, true)?;
    pipeline::report_refused_writes(&report);
    pipeline::report_user_owned_skips(&report);
    let doc_paths = pipeline::stampable_output_paths(files, base_dir);
    pipeline::finalize_hashes(&doc_paths, sources_hash, alef_toml_bytes)?;
    Ok(report.changed_count())
}

/// Handles `Commands::Docs`.
///
/// `skip_snippet_validation` selects between `docs::generate_docs_stage` and
/// `docs::generate_docs_stage_without_snippet_compile_validation` -- see that
/// function's doc comment for why the compile step is worth skipping on
/// purpose rather than just tolerating its failure. ~keep
pub(crate) fn handle(
    config_path: &std::path::Path,
    context: &DispatchContext,
    lang: Option<Vec<String>>,
    output: Option<String>,
    skip_snippet_validation: bool,
) -> Result<Option<Commands>> {
    let (_workspace, resolved) = load_config(config_path)?;
    let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
    let multi = dispatch::is_multi_crate(&crates_to_process);
    let base_dir = std::env::current_dir()?;
    let config_toml = std::fs::read_to_string(config_path)?;
    let mut grand_total: usize = 0;
    // A docs-stage failure is deferred into `StageFailures`, not returned with `?`, so the
    // terminal `finalize_hashes` below still runs for this crate and the loop still reaches
    // every later one. `Commands::Generate` (task #546) and `alef all` (task #186) were both
    // already converted; `alef docs` is the third instance of the same hazard. ~keep
    let mut stage_failures = StageFailures::new();
    for resolved_cfg in &crates_to_process {
        let languages = resolve_doc_languages(resolved_cfg, lang.as_deref())?;
        let selector = languages.iter().map(ToString::to_string).collect::<Vec<_>>().join("-");
        let output_key = output.as_deref().unwrap_or("default");
        let docs_stage_key = format!("docs-{}", cache::hash_content(&format!("{selector}:{output_key}")));
        let api = pipeline::extract(resolved_cfg, config_path, false)?;
        let ir_json = serde_json::to_string(&api)?;
        let stage_hash = cache::compute_stage_hash(&ir_json, &docs_stage_key, &config_toml, &[]);
        let use_stage_cache = resolved_cfg.docs.is_none();
        let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
        if use_stage_cache && cache::is_stage_cached(&resolved_cfg.name, &docs_stage_key, &stage_hash) {
            if multi {
                tracing::info!("[{}] Docs up to date (cached)", resolved_cfg.name);
            } else {
                tracing::info!("Docs up to date (cached)");
            }
            continue;
        }
        if multi {
            tracing::info!(
                "[{}] Generating docs for: {}",
                resolved_cfg.name,
                format_languages(&languages)
            );
        } else {
            tracing::info!("Generating docs for: {}", format_languages(&languages));
        }
        // `generate_docs_stage` hands back every page it rendered even when a later step
        // (snippet validation, CLI/MCP adoption, llms/skills) fails, specifically so a
        // strict-mode bail never discards already-rendered API reference pages. `files` goes
        // through `write_and_stamp` before `docs_result` is propagated, not after -- see that
        // function's doc for what a written-but-unstamped page costs (alef#475). ~keep
        let (files, docs_result) = if skip_snippet_validation {
            crate::docs::generate_docs_stage_without_snippet_compile_validation(
                &api,
                resolved_cfg,
                &languages,
                output.as_deref(),
                &base_dir,
            )
        } else {
            crate::docs::generate_docs_stage(&api, resolved_cfg, &languages, output.as_deref(), &base_dir)
        };
        grand_total += write_and_stamp(&files, &base_dir, &sources_hash, &alef_toml_bytes)?;
        if let Err(error) = docs_result {
            stage_failures.record(&format!("[{}] docs/snippet validation", resolved_cfg.name), error);
            continue;
        }
        if use_stage_cache {
            let output_paths: Vec<PathBuf> = files
                .iter()
                .filter(|file| file.carries_alef_marker())
                .map(|file| base_dir.join(&file.path))
                .collect();
            cache::write_stage_hash(&resolved_cfg.name, &docs_stage_key, stage_hash.as_str(), &output_paths)?;
        }
    }
    tracing::info!("Generated {grand_total} doc files");
    stage_failures.into_result()?;
    Ok(None)
}
