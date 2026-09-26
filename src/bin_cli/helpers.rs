use anyhow::{Context, Result};

pub(crate) mod frozen;
pub(crate) use frozen::FrozenFile;
use frozen::frozen_managed_paths;

mod post_build;
use post_build::run_required_post_builds;
pub(crate) use post_build::{languages_have_post_build_steps, languages_with_post_build_steps};

/// Complete every generated artifact that depends on a backend build and then
/// enforce FFI source/header parity. Keeping these operations together prevents
/// commands from validating a cbindgen header before its producer runs or from
/// omitting the final parity gate. ~keep
///
/// `compile` is the whole of the generation-only mode (`alef all --skip-compile`,
/// `alef generate --skip-compile`): this function holds both of the generation path's compiling
/// steps, so gating them here -- rather than at each producer -- is what makes "generation writes
/// source, `alef build` compiles" a property of one function instead of a convention. Under
/// [`crate::core::backend::CompilePolicy::Skipped`] the FFI header is still *checked*, only never rebuilt: a stale
/// header still fails the run (it describes an ABI the source no longer has, which no flag makes
/// safe), while a missing one degrades to the warning `check_ffi_header_freshness` already emits
/// for that case. ~keep
///
/// The FFI header refresh runs BEFORE `run_required_post_builds`, not after -- even though the
/// refresh only does anything on the one path where this ordering matters. A stale header makes
/// `ensure_ffi_header_freshness` run `cargo build` for the `-ffi` crate (with
/// `ALEF_EXPORT_GENERATED_HEADERS=1`), and that build also drops a fresh cdylib in
/// `target/debug/`. `PostBuildStep::StageFfiLibrary` (Go/Java/C#'s post-build step) copies
/// whichever cdylib is already on disk into each binding's native-library directory, so staging
/// before the refresh copies whatever was there BEFORE this run's own rebuild -- the artifact
/// this run just produced would sit unstaged until some LATER run happened to find the header
/// already fresh and never rebuild it again. Refreshing first closes that gap.
///
/// This reordering can never make a release build stage a debug artifact: this whole function is
/// reachable only from `alef generate`/`alef all` (never from `alef build`), the refresh above
/// always builds debug, and `StageFfiLibrary` runs here under
/// [`crate::cli::pipeline::StagingProfile::NoBuildRequested`] either way --
/// `stage_ffi_preferring_release` always prefers a release artifact already on disk over any
/// debug artifact, so a real release build a prior `alef build --release` left behind is never
/// displaced by this run's debug-only refresh, in either order.
///
/// Header-refresh failure must not skip every other language's post-build the way an early
/// `return` used to: `run_required_post_builds` isolates one language's post-build failure from
/// every other's, and gating the whole post-build pass on an unrelated header check would undo
/// that. The two results are therefore captured independently, via [`refresh_ffi_header`] and
/// [`run_required_post_builds`], and combined by [`combine_artifact_results`] rather than
/// short-circuited with `?`. ~keep
pub(crate) fn complete_generated_artifacts(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
    compile: crate::core::backend::CompilePolicy,
) -> Result<()> {
    let header_result = refresh_ffi_header(languages, config, base_dir, compile);
    let post_build_result = run_required_post_builds(languages, config, base_dir, compile);
    combine_artifact_results(header_result, post_build_result)
}

/// Check, and unless this is a generation-only run refresh, the FFI header's freshness --
/// `Ok(())` with no work done when `languages` has no `ffi` entry at all.
///
/// Split out of [`complete_generated_artifacts`] so its result can be captured independently of
/// `run_required_post_builds`'s -- see that function's doc for why the two must not gate each
/// other. ~keep
fn refresh_ffi_header(
    languages: &[crate::core::config::Language],
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
    compile: crate::core::backend::CompilePolicy,
) -> Result<()> {
    if !languages.contains(&crate::core::config::Language::Ffi) {
        return Ok(());
    }

    if compile == crate::core::backend::CompilePolicy::Skipped {
        tracing::warn!(
            "[ffi] --skip-compile: not building the FFI crate to refresh its cbindgen header; \
             checking the header already on disk instead"
        );
        return crate::cli::pipeline::check_ffi_header_freshness(config, base_dir);
    }

    crate::cli::pipeline::ensure_ffi_header_freshness(config, base_dir, || {
        crate::cli::pipeline::build_with_environment(
            config,
            &[crate::core::config::Language::Ffi],
            false,
            &[("ALEF_EXPORT_GENERATED_HEADERS", "1")],
            false,
        )
    })
}

/// Combine the FFI-header and post-build outcomes without letting either mask the other.
///
/// `complete_generated_artifacts` only returns one `Result`, so a run where the header refresh
/// failed AND some language's post-build also failed must still surface both -- discarding the
/// post-build side (as a plain `header_result.and(post_build_result)` would, since `and` drops
/// an `Err` on the left as soon as the right is also `Err`) would silently hide a genuine
/// post-build failure the moment the header also happened to be stale. ~keep
fn combine_artifact_results(header_result: Result<()>, post_build_result: Result<()>) -> Result<()> {
    match (header_result, post_build_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(post_build_error)) => Err(post_build_error),
        (Err(header_error), Ok(())) => Err(header_error),
        (Err(header_error), Err(post_build_error)) => {
            Err(header_error.context(format!("post-build also failed: {post_build_error:#}")))
        }
    }
}

/// Returns true when every freshly generated file already matches the file on disk,
/// using the same hash-line-insensitive body comparison as [`crate::cli::pipeline::write_files`].
///
/// The per-run side cache (`.alef/hashes/*.output_hashes`) records what was last
/// generated, but the files on disk can drift from it out-of-band — a `git restore`,
/// a hand-edit, a partial write, or an interrupted run. Treating the cache as the
/// sole authority for an "up to date" skip silently retains that stale output: the
/// generator would emit different bytes, yet the skip fires and `write_files` is
/// never reached. Gating the skip on actual disk agreement closes that gap while
/// staying a no-op for the common clean case.
pub(crate) fn generated_files_match_disk(
    lang_files: &[crate::core::backend::GeneratedFile],
    base_dir: &std::path::Path,
) -> bool {
    lang_files.iter().all(|file| {
        let normalized = crate::cli::pipeline::normalize_content(&file.path, &file.content);
        match std::fs::read_to_string(base_dir.join(&file.path)) {
            Ok(disk) => crate::core::hash::strip_hash_line(&disk) == crate::core::hash::strip_hash_line(&normalized),
            Err(_) => false,
        }
    })
}

/// Map the CLI verbosity flags to a default `tracing` level filter.
///
/// A single verbosity channel drives the whole binary: `--quiet` pins the level to
/// `error`; otherwise the `-v` count raises it — no flag is `info` (progress visible),
/// `-v` is `debug` (per-file/per-item detail), `-vv`+ is `trace`. `RUST_LOG` overrides
/// this default when set (see [`init_tracing`]).
pub(crate) fn default_log_level(verbose: u8, quiet: bool) -> &'static str {
    if quiet {
        return "error";
    }
    match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    }
}

pub(crate) fn init_tracing(verbose: u8, quiet: bool, no_color: bool) {
    use tracing_subscriber::EnvFilter;
    let default_level = default_log_level(verbose, quiet);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(!no_color)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();
}

/// Load and resolve an alef.toml, returning the workspace-level config and
/// the per-crate resolved configs.  Detects legacy schema and returns an error
/// with a migration hint rather than a confusing parse error.
pub(crate) fn load_config(
    path: &std::path::Path,
) -> Result<(
    crate::core::config::WorkspaceConfig,
    Vec<crate::core::config::ResolvedCrateConfig>,
)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    crate::core::config::detect_legacy_keys(&content).with_context(|| {
        format!(
            "legacy schema detected in {} — run `alef migrate` to update automatically",
            path.display()
        )
    })?;
    let mut toml_value: toml::Value =
        toml::from_str(&content).with_context(|| format!("Failed to parse alef.toml ({})", path.display()))?;
    let deprecation_warnings = crate::core::config::legacy::strip_deprecated_keys(&mut toml_value);
    for warning in &deprecation_warnings {
        tracing::warn!("{}", warning);
    }
    let cfg: crate::core::config::NewAlefConfig = toml_value
        .try_into()
        .with_context(|| format!("Failed to deserialize alef.toml ({})", path.display()))?;
    let resolved = cfg
        .resolve()
        .with_context(|| format!("failed to resolve crates in {}", path.display()))?;
    for resolved_cfg in &resolved {
        crate::core::config::validation::validate_resolved(resolved_cfg)
            .with_context(|| format!("invalid resolved config for crate `{}`", resolved_cfg.name))?;
    }
    Ok((cfg.workspace, resolved))
}

pub(crate) fn resolve_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
pub(crate) fn resolve_doc_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
///
/// Every Rust crate that publishes to crates.io needs a `crates/<lib>/README.md`,
/// so the readme command must regenerate it from the same templates that produce
/// the per-binding READMEs. Configure with `[crates.readme.languages.rust]` in
/// `alef.toml` to opt in.
pub(crate) fn resolve_readme_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<crate::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Resolve languages for `alef test`.
///
/// Test suites can exist for targets that do not generate host bindings, such
/// as Rust e2e tests for the source crate. Keep binding language resolution
/// strict for generation/build commands, but allow explicit test targets and
/// include e2e-only entries when `alef test --e2e` runs without a filter.
pub(crate) fn resolve_test_languages(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    include_e2e: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || config.test.contains_key(&lang.to_string()) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list or test configuration");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if include_e2e {
                let mut extra_test_langs = vec![];
                for (lang_str, test_config) in &config.test {
                    if test_config.e2e.is_none() {
                        continue;
                    }
                    let lang = parse_language(lang_str)
                        .with_context(|| format!("Invalid test language in alef.toml: {lang_str}"))?;
                    if !langs.contains(&lang) {
                        extra_test_langs.push(lang);
                    }
                }
                extra_test_langs.sort_by_key(|lang| lang.to_string());
                for lang in extra_test_langs {
                    if !langs.contains(&lang) {
                        langs.push(lang);
                    }
                }
            }
            Ok(langs)
        }
    }
}

pub(crate) fn resolve_languages_inner(
    config: &crate::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    allow_rust: bool,
) -> Result<Vec<crate::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || (allow_rust && lang == crate::core::config::Language::Rust) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if allow_rust && !langs.contains(&crate::core::config::Language::Rust) {
                langs.push(crate::core::config::Language::Rust);
            }
            Ok(langs)
        }
    }
}

pub(crate) fn parse_language(lang_str: &str) -> Result<crate::core::config::Language> {
    toml::Value::String(lang_str.to_string())
        .try_into()
        .with_context(|| format!("Unknown language: {lang_str}"))
}

pub(crate) fn format_languages(languages: &[crate::core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}

/// A file whose embedded `alef:hash:` did not match its own on-disk content hash.
///
/// Returned by [`verify_walk`] for every stale file found during a verify walk. The
/// `computed` field is the content-only hash [`stale_among`] recomputed from the file's
/// current on-disk bytes; `embedded` is what was found in the file's header. Since
/// [`crate::core::hash::compute_file_hash`] takes no generation-inputs parameter (see its
/// doc), there is exactly one candidate value per file, not one per crate -- pass
/// `--verbose` / `-v` to `alef verify` to print these.
pub(crate) struct StaleMismatch {
    /// Absolute path of the stale generated file.
    pub(crate) path: String,
    /// The `alef:hash:` value embedded in the file's header.
    pub(crate) embedded: String,
    /// The content-only hash recomputed from the file's current on-disk bytes. The file is
    /// stale because this differs from `embedded`.
    pub(crate) computed: String,
}

/// Re-exported so `helpers`' own callers and tests keep one spelling for the ownership walk
/// after it moved to [`super::verify_scan`]. ~keep
pub(crate) use super::verify_scan::collect_alef_hashes;

/// A cross-artifact ABI-generation disagreement: two or more alef-owned files
/// in the tree carry different values for the same `alef:<key>:` stamp (see
/// [`crate::core::hash::inject_stamp_line`]/[`extract_stamp`]).
///
/// A single `alef generate` run stamps every file it touches with the same
/// value, so two different values coexisting can only mean files from two
/// different generation runs are mixed in the tree — e.g. an FFI header
/// regenerated after a handle-representation change sitting next to a
/// binding backend's opaque-handle file that was not. `alef verify`'s
/// per-file `alef:hash:` staleness check cannot see this: each file's hash is
/// compared against the *current* generation inputs in isolation, never
/// against what a *different* file on disk was stamped with. ~keep
///
/// [`extract_stamp`]: crate::core::hash::extract_stamp
pub(crate) struct StampDisagreement {
    pub(crate) key: String,
    /// One `(display_path, value)` pair per distinct value found, so the
    /// report can show a representative example from each side of the split.
    pub(crate) examples: Vec<(String, String)>,
}

/// Find whether every alef-owned file under `base_dir` that carries an
/// `alef:<key>:` stamp agrees on its value.
///
/// Returns `None` when zero or one distinct value is present. Both are
/// silently fine, deliberately: zero stamped files means no backend in this
/// tree emits `key` yet — every consumer repo today, since nothing calls
/// `inject_stamp_line` — and that tree cannot be verified this way, not that
/// it disagrees; one distinct value is the healthy up-to-date case. Only 2+
/// distinct values is a provable disagreement (see [`StampDisagreement`]).
/// This intentionally does not attempt to flag "some files stamped, others
/// plausibly should be but aren't" as a softer warning: that requires knowing
/// which unstamped files are ABI-relevant, which a generic content walk
/// cannot determine — only the backend that emits a given file knows whether
/// it encodes the handle representation. ~keep
pub(crate) fn find_stamp_disagreement(base_dir: &std::path::Path, key: &str) -> Option<StampDisagreement> {
    let mut by_value: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (path, _hash, content) in collect_alef_hashes(base_dir) {
        let Some(value) = crate::core::hash::extract_stamp(&content, key) else {
            continue;
        };
        by_value.entry(value).or_insert_with(|| path.display().to_string());
    }
    if by_value.len() < 2 {
        return None;
    }
    Some(StampDisagreement {
        key: key.to_string(),
        examples: by_value.into_iter().map(|(value, path)| (path, value)).collect(),
    })
}

/// Display paths (joined onto `base_dir`, matching `StaleMismatch::path`'s
/// convention) of every alef-owned file in `files` that is entirely absent
/// from disk.
///
/// Ownership is decided by [`crate::core::backend::GeneratedFile::carries_alef_marker`]
/// via [`crate::cli::pipeline::managed_generated_files`] — the same predicate
/// [`collect_alef_hashes`] uses to select existing files for the hash walk.
/// A file without the marker (scaffold-once `Cargo.toml`/`package.json`/gemspec
/// templates, lockfiles) is outside alef's freshness claim and is never
/// reported missing here, even when absent — matching [`verify_walk`]'s scope
/// so a clean repo whose scaffold hasn't been (re-)run never fails verify. ~keep
fn missing_managed_paths(files: &[crate::core::backend::GeneratedFile], base_dir: &std::path::Path) -> Vec<String> {
    crate::cli::pipeline::managed_generated_files(files)
        .into_iter()
        .filter(|file| !base_dir.join(&file.path).exists())
        .map(|file| base_dir.join(&file.path).display().to_string())
        .collect()
}

/// The line within `content` that [`crate::core::hash::content_has_alef_marker`]
/// would recognize as the provenance marker, if any.
///
/// Delegates the actual match to `content_has_alef_marker` itself, applied one
/// line at a time, instead of re-implementing its marker text here — so the
/// two can never drift apart. ~keep
fn marker_line(content: &str) -> Option<&str> {
    content
        .lines()
        .find(|line| crate::core::hash::content_has_alef_marker(line))
}

/// Missing and frozen generated files found for one crate — see
/// [`find_missing_and_frozen_generated_files`].
#[derive(Default)]
pub(crate) struct MissingAndFrozenFiles {
    pub(crate) missing: Vec<String>,
    /// The subset of what would otherwise be in `missing` that git also reports ignored --
    /// split out by [`super::verify_gitignore::split_missing_by_gitignore`] because the remedy
    /// differs: `alef generate` writes these paths and the ignore rule discards them again
    /// before commit, so it can never close this gap the way it closes a plain `missing`
    /// entry. See that function's doc. ~keep
    pub(crate) missing_gitignored: Vec<String>,
    pub(crate) frozen: Vec<FrozenFile>,
    /// Absolute paths of alef-marked files that already exist on disk but whose bytes no longer
    /// match what this run's fresh render (`surface`) would produce — see [`drifted_marked_paths`]. ~keep
    pub(crate) drifted: Vec<String>,
    /// Absolute paths of every file this crate's configuration would produce this run, from the
    /// same `surface` `missing`/`frozen` are derived from -- deliberately every path, not only
    /// [`crate::cli::pipeline::managed_output_paths`]'s marker-carrying subset: a self-marking
    /// backend (pyo3's `lib.rs`, `docs::render`'s HTML-commented pages) bakes its header into
    /// `content` at write time via `ensure_generated_header`, not at this in-memory render, so
    /// `carries_alef_marker()` reads `false` here for a file that is unquestionably still
    /// produced this run -- filtering by it would misreport that file as an orphan the moment it
    /// left the marker-carrying subset. A multi-crate caller unions this across every crate
    /// before handing it to [`super::verify_orphans::find_orphaned_generated_files`], so a file
    /// legitimately owned by crate B is never mistaken for an orphan while diffing crate A alone. ~keep
    pub(crate) managed_paths: std::collections::HashSet<std::path::PathBuf>,
    /// [`StageFailure`]s [`collect_managed_surface`] tolerated while still building the
    /// rest of the surface, rendered as `[<stage>] <message>`. `alef verify` is
    /// read-only, so it has no target to decide "does this affect me" the way `alef
    /// adopt` does -- every tolerated failure is real debt and belongs in the report,
    /// never silently absorbed into a clean-looking zero. The `missing`/`frozen` lists
    /// above are still accurate despite these: the failing stage's own files are
    /// absorbed into the surface regardless of its error (see
    /// [`collect_managed_surface`]'s doc), so this is additional signal, not a
    /// disclaimer that the rest of the report cannot be trusted. ~keep
    pub(crate) stage_failures: Vec<String>,
}

/// Find both generated files that generation would now produce for `config`
/// but that do not exist on disk at all ([`missing_managed_paths`]), and
/// generated files that do exist but were never marked
/// ([`frozen_managed_paths`]).
///
/// `verify_walk`/`verify_walk_multi` only ever see files that already exist and
/// carry an `alef:hash:` header, so a file generation would emit but that was
/// never written — a new API item a backend maps to a brand-new per-type file
/// (`src/backends/java/gen_bindings/mod.rs`, `src/backends/csharp/gen_bindings/mod.rs`
/// both emit one file per public type), forgotten after adding the item — is
/// invisible to a pure disk walk. Same for a frozen file: it fails ownership,
/// not freshness, so a walk that only opens marker-bearing files by
/// construction cannot see it either (see [`collect_alef_hashes`]'s doc). Both
/// gaps require knowing what generation would produce, which is IR-derived for
/// some backends and cannot be answered from `alef.toml` alone; this mirrors
/// `alef diff`'s approach ([`crate::cli::pipeline::diff_files`],
/// `src/cli/pipeline/generate/diff.rs`) and pays a comparable cost: every stage
/// in [`collect_managed_surface`] is regenerated in memory (never written to
/// disk) for every configured language — paid once here for both checks
/// together, since paying it twice (one full regeneration pass per check) would
/// double the cost of every `alef verify` run for no benefit. ~keep
///
/// `clean = false` is what [`collect_managed_surface`] passes to
/// `crate::cli::pipeline::generate` deliberately,
/// not to save cost in CI (a fresh checkout's `.alef/` cache is always cold, so
/// CI always pays full regeneration regardless of this flag) but because it is
/// free and safe on a warm local machine: `crate::cli::cache::is_lang_cached`
/// only reports a cache hit when the IR+config hash still matches *and* every
/// previously recorded output path still exists on disk, so skipping a
/// cache-hit language can never hide a genuinely missing or frozen file. ~keep
pub(crate) fn find_missing_and_frozen_generated_files(
    languages: &[crate::core::config::Language],
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
    config_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> anyhow::Result<MissingAndFrozenFiles> {
    let (surface, stage_failures) = collect_managed_surface(languages, api, config, config_path, base_dir)?;
    // `surface` alone omits post-build-owned paths; see `verify_orphans::post_build_owned_paths`. ~keep
    let mut managed_paths: std::collections::HashSet<std::path::PathBuf> =
        surface.iter().map(|file| base_dir.join(&file.path)).collect();
    managed_paths.extend(super::verify_orphans::post_build_owned_paths(
        languages, config, base_dir,
    ));
    let (missing, missing_gitignored) =
        super::verify_gitignore::split_missing_by_gitignore(base_dir, &missing_managed_paths(&surface, base_dir));
    let mut result = MissingAndFrozenFiles {
        missing,
        missing_gitignored,
        frozen: frozen_managed_paths(&surface, base_dir, &rewritten_output_roots(config, base_dir)),
        drifted: drifted_marked_paths(&surface, base_dir),
        managed_paths,
        stage_failures: stage_failures
            .into_iter()
            .map(|failure| format!("[{}] {}", failure.stage, failure.message))
            .collect(),
    };
    result.missing.sort();
    result.missing.dedup();
    result.missing_gitignored.sort();
    result.missing_gitignored.dedup();
    result.frozen.sort_by(|a, b| a.path.cmp(&b.path));
    result.frozen.dedup_by(|a, b| a.path == b.path);
    result.drifted.sort();
    result.drifted.dedup();
    result.stage_failures.sort();
    result.stage_failures.dedup();
    Ok(result)
}

/// Absolute paths of every file in `files` that already exists on disk, already carries alef's
/// own provenance marker, and yet no longer matches the bytes this run's fresh render would
/// produce.
///
/// THE GAP this closes (alef#436): a public type gaining a field left `alef docs`' rendered API
/// reference pages stale while `alef verify` stayed green, because every check `alef verify` ran
/// was structurally blind to it:
///
/// - [`stale_among`]'s per-file `alef:hash:` check recomputes a hash of the file's OWN current
///   bytes and compares it to the hash embedded in those same bytes — it catches a hand-edit,
///   never a file that is internally self-consistent but stale relative to a fresh render.
/// - The crate-scoped `inputs_hash` record (`cache::generation_record`,
///   `bin_cli::core_commands::verify::run`) is written only by `alef generate`/`alef all`, never
///   by `alef docs` — so a docs-only regeneration gap never re-stamps it, but neither does
///   leaving docs untouched ever unstamp it: `alef generate` re-stamps the very baseline this
///   check would need to catch the drift, from a run that never touched a single docs page.
/// - [`missing_managed_paths`]/[`frozen_managed_paths`] only ever ask "does this path exist" and
///   "does it carry a marker" — never "do its bytes match", so a marked, present, stale file is
///   invisible to both.
///
/// Scoped to `.rs` and `.md` paths ONLY — deliberately narrower than "every stage
/// `collect_managed_surface` renders." MEASURED, not assumed: an earlier version of this
/// check ran over the whole surface and broke
/// `verify_reports_an_incomplete_generation_run_instead_of_ordinary_staleness` and two
/// sibling tests, all three asserting the established, pre-existing contract "a completed
/// `alef all` run leaves a tree `alef verify` reports clean." The false positives were
/// `pyproject.toml`, `poly.toml` (TOML, via poly's `taplo`) and a generated `.py` file (via
/// poly's `ruff`): `alef all`'s "Formatting generated files..." stage runs `poly fmt --fix`
/// (`cli::pipeline::format::poly_format`) over the paths it just wrote, AFTER
/// `collect_managed_surface`'s in-memory render is computed, and that external reformatting
/// pass changes bytes `normalize_content` never claims to predict. `normalize_content` only
/// reproduces two of poly's engines today — `rustfmt` for `.rs` (`format_rust_content`) and a
/// blank-line policy for `.md` deliberately matched to poly's `rumdl` (see that function's own
/// doc: "matching downstream rumdl-fmt... so cold and hot paths converge") — so those two
/// extensions are the only ones where an in-memory render is a faithful stand-in for the bytes
/// `alef all` actually leaves on disk. Every other poly-formatted language (Python, JSON,
/// JS/TS, and TOML among them) would false-positive on this check on every clean repo,
/// systemically, not as an edge case — reproducing the `normalize_content`-vs-`poly fmt` gap
/// for every one of them is real future work, not something this fix can absorb by widening a
/// comparison that only two extensions can currently make honest. Restricting scope here is a
/// consequence of that measurement, not a tuning pass to silence a test. ~keep
///
/// Deliberately excludes every file [`frozen_managed_paths`] would already report: an unmarked
/// file is that check's condition, not this one's (this function's own `content_has_alef_marker`
/// guard below is what enforces the split), and reporting the same withheld write under two
/// headings would describe it as two different findings with two different remedies.
///
/// Reuses [`crate::cli::commands::adopt::managed_outputs`] and
/// [`crate::cli::pipeline::matches_alef_output`] — the exact pairing [`frozen_managed_paths`]
/// already uses for its own `drifted` field — rather than a bespoke comparison, specifically
/// because of the caveat that pairing already solves: a self-marking backend (pyo3's `lib.rs`,
/// `docs::render`'s HTML-commented pages) or a `generated_header: true` file both legitimately
/// differ from a naive in-memory render by exactly the provenance header/hash line, and
/// `matches_alef_output` is the one place that discounts it. A hand-rolled byte comparison here
/// would report permanent drift for every one of those files. ~keep
fn drifted_marked_paths(files: &[crate::core::backend::GeneratedFile], base_dir: &std::path::Path) -> Vec<String> {
    files
        .iter()
        .filter_map(|file| {
            let full_path = base_dir.join(&file.path);
            if !render_predicts_final_bytes(&full_path) {
                return None;
            }
            let existing = std::fs::read_to_string(&full_path).ok()?;
            if !crate::core::hash::content_has_alef_marker(&existing) {
                // Unmarked: `frozen_managed_paths`'s territory, not this check's.
                return None;
            }
            let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(file), base_dir);
            let up_to_date = rendered.first().is_some_and(|output| {
                crate::cli::pipeline::matches_alef_output(&full_path, &existing, &output.content)
            });
            (!up_to_date).then(|| full_path.display().to_string())
        })
        .collect()
}

/// Whether [`crate::cli::pipeline::normalize_content`]'s in-memory normalization is a faithful
/// stand-in for the bytes `alef generate`/`alef all` actually leave on disk at `path` — see
/// [`drifted_marked_paths`]'s doc for the measured evidence this narrows to exactly
/// `normalize_content`'s own two special-cased extensions, no others. ~keep
fn render_predicts_final_bytes(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "rs" || extension == "md")
}

/// The absolute output roots this configuration writes with `overwrite = true`.
///
/// Both e2e stages call `pipeline::write_scaffold_files_report(.., true)`
/// (`bin_cli::all_commands::e2e_stage`), which is what makes their output the one part of the
/// tree where `can_skip` never fires and a pre-existing unmarked file is a write refused on
/// every run rather than a create-once seed nobody is attempting to touch. Every other writer
/// this report covers is `overwrite = false`. Derived from the resolved config rather than from
/// the default directory names, so a consumer that renames either root is still described
/// correctly. See [`frozen::FrozenFile::rewritten_every_run`]. ~keep
fn rewritten_output_roots(
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    config
        .e2e
        .iter()
        .flat_map(|e2e| [base_dir.join(&e2e.output), base_dir.join(&e2e.registry.output)])
        .collect()
}

/// Insert `files` into `surface`, letting a later stage win the path.
///
/// Last-write-wins mirrors disk: `alef all` runs these stages in sequence and each
/// one overwrites whatever the previous stage left at the same path, so the bytes a
/// reader is asked to consent to (and the bytes `alef verify` reasons about) must be
/// the *last* stage's, not the first's. The only stages that actually collide are
/// local-mode e2e and registry-mode test apps, which share the snippet output root. ~keep
fn absorb_stage(
    surface: &mut std::collections::BTreeMap<std::path::PathBuf, crate::core::backend::GeneratedFile>,
    files: Vec<crate::core::backend::GeneratedFile>,
) {
    for file in files {
        surface.insert(file.path.clone(), file);
    }
}

/// One [`collect_managed_surface`] stage that rendered files and also reported a
/// failure alongside them.
///
/// Only the two e2e stages can produce one of these today: they are the only stages
/// whose `Result` already separates "files it rendered" from "whether it is happy with
/// them" (see `generate_e2e`'s doc) — every other stage's failure has no partial output
/// to report and stays a hard `collect_managed_surface` error via `?`. `paths` is what
/// that stage's own invocation actually rendered, kept so a caller with a specific
/// target (`alef adopt`) can decide whether this failure is even about a path it asked
/// for, via [`Self::affects_any`]. `alef verify` has no target and reports every one of
/// these unconditionally instead. ~keep
pub(crate) struct StageFailure {
    pub(crate) stage: &'static str,
    pub(crate) message: String,
    pub(crate) paths: Vec<std::path::PathBuf>,
}

impl StageFailure {
    /// Whether any of `targets` could have come from this stage — derived from
    /// [`crate::cli::commands::adopt::matches_target`], the identical predicate `alef
    /// adopt`'s own candidate selection trusts, rather than a second, hand-maintained
    /// notion of what counts as a match. ~keep
    pub(crate) fn affects_any(&self, targets: &[String]) -> bool {
        self.paths.iter().any(|path| {
            targets
                .iter()
                .any(|target| crate::cli::commands::adopt::matches_target(target, path))
        })
    }
}

/// Every file this crate's configuration would cause alef to emit, from **all**
/// generation stages, deduplicated by path.
///
/// This is the single answer to "what does alef own here", and it has exactly two
/// consumers: [`find_missing_and_frozen_generated_files`] (`alef verify`'s report)
/// and `bin_cli::adopt_command` (the remedy the report points
/// at). They are the report and the fix for the same fact, so they must not be able
/// to disagree — and they did. Both were built from hand-maintained stage lists
/// that each omitted different stages: adopt covered bindings + stubs + scaffold and
/// verify covered those plus service/public API, and *neither* covered e2e, test
/// apps, READMEs or docs. Consumer repos then committed regenerated e2e snippet
/// `.md` files that carried no marker (`e2e::snippets::render_snippet_markdown` did
/// not route through `docs::render::with_html_header` at the time), so the write
/// guard froze 15,677 of them in one repo and 9,139 in another, `alef verify` was
/// structurally blind to every one, and `alef adopt` on a snippet glob bailed with
/// "no alef-managed output matches" — the designed way out did not reach the files
/// that needed it. A new emitter added to `alef all` must be added here too, or it
/// re-opens exactly that hole. ~keep
///
/// Every stage below is a pure in-memory render; nothing here writes to disk. Docs
/// are the one stage whose failure is downgraded: `docs::generate_docs_stage`
/// deliberately returns the pages it rendered *alongside* an error from a later
/// sub-step (snippet validation, CLI/MCP extraction), and neither of this function's
/// consumers should lose the whole managed set — or refuse to unfreeze a binding
/// file — because a docs sub-step is unhappy. ~keep
///
/// Returns the surface alongside every [`StageFailure`] this call tolerated rather than
/// aborting on. Both consumers used to see a stage failure as `Err` for the *whole*
/// function -- including `alef verify`, a read-only report, which would abort before
/// its own frozen-file walk ever ran, and `alef adopt`, which would refuse a path with
/// no relationship whatsoever to the failing stage. Neither reads a `Result::Err` here
/// as license to skip reporting the failure: `alef verify` prints every
/// [`StageFailure`] as its own section (see [`find_missing_and_frozen_generated_files`]);
/// `alef adopt` bails with it when [`StageFailure::affects_any`] says the operator's own
/// target could have come from that stage, and otherwise logs it at DEBUG and proceeds.
/// A caller that dropped the returned `Vec<StageFailure>` on the floor would silently
/// reintroduce the exact bug this return shape exists to prevent -- a report that looks
/// clean because the thing that would have said otherwise never ran. ~keep
pub(crate) fn collect_managed_surface(
    languages: &[crate::core::config::Language],
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
    config_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> anyhow::Result<(Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>)> {
    // Every stage below reads the same `&api`/`&config` and returns owned files, so rendering
    // them is embarrassingly parallel -- and rendering is where the time goes: the two e2e
    // stages alone emit several thousand files each on a full consumer tree, which is most of
    // what made a single-path `alef adopt` cost half a minute.
    //
    // ABSORPTION is not parallel-safe, and the distinction is the whole point of this shape:
    // `absorb_stage` is last-wins on a path collision, so the sequence stages are folded into
    // `surface` is what decides which stage owns a contested path. Render concurrently, fold in
    // the original order -- `rayon`'s indexed `collect` preserves it. ~keep
    type StageOutcome = anyhow::Result<(Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>)>;
    type Stage<'a> = Box<dyn Fn() -> StageOutcome + Send + Sync + 'a>;

    // `generate_service_api` already no-ops when `api.services` is empty, so it is
    // safe to call unconditionally. `generate_public_api` has no such internal
    // guard — mirror `Commands::Generate`'s `config.generate.public_api` gate so
    // this stays a faithful picture of what `alef generate` would produce, not a
    // superset of it.
    //
    // `clean: true`, not `false`: `pipeline::generate`'s own `clean` parameter gates a
    // language-level cache short-circuit (`!clean && cache::is_lang_cached(..)` in
    // `generate/generation.rs`) that returns zero files for a language whose lang-hash
    // already matches the on-disk `.alef/<crate>/hashes/<lang>.hash` -- exactly the state
    // every language is in immediately after the `alef generate` run that wrote it, which is
    // the single most common moment `alef verify` runs. `clean: false` here silently dropped
    // every one of that language's bindings-stage files from `surface` in precisely that
    // steady state, which `missing_managed_paths`/`frozen_managed_paths` tolerate (an absent
    // `surface` entry just means that path is not checked) but which made every one of that
    // language's self-marking bindings files -- `packages/python/lib.rs` and its pyo3
    // equivalents on every other self-marking backend -- a guaranteed, permanent orphan false
    // positive on exactly the run `alef verify` is normally expected to pass. `clean: true`
    // forces full in-memory regeneration regardless of cache state, matching this function's
    // own contract: `surface` is "what this run's backends would produce," not "what they
    // would bother producing given whatever happens to already be cached." ~keep
    let bindings_stage: Stage<'_> = Box::new(|| {
        let mut files = Vec::new();
        for (_, produced) in crate::cli::pipeline::generate(api, config, languages, true, config_path, false)? {
            files.extend(produced);
        }
        for (_, produced) in crate::cli::pipeline::generate_service_api(api, config, languages)? {
            files.extend(produced);
        }
        Ok((files, Vec::new()))
    });
    let scaffold_stage: Stage<'_> = Box::new(|| {
        let mut files = crate::cli::pipeline::scaffold(api, config, languages, config_path)?;
        for (_, produced) in crate::cli::pipeline::generate_stubs(api, config, languages)? {
            files.extend(produced);
        }
        if config.generate.public_api {
            for (_, produced) in crate::cli::pipeline::generate_public_api(api, config, languages, config_path)? {
                files.extend(produced);
            }
        }
        Ok((files, Vec::new()))
    });
    // One diagnostic log across both e2e renders below. The two differ only in `dep_mode`,
    // which no validator reads, so the registry pass recomputes a bit-identical diagnostic set;
    // sharing the log reports each finding once per crate instead of twice. Scoped to this call,
    // not a static, so a later invocation's identical finding is still reported -- see
    // `crate::e2e::diagnostic_log`. ~keep
    let diagnostic_log = crate::e2e::diagnostic_log::DiagnosticLog::new();

    // Both e2e modes, because they emit to different roots (`e2e.output` versus
    // `e2e.registry.output`) and a file can be frozen under either. `generate_e2e`
    // returns its per-backend generator failure alongside the files it did produce
    // rather than folding it into this `Result`, so a failure here no longer aborts
    // `collect_managed_surface` (as it used to, with `alef verify`'s frozen-file walk
    // and every later stage as collateral): it is absorbed into `stage_failures`
    // instead, and every caller of this function is required to look at that list --
    // see this function's own doc for why neither consumer may drop it silently. ~keep
    let e2e_local_stage: Stage<'_> = Box::new(|| {
        let Some(e2e_config) = &config.e2e else {
            return Ok((Vec::new(), Vec::new()));
        };
        let (files, generator_error) = crate::e2e::generate_e2e_with_log(
            config,
            e2e_config,
            None,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
            &diagnostic_log,
        )
        .context("failed to render the e2e stage of alef's managed output")?;
        Ok(stage_failure_for("e2e", generator_error, files))
    });
    let e2e_registry_stage: Stage<'_> = Box::new(|| {
        let Some(e2e_config) = &config.e2e else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut registry_config = e2e_config.clone();
        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
        let (files, generator_error) = crate::e2e::generate_e2e_with_log(
            config,
            &registry_config,
            None,
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
            &diagnostic_log,
        )
        .context("failed to render the registry-mode test-app stage of alef's managed output")?;
        Ok(stage_failure_for("test-apps (registry mode)", generator_error, files))
    });
    let readme_stage: Stage<'_> = Box::new(|| {
        let readme_languages = crate::readme::expand_configured_readme_languages(config, languages);
        Ok((
            crate::cli::pipeline::readme(api, config, &readme_languages)?,
            Vec::new(),
        ))
    });
    // Neither `alef adopt` nor `alef verify` asks whether a snippet compiles, type-checks, or
    // runs -- they ask what file surface alef's configuration owns, and that compile step
    // produces no files (see `generate_docs_stage_without_snippet_compile_validation`'s doc).
    // Running it here was the entire cost of the 90-minute `alef adopt` on a single `Cargo.toml`:
    // thousands of per-backend toolchain subprocess invocations to answer an ownership question
    // that never looked at their result, since a docs-stage `Err` is already downgraded to the
    // debug log below regardless of which sub-step produced it. ~keep
    let docs_stage: Stage<'_> = Box::new(|| {
        let doc_languages = resolve_doc_languages(config, None)?;
        let (doc_files, doc_result) = crate::docs::generate_docs_stage_without_snippet_compile_validation(
            api,
            config,
            &doc_languages,
            None,
            base_dir,
        );
        if let Err(error) = doc_result {
            tracing::debug!("docs stage reported {error:#}; using the pages it rendered before the failure");
        }
        Ok((doc_files, Vec::new()))
    });

    let stages = [
        bindings_stage,
        scaffold_stage,
        e2e_local_stage,
        e2e_registry_stage,
        readme_stage,
        docs_stage,
    ];
    let outcomes: Vec<StageOutcome> = {
        use rayon::prelude::*;
        stages.par_iter().map(|stage| stage()).collect()
    };

    let mut surface = std::collections::BTreeMap::new();
    let mut stage_failures = Vec::new();
    for outcome in outcomes {
        let (files, failures) = outcome?;
        stage_failures.extend(failures);
        absorb_stage(&mut surface, files);
    }

    Ok((surface.into_values().collect(), stage_failures))
}

/// Pair a stage's rendered files with the [`StageFailure`] its generator reported alongside them,
/// if any. Split out only so the two e2e stage closures state the pairing once instead of twice.
fn stage_failure_for(
    stage: &'static str,
    generator_error: Option<anyhow::Error>,
    files: Vec<crate::core::backend::GeneratedFile>,
) -> (Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>) {
    let failures = generator_error
        .map(|error| StageFailure {
            stage,
            message: format!("{error:#}"),
            paths: files.iter().map(|file| file.path.clone()).collect(),
        })
        .into_iter()
        .collect();
    (files, failures)
}

/// The stale subset of an ownership walk's output: every scanned file whose embedded
/// `alef:hash:` does not equal a fresh content-only hash of its own on-disk bytes.
///
/// Takes the walk's result rather than a directory so a caller that needs both the staleness
/// verdict AND the walk's own coverage tally (`alef verify`, via
/// [`super::verify_scan::scan`]) can get them from ONE walk. Two walks would be two
/// answers to "what is on disk", free to disagree about the very set the report describes.
///
/// This is a pure content check -- see [`crate::core::hash::compute_file_hash`]'s doc for
/// why it takes no generation-inputs argument. It cannot by itself tell "this file's inputs
/// changed since it was generated"; that coarser, crate-scoped question is answered
/// separately by comparing the current `inputs_hash` against
/// `crate::cli::cache::generation_record`'s committed record (see `core::hash`'s module doc
/// and `bin_cli::core_commands::verify`). ~keep
pub(crate) fn stale_among(scanned: &[(std::path::PathBuf, Option<String>, String)]) -> Vec<StaleMismatch> {
    let mut stale: Vec<StaleMismatch> = scanned
        .iter()
        .filter_map(|(path, disk_hash, content)| {
            let computed = crate::core::hash::compute_file_hash(content);
            let embedded = disk_hash.clone();
            (embedded.as_deref() != Some(computed.as_str())).then(|| StaleMismatch {
                path: path.display().to_string(),
                embedded: embedded.unwrap_or_else(|| "<missing>".to_owned()),
                computed,
            })
        })
        .collect();

    stale.sort_by(|a, b| a.path.cmp(&b.path));
    stale
}

/// Walk the consumer's repo from `base_dir`, find every alef-headered file, and
/// return the list of stale ones — where the embedded `alef:hash:<hex>` does not
/// equal a fresh content-only hash of the file's own current bytes.
///
/// Skips obvious build/cache directories (`target/`, `node_modules/`, `_build/`,
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`), and every directory git considers
/// ignored (see [`super::verify_gitignore::gitignored_dirs`]) — a consumer's own
/// dependency-fetch cache or build-output directory is neither this run's generated output
/// nor its generation input — so verify stays fast and never claims foreign content. Files
/// without the alef header marker are skipped silently — those are user-owned (scaffold-once
/// Cargo.toml templates, composer.json, package.json, lockfiles, etc.) and alef has
/// no claim. The Ruby gemspec and `.rubocop.yml` are NOT in this category — both carry the
/// alef header (`generated_header: true`) and are alef-owned, overwritten on every `alef build`.
///
/// `cfg(test)` since `alef verify` moved to the single-walk [`stale_among`] path: the writer-side
/// stamp-scope tests deliberately assert against the reader side's own function rather than a
/// local restatement of it, so this stays the shared oracle for them even though the command no
/// longer routes through it. ~keep
#[cfg(test)]
pub(crate) fn verify_walk(base_dir: &std::path::Path) -> anyhow::Result<Vec<StaleMismatch>> {
    Ok(stale_among(&collect_alef_hashes(base_dir)))
}

#[cfg(all(test, unix))]
mod complete_generated_artifacts_staging_order_tests;
#[cfg(test)]
mod drift_tests;
#[cfg(test)]
mod tests;
