//! `alef snippets gaps` invocation shape and reporting.
//!
//! Split out of `cli::commands::snippets` when that file crossed the 1,000-line cap. The concept
//! boundary is the gaps subcommand specifically -- its invocation struct and its reporting -- as
//! distinct from the list/check/audit/parse subcommands that remain in the parent. ~keep

use super::reject_missing_configured_directories;
use crate::snippets::gaps::{GapConfig, detect_gaps};
use crate::snippets::types::Language;
use std::path::PathBuf;
use std::process::ExitCode;

/// One `alef snippets gaps` invocation, grouped so the call stays under clippy's argument
/// threshold.
pub(super) struct GapInvocation<'a> {
    pub(super) snippet_dirs: &'a [PathBuf],
    pub(super) docs_dirs: &'a [PathBuf],
    pub(super) required_languages: Option<&'a [String]>,
    /// The raw `--include-base-path` list, before the docs-root fallback. Unset-ness is only
    /// observable here. ~keep
    pub(super) include_base_paths: &'a [PathBuf],
    pub(super) strict: bool,
}

pub(super) fn run_gaps(invocation: &GapInvocation<'_>) -> ExitCode {
    let GapInvocation {
        snippet_dirs,
        docs_dirs,
        required_languages,
        include_base_paths,
        strict,
    } = *invocation;
    if let Err(code) = reject_missing_configured_directories(snippet_dirs, docs_dirs) {
        return code;
    }
    // An unrecognised `--required-languages` value must not silently drop out of the parity
    // check: it used to (`Language::from_fence_tag` returning `Unknown` was filtered away with
    // no message), so a typo -- or reaching for a session target name like `kotlin_android`
    // instead of its fence tag `kotlin` -- quietly shrank the comparison instead of failing it.
    // ~keep
    let required: Vec<Language> = match required_languages
        .map(|languages| {
            languages
                .iter()
                .map(|language| crate::snippets::types::resolve_required_language(language))
                .collect::<Result<Vec<Language>, String>>()
        })
        .transpose()
    {
        Ok(required) => required.unwrap_or_default(),
        Err(error) => {
            tracing::error!("invalid --required-languages entry: {error}");
            return ExitCode::FAILURE;
        }
    };
    let resolved_base_paths: Vec<PathBuf> = if include_base_paths.is_empty() {
        docs_dirs.to_vec()
    } else {
        include_base_paths.to_vec()
    };
    let configured_references = match crate::snippets::gaps::coverage_ledger_references(snippet_dirs) {
        Ok(references) => references,
        Err(error) => {
            tracing::error!("reading generated snippet coverage: {error}");
            return ExitCode::FAILURE;
        }
    };
    let config = GapConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        required_languages: required,
        include_base_paths: resolved_base_paths,
        configured_references,
        exclude: Vec::new(),
    };
    let report = match detect_gaps(&config) {
        Ok(report) => report,
        Err(err) => {
            tracing::error!("detecting gaps: {err}");
            return ExitCode::FAILURE;
        }
    };
    // `include_base_paths` is only reported unset when the run actually found a `--8<--` target
    // that would have used it -- an Astro/MDX-only docs tree never does. ~keep
    let unset = crate::snippets::gap_coverage::unset_gap_inputs(
        docs_dirs,
        &config.required_languages,
        include_base_paths,
        report.coverage.mkdocs_include_references,
    );
    // Printed on every run, gaps or none. A coverage report that appears only alongside
    // findings is absent from precisely the runs whose scope a reader needs to weigh. ~keep
    for line in report.coverage.report_lines() {
        crate::bin_cli::output::line(line);
    }
    for line in crate::snippets::gap_coverage::unset_input_lines(&unset, strict) {
        crate::bin_cli::output::line(line);
    }
    let unconfigured_failure = strict && crate::snippets::gap_coverage::has_vacuous_input(&unset);
    if unconfigured_failure {
        // A clean result proves nothing when a check class had nothing to compare, so `--strict`
        // must not report one. ~keep
        tracing::error!(
            "--strict: an unset input above left a check class with nothing to compare; the gap check \
             cannot pass unconfigured"
        );
    }
    if !report.has_gaps() {
        crate::bin_cli::output::line("No gaps found.");
        return if unconfigured_failure {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }
    if !report.missing_references.is_empty() {
        crate::bin_cli::output::line(format!(
            "Missing include targets ({}):",
            report.missing_references.len()
        ));
        for reference in &report.missing_references {
            crate::bin_cli::output::line(format!(
                "  {}:{} → {}",
                reference.source.display(),
                reference.line,
                reference.target.display()
            ));
        }
    }
    if !report.unreferenced_snippets.is_empty() {
        crate::bin_cli::output::line(format!(
            "Unreferenced snippets ({}):",
            report.unreferenced_snippets.len()
        ));
        for path in &report.unreferenced_snippets {
            crate::bin_cli::output::line(format!("  {}", path.display()));
        }
    }
    if !report.missing_language_variants.is_empty() {
        crate::bin_cli::output::line(format!(
            "Missing language variants ({}):",
            report.missing_language_variants.len()
        ));
        for variant in &report.missing_language_variants {
            crate::bin_cli::output::line(format!("  {} — {}", variant.group.display(), variant.language));
        }
    }
    if !report.skips_without_reason.is_empty() {
        crate::bin_cli::output::line(format!("Skips without reason ({}):", report.skips_without_reason.len()));
        for location in &report.skips_without_reason {
            crate::bin_cli::output::line(format!(
                "  {}:{} (block {})",
                location.path.display(),
                location.line,
                location.block_index
            ));
        }
    }
    if !report.unknown_languages.is_empty() {
        crate::bin_cli::output::line(format!("Unknown languages ({}):", report.unknown_languages.len()));
        for unknown in &report.unknown_languages {
            crate::bin_cli::output::line(format!(
                "  {}:{} tag={}",
                unknown.path.display(),
                unknown.line,
                unknown.tag
            ));
        }
    }
    // Mirrors `run_check`/`run_configured_audit_and_gaps`: structural findings (missing include
    // targets, missing language variants, undocumented skips, unknown fence languages) always
    // fail; an unreferenced-only finding fails only under `strict`, same as `check`. This used
    // to fail unconditionally on ANY finding here, so `gaps` and `check` disagreed about the
    // identical unreferenced-snippet-only case. ~keep
    if report.is_failure(strict) || unconfigured_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
