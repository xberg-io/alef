//! `alef snippets` subcommand — discover, validate, audit, and gap-check documentation snippets.

use crate::snippets::audit::{AuditConfig, AuditSeverity, audit};
use crate::snippets::discovery;
use crate::snippets::gaps::{GapConfig, detect_gaps};
use crate::snippets::output;
use crate::snippets::runner::{RunnerConfig, run_validation};
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{Language, SideEffectClass, SnippetStatus, ValidationLevel};
use crate::snippets::validators::ValidatorRegistry;
use accounting::{AuditOutcome, accounting_scope_line, audit_outcome, configured_curated_paths};
use clap::Subcommand;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum SnippetsAction {
    /// List discovered snippets and a per-language count summary.
    List {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, value_delimiter = ',')]
        languages: Option<Vec<String>>,
    },

    /// Run the configured snippet discovery, validation, audit, and gap checks.
    Check {
        #[arg(short, long, default_value = "alef.toml")]
        config: PathBuf,
        /// Union'd with `[crates.docs.snippets].strict` (either one is enough): treats coverage
        /// gaps, unavailable/downgraded checks, unreferenced snippets, and unclassified
        /// side-effect snippets as failures. Equivalent to setting `strict = true` in config —
        /// use the flag for one-off CI runs, the config field to make it the project default.
        #[arg(long)]
        strict: bool,
        #[arg(long, default_value = "on", value_parser = ["on", "off"])]
        cache: String,

        /// Validate only these languages, by fence tag (`--lang go --lang zig`, or
        /// `--lang go,zig`).
        ///
        /// Diagnosing one language's snippets otherwise means paying for all of them: a full
        /// consumer tree is thousands of snippets across sixteen toolchains, and every
        /// iteration on a single backend's codegen re-ran the lot. The audit and gap passes
        /// still see the whole corpus, because both are cross-language questions — an
        /// unreferenced snippet or a missing language variant cannot be judged from a subset.
        /// ~keep
        #[arg(long = "lang", value_delimiter = ',', num_args = 1..)]
        languages: Option<Vec<String>>,

        /// Override `[docs.snippets].validation_level` for this run (`syntax`, `typecheck`,
        /// `compile`, or `run`).
        ///
        /// `alef all`/`alef docs` never build a language's real artifacts in the same
        /// invocation (see `docs::enforce_snippet_summary`'s doc comment), so a
        /// `compile`/`typecheck`/`run` `validation_level` reliably downgrades to
        /// `unresolved_dependency` there until a separate `alef build` runs. This flag is the
        /// explicit way to ask for real compile-level checking after that build, without
        /// weakening `validation_level` for every other caller of this config (task #542). ~keep
        #[arg(long)]
        level: Option<String>,
    },

    /// Parse a single file and print its code blocks.
    Parse { file: PathBuf },

    /// Structural integrity audit (frontmatter, fences, include targets).
    Audit {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        #[arg(long)]
        require_frontmatter: bool,

        /// Read `[crates.e2e.snippets].curated_snippets` from this config to tell a
        /// deliberately hand-authored snippet apart from an unaccounted coverage gap.
        ///
        /// Optional, unlike `check`'s `--config`, because `audit`'s inputs are the explicit
        /// `--snippets`/`--docs` roots and every structural check runs without configuration.
        /// Only the accounting pass needs it — and it is skipped, by name, when this is unset,
        /// rather than silently reporting every hand-authored file as a gap or reporting a
        /// clean accounting it never computed. ~keep
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Coverage gap report (unreferenced snippets, missing language variants).
    Gaps {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        /// Languages every language-grouped snippet must provide, either as a snippet fence
        /// tag (`python`, `go`, `kotlin`, ...) or a session target name (`kotlin_android`,
        /// `node`, `wasm`, ...).
        #[arg(short = 'L', long, value_delimiter = ',')]
        required_languages: Option<Vec<String>>,

        /// Additional base paths to search when resolving `--8<--` include targets.
        ///
        /// Mirrors the `pymdownx.snippets` `base_path` list. Each target is
        /// resolved against these paths in order; the first match wins. When
        /// unset, only the docs root is searched (preserving the prior behaviour).
        #[arg(long = "include-base-path", num_args = 0..)]
        include_base_paths: Vec<PathBuf>,

        /// Fail instead of passing when an input the verdict depends on is unset.
        ///
        /// A CI job whose entire purpose is gap detection must not be able to go green by
        /// being unconfigured. Without `--docs` no documentation page is opened and the
        /// include-target check compares nothing; without `--required-languages` the
        /// language-parity check never runs. Either way an unconfigured run reported
        /// "No gaps found." ~keep
        #[arg(long)]
        strict: bool,
    },
}

pub fn run(action: SnippetsAction) -> ExitCode {
    match action {
        SnippetsAction::List { snippets, languages } => run_list(&snippets, languages.as_ref()),
        SnippetsAction::Check {
            config,
            strict,
            cache,
            languages,
            level,
        } => run_check(&config, strict, cache != "off", languages.as_deref(), level.as_deref()),
        SnippetsAction::Parse { file } => run_parse(&file),
        SnippetsAction::Audit {
            snippets,
            docs,
            require_frontmatter,
            config,
        } => run_audit(&snippets, &docs, require_frontmatter, config.as_deref()),
        SnippetsAction::Gaps {
            snippets,
            docs,
            required_languages,
            include_base_paths,
            strict,
        } => run_gaps(&GapInvocation {
            snippet_dirs: &snippets,
            docs_dirs: &docs,
            required_languages: required_languages.as_deref(),
            include_base_paths: &include_base_paths,
            strict,
        }),
    }
}

/// A resolved `--lang` selection, keeping rejects so the caller can name them.
struct LanguageFilter {
    recognised: Vec<Language>,
    unrecognised: Vec<String>,
}

/// Resolve `--lang` values to snippet languages.
///
/// Accepts session target names (`kotlin_android`, `node`, `wasm`) as well as fence tags, because
/// the name a user reaches for is the one they just read in their `alef.toml`, and those two
/// vocabularies do not coincide. ~keep
fn parse_language_filter(languages: Option<&[String]>) -> Option<LanguageFilter> {
    let languages = languages?;
    let mut recognised: Vec<Language> = Vec::new();
    let mut unrecognised: Vec<String> = Vec::new();
    for requested in languages {
        match Language::from_session_target(requested) {
            Language::Unknown => unrecognised.push(requested.clone()),
            language => {
                if !recognised.contains(&language) {
                    recognised.push(language);
                }
            }
        }
    }
    Some(LanguageFilter {
        recognised,
        unrecognised,
    })
}

/// Report `--lang` values that named nothing, so a typo cannot silently widen or empty the run.
fn reject_unrecognised_languages(filter: Option<&LanguageFilter>) -> Result<(), ExitCode> {
    let Some(filter) = filter else { return Ok(()) };
    if filter.unrecognised.is_empty() {
        return Ok(());
    }
    tracing::error!(
        "unrecognised --lang value(s): {:?}. Use a snippet fence tag (`go`, `kotlin`, ...) or a \
         session target name from alef.toml (`kotlin_android`, `node`, ...)",
        filter.unrecognised
    );
    Err(ExitCode::FAILURE)
}

fn run_list(snippets: &[PathBuf], languages: Option<&Vec<String>>) -> ExitCode {
    let filter = parse_language_filter(languages.map(Vec::as_slice));
    if let Err(code) = reject_unrecognised_languages(filter.as_ref()) {
        return code;
    }
    let selected = filter.as_ref().map(|filter| filter.recognised.as_slice());
    match discovery::discover_snippets(snippets, selected) {
        Ok(found) => {
            output::print_snippet_list(&found);
            crate::bin_cli::output::blank();
            for (language, count) in &discovery::count_by_language(&found) {
                crate::bin_cli::output::line(format!("  {language:<12} {count}"));
            }
            crate::bin_cli::output::blank();
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("discovering snippets: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_check(
    config_path: &Path,
    force_strict: bool,
    use_cache: bool,
    languages: Option<&[String]>,
    level_override: Option<&str>,
) -> ExitCode {
    let (_, resolved) = match crate::bin_cli::helpers::load_config(config_path) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("loading snippet config: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some((crate_config, config)) = resolved
        .iter()
        .find_map(|krate| Some((krate, krate.docs.as_ref()?.snippets.as_ref()?)))
    else {
        tracing::error!("no [workspace.docs.snippets] or [crates.docs.snippets] configuration found");
        return ExitCode::FAILURE;
    };
    let root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let excluded_paths: Vec<PathBuf> = config.exclude.iter().map(|excluded| root.join(excluded)).collect();
    let snippet_directories = resolved_roots(root, &config.dirs, &excluded_paths);
    let mut directories = snippet_directories.clone();
    directories.extend(resolved_roots(root, &config.inline_dirs, &excluded_paths));
    let docs_directories: Vec<PathBuf> = config.docs_dirs.iter().map(|path| root.join(path)).collect();
    let include_base_paths: Vec<PathBuf> = if config.include_base_paths.is_empty() {
        docs_directories.clone()
    } else {
        config.include_base_paths.iter().map(|path| root.join(path)).collect()
    };
    let required_languages = match config
        .required_languages
        .iter()
        .map(|language| crate::snippets::types::resolve_required_language(language))
        .collect::<Result<Vec<Language>, String>>()
    {
        Ok(languages) => languages,
        Err(error) => {
            tracing::error!("invalid docs.snippets.required_languages entry: {error}");
            return ExitCode::FAILURE;
        }
    };
    let level = match resolve_check_level(level_override, config.validation_level.as_deref()) {
        Ok(level) => level,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let strict = force_strict || config.strict;
    // An unrecognised `--lang` must not silently widen the run back to everything: an
    // empty-but-`Some` filter reads to discovery as "match nothing", and the run would then exit
    // on "returned no snippets" naming the directories rather than the bad tag. ~keep
    let language_filter = parse_language_filter(languages);
    if let Err(code) = reject_unrecognised_languages(language_filter.as_ref()) {
        return code;
    }
    let selected = language_filter.as_ref().map(|filter| filter.recognised.as_slice());
    let found = match discovery::discover_snippets(&directories, selected) {
        Ok(found) if !found.is_empty() => found,
        Ok(_) => {
            match &language_filter {
                Some(filter) => tracing::error!("no snippets matched --lang {:?}", filter.recognised),
                None => tracing::error!("snippet discovery returned no snippets"),
            }
            return ExitCode::FAILURE;
        }
        Err(error) => {
            tracing::error!("discovering configured snippets: {error}");
            return ExitCode::FAILURE;
        }
    };
    let allowed_side_effects = config
        .allowed_side_effects
        .iter()
        .filter_map(|value| parse_side_effect(value))
        .collect();
    let runner = RunnerConfig {
        level,
        // Mirrors `RunnerConfig::default()`'s own choice (`snippets::runner::available_parallelism`,
        // private to that module) rather than `std::thread::available_parallelism()`: this reads
        // back whatever the top-level `--jobs`/`-j` flag already sized the process-wide rayon pool
        // to, so a user capping parallelism on a busy host is honoured here too, not just by the
        // build/generate/format/clean/update/setup stages that already read the same global pool
        // through a bare `par_iter()`. ~keep
        parallelism: rayon::current_num_threads(),
        timeout_secs: config.timeout_secs.unwrap_or(120),
        before_timeout_secs: config.before_timeout_secs,
        fail_fast: config.fail_fast,
        deny_unclassified: resolved_deny_unclassified(config.deny_unclassified, strict),
        allowed_side_effects,
        cache_dir: use_cache.then(|| root.join(config.cache_dir())),
        changed_only: use_cache,
        toolchain_cache_generations: crate::snippets::session::DEFAULT_TOOLCHAIN_CACHE_GENERATIONS,
        sessions: match configured_sessions(config, root, &crate_config.features) {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::error!("{error}");
                return ExitCode::FAILURE;
            }
        },
    };
    let summary = match run_validation(&found, &ValidatorRegistry::new(), &runner) {
        Ok(summary) => summary,
        Err(error) => {
            tracing::error!("running configured snippet validation: {error}");
            return ExitCode::FAILURE;
        }
    };
    output::print_summary(&summary, false);
    if let Some(path) = &config.report_output
        && let Err(error) = output::write_report(&summary, &root.join(path), false)
    {
        tracing::error!("writing snippet report: {error}");
        return ExitCode::FAILURE;
    }
    let strict_failure = strict && has_incomplete_coverage(&summary);
    let missing_generated = match missing_generated_snippets(&directories) {
        Ok(missing) => missing,
        Err(error) => {
            tracing::error!("reading generated snippet coverage: {error}");
            return ExitCode::FAILURE;
        }
    };
    for missing in &missing_generated {
        tracing::warn!(
            "generated snippet missing for fixture `{}` language `{}`: {}",
            missing.key.fixture_id,
            missing.key.language,
            missing.reason
        );
    }
    let content_collections: std::collections::BTreeMap<String, PathBuf> = config
        .content_collections
        .iter()
        .map(|(name, collection_root)| (name.clone(), root.join(collection_root)))
        .collect();
    let curated_paths = match configured_curated_paths(config_path) {
        Ok(curated) => curated,
        Err(error) => {
            tracing::error!("resolving curated snippet declaration: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let (audit_failure, gap_failure) = match run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &docs_directories,
        include_base_paths: &include_base_paths,
        configured_include_base_paths: &config.include_base_paths,
        required_languages: &required_languages,
        exclude: &excluded_paths,
        curated_paths: &curated_paths,
        readme: crate_config.readme.as_ref(),
        content_collections: &content_collections,
        workspace_root: root,
        require_frontmatter: config.require_frontmatter,
        strict,
    }) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!("running configured snippet audit and gap checks: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Unconditional, not gated on `strict` (task #488): a run where not one result reached its
    // requested level is never a legitimate mixed outcome, unlike an individual
    // `capability_capped`/`unavailable` result for one language among many, which can be. Without
    // this a corpus that is entirely `capability_capped` (every validator structurally capped
    // below the configured level) or entirely `unavailable` reports a clean `passed`/`unavailable`
    // split and exits success, even though the requested level was satisfied nowhere at all. See
    // `RunSummary::checked_nothing`'s doc comment for the full reasoning. ~keep
    if summary.has_failures()
        || summary.checked_nothing()
        || strict_failure
        || strict && !missing_generated.is_empty()
        || audit_failure
        || gap_failure
    {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Resolve `check`'s effective validation level: an explicit `--level` wins outright, otherwise
/// `[docs.snippets].validation_level` (defaulting to `syntax`).
///
/// The two sides are validated differently on purpose (task #542). `--level` is a one-off flag
/// whose entire purpose is requesting a specific level; a typo there must fail loudly rather than
/// silently downgrade to `syntax`, which would defeat the flag. The config value keeps its
/// existing lax fallback (an unrecognised string silently becomes `syntax`) for backward
/// compatibility -- changing that now would fail configs that previously loaded fine. ~keep
fn resolve_check_level(level_override: Option<&str>, configured: Option<&str>) -> Result<ValidationLevel, String> {
    match level_override {
        Some(raw) => raw
            .parse::<ValidationLevel>()
            .map_err(|error| format!("invalid --level value `{raw}`: {error}")),
        None => Ok(configured
            .unwrap_or("syntax")
            .parse::<ValidationLevel>()
            .unwrap_or(ValidationLevel::Syntax)),
    }
}

/// Whether the snippet runner should reject snippets with no side-effect classification.
///
/// `strict` here is already the union of `--strict` and `[crates.docs.snippets].strict` (see
/// `run_check`'s `strict` binding) -- every other strict-gated behavior in this command (coverage
/// completeness, missing generated snippets, gap findings) already reads that combined value.
/// This used to read the raw `--strict` flag alone, so `[crates.docs.snippets].strict = true`
/// left unclassified snippets free to reach real execution at `ValidationLevel::Run` instead of
/// being pre-emptively skipped -- a config-only `strict` consumer had no way to get the same
/// safety gate a flag-only `strict` consumer got for free. `config.deny_unclassified` remains a
/// standalone opt-in on top, for a consumer who wants the gate without going fully strict. ~keep
fn resolved_deny_unclassified(config_deny_unclassified: bool, strict: bool) -> bool {
    config_deny_unclassified || strict
}

/// Resolve configured snippet roots against `root`, dropping any that fall
/// under an excluded prefix.
fn resolved_roots(root: &Path, dirs: &[PathBuf], excluded: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter()
        .map(|path| root.join(path))
        .filter(|path| !excluded.iter().any(|prefix| path.starts_with(prefix)))
        .collect()
}

/// Inputs for `check`'s configured audit and gap pass, grouped into one
/// struct so the call stays under clippy's argument threshold.
struct ConfiguredCheckInputs<'a> {
    /// Snippet roots proper — `docs.snippets.dirs` only. `inline_dirs` are
    /// deliberately absent: they are prose documentation pages whose fenced
    /// blocks are validated as snippets, and they are never `--8<--` include
    /// targets, so gap-checking them would report every documentation page as
    /// an unreferenced snippet. Mirrors `docs/mod.rs::validate_snippets`,
    /// which likewise audits and gap-checks only the `dirs`-derived list.
    snippet_directories: &'a [PathBuf],
    docs_directories: &'a [PathBuf],
    include_base_paths: &'a [PathBuf],
    /// The raw `[crates.docs.snippets].include_base_paths` value, before `run_check` substitutes
    /// the docs roots for an empty list. Kept alongside the resolved list because unset-ness is
    /// unobservable after that fallback. ~keep
    configured_include_base_paths: &'a [PathBuf],
    required_languages: &'a [Language],
    exclude: &'a [PathBuf],
    /// Already-resolved `[crates.e2e.snippets].curated_snippets` paths, spelled the way the
    /// snippet-root walk produces them.
    curated_paths: &'a [PathBuf],
    readme: Option<&'a crate::core::config::ReadmeConfig>,
    /// Astro content collection names mapped to their already-resolved roots.
    content_collections: &'a std::collections::BTreeMap<String, PathBuf>,
    workspace_root: &'a Path,
    require_frontmatter: bool,
    strict: bool,
}

/// Run the configured audit and gap checks against the configured snippet
/// roots, so `check` cannot disagree with `alef validate` about which files
/// are in scope.
///
/// References a snippet can legitimately have without any `--8<--` include
/// are collected from the same three sources as
/// `docs/mod.rs::validate_snippets`: `[crates.readme]` snippet mappings,
/// generated-snippet coverage ledgers, and Astro content collections queried
/// by a documentation page.
///
/// Audit issues of `AuditSeverity::Error` always fail the gate, and audit is
/// skipped entirely without a configured docs surface (matching the
/// precedent) so a snippets-only config is not failed by fence tags no
/// documentation ever renders. Gap findings split the same way `alef
/// validate`'s snippet gate already treats them: unreferenced snippets are
/// only a failure under `strict` (extra examples can be intentional), while
/// missing include targets, missing required language variants, undocumented
/// skips, and unknown fence languages always fail. Gaps are skipped entirely
/// when neither `docs_dirs` nor `required_languages` is configured —
/// otherwise every discovered snippet would read as "unreferenced" and a
/// `strict` config with no docs surface configured would flip from green to
/// red for a check that was never meaningful for it.
///
/// Coverage ledgers are read with missing fixture/language cells tolerated:
/// `run_check` already reports those through `missing_generated_snippets` and
/// only fails on them under `strict`, so rejecting them here would both
/// override that gate and misattribute the failure.
///
/// # Errors
///
/// Returns an error when a coverage ledger is broken, an Astro collection
/// root cannot be walked, or a documentation file cannot be read.
fn run_configured_audit_and_gaps(inputs: &ConfiguredCheckInputs<'_>) -> anyhow::Result<(bool, bool)> {
    let mut configured_references =
        crate::snippets::gaps::readme_snippet_references(inputs.workspace_root, inputs.readme);
    // Kept apart from the wider reference list: accounting asks specifically whether ALEF
    // generated a file, which a README mapping or an Astro collection query never answers. ~keep
    let generated_paths =
        crate::snippets::gaps::coverage_ledger_references_allowing_missing_cells(inputs.snippet_directories)?;
    configured_references.extend(generated_paths.iter().cloned());
    configured_references.extend(crate::snippets::gaps::astro_collection_references(
        inputs.docs_directories,
        inputs.content_collections,
    )?);

    let audit_failure = if inputs.docs_directories.is_empty() {
        false
    } else {
        report_audit(&audit(&AuditConfig {
            docs_dirs: inputs.docs_directories.to_vec(),
            snippet_dirs: inputs.snippet_directories.to_vec(),
            require_frontmatter: inputs.require_frontmatter,
            include_base_paths: inputs.include_base_paths.to_vec(),
            configured_references: configured_references.clone(),
            exclude: inputs.exclude.to_vec(),
            accounting: crate::snippets::audit::SnippetAccounting {
                generated_paths: generated_paths.clone(),
                curated_paths: inputs.curated_paths.to_vec(),
                enabled: !generated_paths.is_empty(),
            },
        }))
    };

    // Naming the unset keys is the whole point: the skip below is deliberate, but it used to be
    // silent, so a consumer whose `alef.toml` omitted `docs_dirs` and `required_languages` read a
    // green `snippets check` for a gap pass that never ran. ~keep
    if inputs.docs_directories.is_empty() && inputs.required_languages.is_empty() {
        // No documentation page could have been opened, so no `--8<--` target could exist either
        // -- `mkdocs_include_references` is unconditionally 0 here, matching what a gap pass
        // would have measured had it run. ~keep
        let unset = crate::snippets::gap_coverage::unset_gap_inputs(
            inputs.docs_directories,
            inputs.required_languages,
            inputs.configured_include_base_paths,
            0,
        );
        for line in crate::snippets::gap_coverage::unset_input_lines(&unset, inputs.strict) {
            tracing::warn!("{line}");
        }
        let unconfigured_failure = inputs.strict && crate::snippets::gap_coverage::has_vacuous_input(&unset);
        if unconfigured_failure {
            // A strict run must not pass on a check that compared nothing -- the whole point of
            // `--strict` is that a green result certifies a comparison actually happened. ~keep
            tracing::error!(
                "strict: the snippet gap pass was skipped -- neither docs_dirs nor required_languages \
                 is configured under [crates.docs.snippets]"
            );
        }
        return Ok((audit_failure, unconfigured_failure));
    }
    let gap_report = detect_gaps(&GapConfig {
        docs_dirs: inputs.docs_directories.to_vec(),
        snippet_dirs: inputs.snippet_directories.to_vec(),
        required_languages: inputs.required_languages.to_vec(),
        include_base_paths: inputs.include_base_paths.to_vec(),
        configured_references,
        exclude: inputs.exclude.to_vec(),
    })?;
    // `include_base_paths` is only reported unset when the run actually found a `--8<--` target
    // that would have used it -- an Astro/MDX-only docs tree never does, and warning about a
    // base-path list it has no use for would be an unsilenceable, unactionable warning on
    // `check` (there is no `--include-base-path` flag on this command; the config key is the
    // only surface, and setting it configures nothing real when there is no `--8<--` syntax in
    // use at all). ~keep
    let unset = crate::snippets::gap_coverage::unset_gap_inputs(
        inputs.docs_directories,
        inputs.required_languages,
        inputs.configured_include_base_paths,
        gap_report.coverage.mkdocs_include_references,
    );
    for line in crate::snippets::gap_coverage::unset_input_lines(&unset, inputs.strict) {
        tracing::warn!("{line}");
    }
    log_gaps(&gap_report);
    Ok((audit_failure, gap_report.is_failure(inputs.strict)))
}

fn report_audit(report: &crate::snippets::audit::AuditReport) -> bool {
    for issue in &report.issues {
        let message = format!(
            "snippet audit: {}:{} ({:?}) {}",
            issue.path.display(),
            issue.line,
            issue.kind,
            issue.message
        );
        match issue.severity {
            AuditSeverity::Error => tracing::error!("{message}"),
            AuditSeverity::Warning => tracing::warn!("{message}"),
        }
    }
    report.has_errors()
}

/// Logs every gap finding via `tracing`.
///
/// The pass/fail verdict is [`crate::snippets::gaps::GapReport::is_failure`] — this function is
/// output only, so `check` and `gaps` (which prints the same findings through
/// `crate::bin_cli::output::line` instead) cannot drift on which findings actually fail a run.
pub(super) fn log_gaps(report: &crate::snippets::gaps::GapReport) {
    for reference in &report.missing_references {
        tracing::error!(
            "snippet gap: missing include target {}:{} -> {}",
            reference.source.display(),
            reference.line,
            reference.target.display()
        );
    }
    for path in &report.unreferenced_snippets {
        tracing::warn!("snippet gap: unreferenced snippet {}", path.display());
    }
    for variant in &report.missing_language_variants {
        tracing::error!(
            "snippet gap: missing required language variant `{}` for {}",
            variant.language,
            variant.group.display()
        );
    }
    for location in &report.skips_without_reason {
        tracing::error!(
            "snippet gap: skip without reason {}:{} (block {})",
            location.path.display(),
            location.line,
            location.block_index
        );
    }
    for unknown in &report.unknown_languages {
        tracing::error!(
            "snippet gap: unknown fence language {}:{} tag=`{}`",
            unknown.path.display(),
            unknown.line,
            unknown.tag
        );
    }
}

fn configured_sessions(
    config: &crate::core::config::DocsSnippetsConfig,
    root: &std::path::Path,
    crate_features: &[String],
) -> Result<std::collections::HashMap<String, SessionSpec>, String> {
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("resolving current directory for snippet sessions: {error}"))?
            .join(root)
    };
    let mut sessions = std::collections::HashMap::new();
    for (target, session) in &config.sessions {
        let normalized = Language::normalize_session_target(target);
        let language = Language::from_session_target(&normalized);
        if language == Language::Unknown {
            return Err(format!("unknown docs.snippets session target `{target}`"));
        }
        let mut rust_features = session.rust_features.clone();
        if language == Language::Rust {
            rust_features.extend(crate_features.iter().cloned());
            rust_features.sort();
            rust_features.dedup();
        }
        let spec = SessionSpec {
            language,
            working_directory: root.join(&session.cwd),
            manifest: session.manifest.as_ref().map(|path| root.join(path)),
            before: session.before.clone(),
            env: session.env.clone(),
            include_paths: session.include_paths.iter().map(|path| root.join(path)).collect(),
            rust_features,
            rust_dependencies: session.rust_dependencies.clone(),
        };
        if sessions.insert(normalized.clone(), spec).is_some() {
            return Err(format!("duplicate docs.snippets session target `{normalized}`"));
        }
    }
    Ok(sessions)
}

fn missing_generated_snippets(directories: &[PathBuf]) -> anyhow::Result<Vec<crate::e2e::snippets::MissingSnippet>> {
    let mut missing = Vec::new();
    for directory in directories {
        let path = directory.join(crate::e2e::snippets::COVERAGE_MANIFEST);
        if !path.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        let ledger: crate::e2e::snippets::SnippetCoverageLedger = serde_json::from_str(&content)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))?;
        missing.extend(ledger.missing);
    }
    missing.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(missing)
}

fn has_incomplete_coverage(summary: &crate::snippets::types::RunSummary) -> bool {
    summary.results.iter().any(|result| is_incomplete_status(result.status))
}

fn is_incomplete_status(status: SnippetStatus) -> bool {
    matches!(
        status,
        SnippetStatus::Skip | SnippetStatus::Unavailable | SnippetStatus::Downgraded
    )
}

fn parse_side_effect(value: &str) -> Option<SideEffectClass> {
    match value.trim().to_ascii_lowercase().as_str() {
        "safe" => Some(SideEffectClass::Safe),
        "network" => Some(SideEffectClass::Network),
        "process" => Some(SideEffectClass::Process),
        "install" => Some(SideEffectClass::Install),
        "server" => Some(SideEffectClass::Server),
        _ => None,
    }
}

fn run_parse(file: &Path) -> ExitCode {
    match crate::snippets::parser::parse_code_blocks(file) {
        Ok(blocks) => {
            if blocks.is_empty() {
                crate::bin_cli::output::line(format!("No code blocks found in {}", file.display()));
            } else {
                for (index, block) in blocks.iter().enumerate() {
                    crate::bin_cli::output::line(format!("--- Block {} (line {}) ---", index + 1, block.start_line));
                    crate::bin_cli::output::line(format!("Language: {}", block.lang));
                    if let Some(title) = &block.title {
                        crate::bin_cli::output::line(format!("Title: {title}"));
                    }
                    if let Some(comment) = &block.preceding_comment {
                        crate::bin_cli::output::line(format!("Annotation: {comment}"));
                    }
                    crate::bin_cli::output::line(format!("Code ({} lines):", block.code.lines().count()));
                    crate::bin_cli::output::line(&block.code);
                    crate::bin_cli::output::blank();
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("parsing {}: {err}", file.display());
            ExitCode::FAILURE
        }
    }
}

/// Reject configured roots that are not on disk before either report is computed.
///
/// Both commands already failed on a missing `--snippets` root, but only by accident and with
/// the wrong cause: the first thing they do is walk it for coverage ledgers, so the user was
/// told "reading generated snippet coverage: ... IO error" for what is simply a path that does
/// not exist. A missing `--docs` root was worse -- nothing walks it eagerly, so `audit` reported
/// "Audit clean" over a documentation tree it never opened. See
/// `discovery::missing_configured_directories` for the policy both share. ~keep
pub(super) fn reject_missing_configured_directories(
    snippet_dirs: &[PathBuf],
    docs_dirs: &[PathBuf],
) -> Result<(), ExitCode> {
    let checked = [
        (discovery::SNIPPET_DIRECTORY_KIND, snippet_dirs),
        (discovery::DOCUMENTATION_DIRECTORY_KIND, docs_dirs),
    ];
    for (kind, dirs) in checked {
        if let Err(error) = discovery::ensure_configured_directories_exist(kind, dirs) {
            tracing::error!("{error}");
            return Err(ExitCode::FAILURE);
        }
    }
    Ok(())
}

/// Name the audit's coverage so a clean result cannot be mistaken for a wider one.
///
/// ~keep An invocation with no `--docs` root skips the documentation-page checks
/// (fence languages, include targets) entirely and still printed a bare
/// "Audit clean: no issues found." A consumer whose CI omitted `--docs` therefore read a
/// green for a check class that never ran. This is the same defect
/// `reject_missing_configured_directories` fixed for a *missing* root; an *absent* root
/// needs saying too, but must not fail the run — auditing snippets alone is legitimate.
fn audit_scope_summary(docs_dirs: &[PathBuf]) -> &'static str {
    if docs_dirs.is_empty() {
        "Audit clean: no issues found in the snippet roots. \
         Documentation pages were NOT audited — pass --docs to check fence languages and include targets."
    } else {
        "Audit clean: no issues found."
    }
}

fn run_audit(
    snippet_dirs: &[PathBuf],
    docs_dirs: &[PathBuf],
    require_frontmatter: bool,
    config_path: Option<&Path>,
) -> ExitCode {
    if let Err(code) = reject_missing_configured_directories(snippet_dirs, docs_dirs) {
        return code;
    }
    let AuditOutcome {
        report,
        accounting_enabled,
    } = match audit_outcome(snippet_dirs, docs_dirs, require_frontmatter, config_path) {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!("auditing snippets: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    crate::bin_cli::output::line(accounting_scope_line(
        config_path,
        accounting_enabled,
        report.curated.len(),
    ));
    for path in &report.curated {
        crate::bin_cli::output::line(format!("  curated {}", path.display()));
    }
    if report.issues.is_empty() {
        crate::bin_cli::output::line(audit_scope_summary(docs_dirs));
        return ExitCode::SUCCESS;
    }
    crate::bin_cli::output::line(format!("Audit found {} issue(s):", report.issues.len()));
    for issue in &report.issues {
        let severity = match issue.severity {
            AuditSeverity::Error => "ERROR",
            AuditSeverity::Warning => "WARN",
        };
        crate::bin_cli::output::line(format!(
            "  [{severity}] {}:{} ({:?}) {}",
            issue.path.display(),
            issue.line,
            issue.kind,
            issue.message
        ));
    }
    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

mod accounting;
mod gaps_command;

use gaps_command::{GapInvocation, run_gaps};

#[cfg(test)]
mod tests;
