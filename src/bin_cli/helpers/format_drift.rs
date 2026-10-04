//! The alef#436 marked-file drift comparison ("does this already-marked, already-present file
//! still match what this run would produce") -- alef#436's second half, and the home for
//! [`drifted_marked_paths`], the two-tier comparison `bin_cli::helpers::find_missing_and_frozen_
//! generated_files` calls into.
//!
//! # What this has to get right
//!
//! A fresh in-memory render is NOT what ends up on disk. `alef all` writes the render and then
//! reformats it -- `poly fmt --fix`, `cargo fmt --all`, `cargo sort -n -w`, `mix format`, a
//! `[crates.e2e.format]` override -- and only then stamps the hash. So "did this file drift"
//! can only be answered against the render PLUS whatever alef's own pass does to it at that
//! path. Getting the "whatever" wrong in either direction is a real defect:
//!
//! - Model too much formatting and the check reports drift the writer can never clear.
//! - Model too little and it certifies a stale file as fresh.
//!
//! [`crate::cli::pipeline::formatting_owner`] is the single answer to "what formats this path",
//! shared with the writer. This module consumes it; it does not keep its own copy.
//!
//! # The alef#478 defect
//!
//! This module used to hold a second, one-line, wrong copy of that answer: *`poly fmt`, always*,
//! applied to a temp sibling of the real file. In a consumer repo that manufactured **197**
//! drift findings against a tree two consecutive `alef all` runs left byte-identical -- **zero**
//! of the 197 overlapped the set `alef all` actually rewrites. Three independent causes, all
//! measured:
//!
//! 1. **poly's excludes are name-keyed, and a temp sibling escapes them.** The real
//!    `crates/<crate>-ffi/Cargo.toml` is excluded by the consumer's `**/Cargo.toml`;
//!    `.alef-verify-drift-<rand>.toml` beside it is not. The check compared disk against a
//!    taplo-reformatted rendering of a file poly never touches.
//! 2. **Trees poly's discovery prunes.** A gitignored directory, or a bare `e2e/**` glob that
//!    (per poly's own documentation) also prunes `src/test/java/io/<org>/e2e/`, where `e2e` is a
//!    package name.
//! 3. **Formatters the check modelled not at all** -- the residuals and `[crates.e2e.format]`
//!    overrides above. Elixir was the sharpest: poly is excluded from `.ex`/`.exs` on BOTH
//!    sides, so the temp copy was a no-op and the check demanded that disk equal the raw
//!    emitter output while the writer had run `mix format` over it.
//!
//! Measured against the same consumer: of those 197 paths, poly's own pass reaches **32**; the
//! other **165** it never sees. ~keep
//!
//! # Why `.rs` and `.md` are no longer symmetric (alef#458, alef#465, alef#469, alef#478)
//!
//! Both extensions once took a fast path that skipped the formatter entirely, on the theory
//! that [`crate::cli::pipeline::normalize_content`] reproduces what poly would do to them.
//! That is true for `.rs` and **false for `.md`**, and alef#458 got here by reasoning about
//! implementation symmetry instead of measuring each one.
//!
//! `.rs` keeps the fast path only when poly is absent. `normalize_content` runs a REAL `rustfmt`
//! (`format_rust_content`), so that prediction is exact when no later formatter runs. With poly
//! present, however, the writer gives poly the final word and verification must do the same:
//! consumer configuration can make poly's Rust pass differ from the in-memory normalization.
//! alef#465 still proves the no-poly case end to end with a real from-scratch `alef all`. ~keep
//!
//! `.md` loses it whenever poly is present. alef models exactly one rumdl rule: a blank-line cap
//! matched to **MD012** (`normalize_whitespace_with_policy`). The poly config alef itself
//! generates (`scaffold::languages::poly`'s `RUMDL_DISABLE`) **disables MD012** -- so the one
//! rule alef reproduces is the one rule that is switched off, and every rule that actually
//! reshapes alef's markdown is left enabled and unmodelled.
//!
//! Measured in a consumer, by disabling each rule in turn and re-running the real formatter over
//! the raw render: a cold `alef readme` left **11 of 16** generated READMEs differing from
//! committed and a cold `alef docs` **18 of 23** reference pages, and a real
//! `poly fmt --fix --fix-generated` reconciled **all 29** exactly. The responsible rules are
//! **MD032** (blanks around lists -- all 11 READMEs), **MD038** (spaces inside code spans -- 17
//! of the 18 docs pages), **MD022** (blanks around headings -- 2 pages) and **MD040** (1). Note
//! MD032 and MD022 are one emitter defect wearing two rule names: alef emits a heading with no
//! blank line after it, and which rule repairs it depends only on whether a list or prose
//! follows. **Zero** of the 29 differ by MD022 alone, so a prediction written against MD022 --
//! the first and wrong guess at this -- would have missed every README.
//!
//! The writer was a correct fixed point the whole time and this check was wrong about all 29.
//! With poly ABSENT the prediction is exact again -- nothing reformats the file after alef
//! writes it -- so the fast path is kept for that case and only that case.
//!
//! The lasting lesson is why this module no longer enumerates rules at all: the fix is not to
//! model MD032 and MD038 too, it is to stop predicting and ask the real formatter. Any list of
//! rules here would be one poly release away from being wrong again. ~keep
//!
//! # Why the temp files live beside the real file, not in a scratch directory
//!
//! poly resolves `poly.toml` (and each engine's own project markers -- `pyproject.toml`,
//! `tsconfig.json`, ...) by walking up from the target file's own directory. A scratch directory
//! elsewhere would need every one of those mirrored alongside it, which is exactly the
//! "reproduce poly's behaviour" trap this module exists to avoid. A temporary child directory
//! beside the real file preserves its exact basename while resolving the same ancestor configs.
//! Its name is deliberately not dot-prefixed: poly prunes hidden directories before formatter
//! dispatch, so a hidden staging directory makes every comparison use the raw render even when
//! the writer formatted the real file. ~keep
//!
//! Staging is now confined to paths [`crate::cli::pipeline::FormattingOwner::Poly`] actually
//! claims. `alef verify` is documented read-only, and on the consumer above the old
//! unconditional version wrote **2156** temp files into the working tree on a single run. ~keep
//!
//! # Why one batched `poly fmt --fix` invocation, not one per file
//!
//! `poly` is a subprocess; spawning one per candidate turns a verify run into hundreds of
//! process spawns. Staged files are batched by the writer context that owns them: one repo-root
//! invocation plus one per E2E language directory represented in the candidates. ~keep

use crate::cli::pipeline::{FormattingOwner, PolyCoverage, formatting_owner};
use crate::core::config::ResolvedCrateConfig;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One file whose fast (`.rs`) prediction does not apply and whose bytes poly does shape,
/// paired with its current disk content and this run's freshly rendered content -- already
/// through [`crate::cli::commands::adopt::managed_outputs`]'s normalization/header pass, i.e.
/// the bytes `write_files` would put on disk BEFORE the post-write `poly fmt --fix` pass.
pub(super) struct RealFormatCandidate {
    pub(super) full_path: PathBuf,
    pub(super) disk_content: String,
    pub(super) rendered_content: String,
    pub(super) format_context: PathBuf,
}

/// What [`drifted_marked_paths`] was able to say about each candidate, and what it could not.
///
/// Surfaced all the way to `alef verify`'s own report (`bin_cli::core_commands::verify`) so a
/// gap is a loud, counted skip rather than a silent pass -- this repo's dominant defect shape is
/// a check that examines nothing and still reports clean (the `prove-the-check-fired` rule).
/// No skip is ever folded into `compared`: they answer different questions -- "did the check
/// run" and "what did it find" -- and a report that only stated the sum could not tell a clean,
/// fully-examined tree from one this call gave up on. ~keep
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormatDriftStats {
    pub(crate) compared: usize,
    pub(crate) skipped_missing_formatter: usize,
    /// Candidates whose temp copy could not be written at all -- a read-only checkout, a
    /// permission-denied directory. Counted apart from `skipped_missing_formatter` because the
    /// remedy is different and a report that blamed a missing `poly` for a read-only tree would
    /// send the reader after the wrong thing. ~keep
    pub(crate) skipped_staging_error: usize,
    /// Candidates alef reformats with a tool whose output it does not model, so no honest
    /// byte-level prediction exists: a `[crates.e2e.format]` override (opaque user shell), or a
    /// residual `cargo sort -n -w` / `mix format` pass.
    ///
    /// The alternative to counting these is to compare against the raw render anyway, which is
    /// precisely what produced alef#478's 197 false findings. Passing them silently would be
    /// the opposite defect. Counting is the only honest third option, and it is why this field
    /// exists rather than the paths simply being dropped. ~keep
    pub(crate) skipped_no_faithful_prediction: usize,
    /// Candidates settled by byte equality with the render itself, with no formatter tier
    /// involved -- the `.rs`/poly-absent-`.md` fast paths and every
    /// [`FormattingOwner::None`] path, i.e. exactly the files where alef runs nothing that
    /// could reshape them and the render therefore IS the final bytes.
    ///
    /// Reported beside `compared` rather than folded into it because the two are different
    /// grades of evidence: `compared` means a real formatter ran and the result matched, this
    /// means no formatter was needed. Folding them would hide which of the two a clean report
    /// rested on. ~keep
    pub(crate) matched_render_exactly: usize,
}

impl FormatDriftStats {
    /// Fold one crate's counters into a multi-crate total.
    ///
    /// Exhaustively destructured rather than field-by-field `+=`, so adding a counter to this
    /// struct fails to compile here instead of silently reporting zero for it across every
    /// multi-crate run -- the same class of quiet under-count this whole type exists to make
    /// impossible. ~keep
    pub(crate) fn add(&mut self, other: Self) {
        let Self {
            compared,
            skipped_missing_formatter,
            skipped_staging_error,
            skipped_no_faithful_prediction,
            matched_render_exactly,
        } = other;
        self.compared += compared;
        self.skipped_missing_formatter += skipped_missing_formatter;
        self.skipped_staging_error += skipped_staging_error;
        self.skipped_no_faithful_prediction += skipped_no_faithful_prediction;
        self.matched_render_exactly += matched_render_exactly;
    }
}

/// Compare every `candidates` entry against disk through a real `poly fmt --fix` pass over a
/// colocated temp copy of its rendered bytes -- see the module doc for why the exact basename and
/// writer working directory are preserved and why formatting is batched. Returns the drifted subset (as `full_path.display()` strings, matching every
/// other list in [`super::MissingAndFrozenFiles`]) alongside [`FormatDriftStats`].
///
/// Gates the up-front "is poly even installed" decision through the injected `is_available`
/// rather than reading PATH directly, so the skip-and-count branch is provable without
/// depending on whether the host running the suite happens to have `poly`. There is
/// The production path always runs the real formatter; the runner seam exists only so tests can
/// prove filename-based dispatch without depending on the host's configured engines. ~keep
fn real_formatter_drift_with(
    candidates: Vec<RealFormatCandidate>,
    base_dir: &Path,
    is_available: &dyn Fn(&str) -> bool,
) -> (Vec<String>, FormatDriftStats) {
    real_formatter_drift_with_runner(
        candidates,
        base_dir,
        is_available,
        &crate::cli::pipeline::poly_format_strict,
    )
}

fn real_formatter_drift_with_runner(
    candidates: Vec<RealFormatCandidate>,
    _base_dir: &Path,
    is_available: &dyn Fn(&str) -> bool,
    run_formatter: &dyn Fn(&[PathBuf], &Path) -> anyhow::Result<()>,
) -> (Vec<String>, FormatDriftStats) {
    if candidates.is_empty() {
        return (Vec::new(), FormatDriftStats::default());
    }
    if !is_available("poly") {
        return (
            Vec::new(),
            FormatDriftStats {
                skipped_missing_formatter: candidates.len(),
                ..FormatDriftStats::default()
            },
        );
    }

    let temp_files: Vec<Option<StagedFormatFile>> = candidates
        .iter()
        .map(
            |candidate| match write_sibling_temp_file(&candidate.full_path, &candidate.rendered_content) {
                Ok(temp_file) => Some(temp_file),
                Err(error) => {
                    tracing::debug!(
                        path = %candidate.full_path.display(),
                        "could not stage a drift-check temp copy ({error}); skipping this file's \
                         formatted-output comparison"
                    );
                    None
                }
            },
        )
        .collect();

    let mut paths_by_context: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    for (candidate, temp_file) in candidates.iter().zip(&temp_files) {
        if let Some(temp_file) = temp_file {
            paths_by_context
                .entry(&candidate.format_context)
                .or_default()
                .push(temp_file.path().to_path_buf());
        }
    }
    for (context, paths) in paths_by_context {
        if let Err(error) = run_formatter(&paths, context) {
            tracing::warn!(
                context = %context.display(),
                "poly fmt over the drift-check temp copies failed (non-fatal): {error:#}"
            );
        }
    }

    let mut drifted = Vec::new();
    let mut compared = 0usize;
    let mut staging_errors = 0usize;
    for (candidate, temp_file) in candidates.iter().zip(temp_files.iter()) {
        let Some(temp_file) = temp_file else {
            staging_errors += 1;
            continue;
        };
        let formatted =
            std::fs::read_to_string(temp_file.path()).unwrap_or_else(|_| candidate.rendered_content.clone());
        compared += 1;
        if !crate::cli::pipeline::matches_alef_output(&candidate.full_path, &candidate.disk_content, &formatted) {
            drifted.push(candidate.full_path.display().to_string());
        }
    }

    (
        drifted,
        FormatDriftStats {
            compared,
            skipped_staging_error: staging_errors,
            ..FormatDriftStats::default()
        },
    )
}

/// Write `content` under its exact basename in a securely-named temp directory beside
/// `real_path` -- see the module doc for why this must stay colocated rather than use scratch.
///
/// `TempDir` is deliberate: it preserves the exact basename formatter dispatch needs and deletes
/// itself on drop, including after a panic or early return. ~keep
struct StagedFormatFile {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl StagedFormatFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn write_sibling_temp_file(real_path: &Path, content: &str) -> std::io::Result<StagedFormatFile> {
    let parent = real_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = real_path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "formatter candidate has no file name"))?;
    // A dot-prefixed temp directory is invisible to poly's discovery, even when its contained
    // file is passed explicitly. Security comes from tempfile's random name and permissions,
    // not from hiding the directory. ~keep
    let directory = tempfile::Builder::new()
        .prefix("alef-verify-drift-")
        .tempdir_in(parent)?;
    let path = directory.path().join(file_name);
    std::fs::write(&path, content)?;
    Ok(StagedFormatFile {
        _directory: directory,
        path,
    })
}

/// Absolute paths of every file in `files` that already exists on disk, already carries alef's
/// own provenance marker, and yet no longer matches the bytes this run's fresh render would
/// produce once alef's own formatting pass has had its say.
///
/// THE GAP this closes (alef#436): a public type gaining a field left `alef docs`' rendered API
/// reference pages stale while `alef verify` stayed green, because every other check was
/// structurally blind to it:
///
/// - [`super::stale_among`]'s per-file `alef:hash:` check recomputes a hash of the file's OWN
///   current bytes and compares it to the hash embedded in those same bytes — it catches a
///   hand-edit, never a file that is internally self-consistent but stale relative to a fresh
///   render.
/// - The crate-scoped `inputs_hash` record (`cache::generation_record`,
///   `bin_cli::core_commands::verify::run`) is written only by `alef generate`/`alef all`, never
///   by `alef docs` — so a docs-only regeneration gap never re-stamps it, but neither does
///   leaving docs untouched ever unstamp it.
/// - `missing_managed_paths`/`frozen_managed_paths` only ever ask "does this path exist" and
///   "does it carry a marker" — never "do its bytes match".
///
/// Three tiers, dispatched by [`formatting_owner`] (see the module doc for the alef#478 defect
/// that having a second, private answer to the same question produced):
///
/// - **Nothing reformats it** ([`FormattingOwner::None`]) — the render IS the final bytes, so
///   compare them directly. This is the large majority of a real tree, and the tier the old
///   code was missing entirely: it staged a temp copy and ran poly over paths poly never sees.
/// - **poly reformats it** ([`FormattingOwner::Poly`]) — run the REAL `poly fmt --fix` over the
///   rendered bytes under the same basename in a colocated temp directory and compare the result. Exact rather than
///   approximate, and immune to poly gaining or losing an engine.
/// - **alef reformats it with something it cannot model** ([`FormattingOwner::Residual`],
///   [`FormattingOwner::E2eOverride`]) — counted into
///   [`FormatDriftStats::skipped_no_faithful_prediction`] and reported, never silently passed.
///
/// `.rs` short-circuits ahead of all three, and `.md` does too when poly is absent -- see
/// [`render_predicts_final_bytes`] and the module doc for why those two are not the same case.
///
/// Deliberately excludes every create-once seed: after its first creation the writer preserves
/// its consumer-owned bytes, so a current render is not an authoritative freshness prediction.
/// Also excludes every file `frozen_managed_paths` would already report: an unmarked
/// file is that check's condition, not this one's, and reporting the same withheld write under
/// two headings would describe it as two findings with two remedies. Also excludes
/// [`crate::cli::pipeline::is_base64_binary_output`] paths.
///
/// Reuses [`crate::cli::commands::adopt::managed_outputs`] and
/// [`crate::cli::pipeline::matches_alef_output`] — the exact pairing `frozen_managed_paths`
/// already uses — rather than a bespoke comparison, because of the caveat that pairing already
/// solves: a self-marking backend (pyo3's `lib.rs`, `docs::render`'s HTML-commented pages) or a
/// `generated_header: true` file both legitimately differ from a naive render by exactly the
/// provenance header/hash line, and `matches_alef_output` is the one place that discounts it. ~keep
pub(super) fn drifted_marked_paths(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &Path,
    config: &ResolvedCrateConfig,
) -> (Vec<String>, FormatDriftStats) {
    drifted_marked_paths_with(files, base_dir, config, &crate::cli::pipeline::is_tool_available)
}

/// Testable seam for [`drifted_marked_paths`]: resolves poly's availability through
/// `is_available` rather than PATH, so both the counted-skip branch and the `.md` fast path's
/// poly-absent case are provable on a host that does have poly installed. ~keep
fn drifted_marked_paths_with(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &Path,
    config: &ResolvedCrateConfig,
    is_available: &dyn Fn(&str) -> bool,
) -> (Vec<String>, FormatDriftStats) {
    let poly_available = is_available("poly");
    let coverage = PolyCoverage::probe(&poly_probe_roots(config, base_dir), base_dir);
    let declared = crate::cli::pipeline::declared_user_owned(base_dir)
        .unwrap_or_else(|_| crate::core::config::UserOwnedPaths::none());
    let mut drifted = Vec::new();
    let mut real_format_candidates = Vec::new();
    let mut unpredictable = 0usize;
    let mut matched_render = 0usize;
    for file in files {
        let full_path = base_dir.join(&file.path);
        if declared.matches(base_dir, &full_path) {
            continue;
        }
        let Ok(existing) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        if !crate::core::hash::content_has_alef_marker(&existing) {
            // Unmarked: `frozen_managed_paths`'s territory, not this check's.
            continue;
        }
        if crate::cli::pipeline::is_base64_binary_output(&file.path) {
            continue;
        }
        if is_create_once_for_drift(file) {
            continue;
        }
        let rendered = managed_output_for_drift(file, &existing, base_dir);
        let Some(output) = rendered.into_iter().next() else {
            continue;
        };
        let compare_directly = |drifted: &mut Vec<String>, matched: &mut usize| {
            if crate::cli::pipeline::matches_alef_output(&full_path, &existing, &output.content) {
                *matched += 1;
            } else {
                drifted.push(full_path.display().to_string());
            }
        };
        if render_predicts_final_bytes(&full_path, poly_available) {
            compare_directly(&mut drifted, &mut matched_render);
            continue;
        }
        match formatting_owner(&full_path, config, base_dir, &coverage) {
            FormattingOwner::None => compare_directly(&mut drifted, &mut matched_render),
            FormattingOwner::Poly => real_format_candidates.push(RealFormatCandidate {
                full_path,
                disk_content: existing,
                rendered_content: output.content,
                format_context: coverage
                    .format_context(&base_dir.join(&file.path))
                    .unwrap_or(base_dir)
                    .to_path_buf(),
            }),
            FormattingOwner::Residual(command) => {
                tracing::debug!(
                    path = %full_path.display(),
                    "no faithful prediction: alef reformats this path with `{command}`, whose output it does not model"
                );
                unpredictable += 1;
            }
            FormattingOwner::E2eOverride(command) => {
                tracing::debug!(
                    path = %full_path.display(),
                    "no faithful prediction: a [crates.e2e.format] override (`{command}`) replaces the poly pass here"
                );
                unpredictable += 1;
            }
        }
    }
    let (real_drifted, mut stats) = real_formatter_drift_with(real_format_candidates, base_dir, is_available);
    drifted.extend(real_drifted);
    stats.skipped_no_faithful_prediction = unpredictable;
    stats.matched_render_exactly = matched_render;
    (drifted, stats)
}

/// Whether the normal writer preserves an existing scaffold rather than treating this render as
/// authoritative. `.gitattributes` is the one self-marked create-once scaffold: its literal
/// `Generated by alef scaffold` line makes the generic classifier see a marker even though its
/// emitter sets `generated_header = false` and the normal scaffold stage uses overwrite=false.
/// Keep the exception exact so self-marked derived output remains freshness-checked. ~keep
fn is_create_once_for_drift(file: &crate::core::backend::GeneratedFile) -> bool {
    crate::cli::commands::adopt::is_create_once_seed(file)
        || (!file.generated_header && file.path == Path::new(".gitattributes"))
}

/// Prepare the bytes the writer would compare for drift, including the TOML merge that preserves
/// consumer-owned entries and comments in managed manifests such as `poly.toml`, and the
/// disk-aware marker preservation used for formerly generated create-once manifests. ~keep
fn managed_output_for_drift(
    file: &crate::core::backend::GeneratedFile,
    existing: &str,
    base_dir: &Path,
) -> Vec<crate::cli::commands::adopt::ManagedOutput> {
    let mut prepared = file.clone();
    if crate::core::hash::content_has_alef_marker(existing) {
        prepared.generated_header = true;
    }
    if file.generated_header && file.path == Path::new("poly.toml") {
        prepared.content = match crate::cli::pipeline::generate::merge_managed_toml_preview(
            existing,
            &file.content,
            base_dir,
            &file.path,
        ) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(
                    path = %file.path.display(),
                    "could not preview managed TOML merge while checking drift: {error:#}"
                );
                file.content.clone()
            }
        };
    }
    crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&prepared), base_dir)
}

/// The roots to ask poly about, matching the roots alef's own formatting pass hands it.
///
/// `base_dir` is what [`crate::cli::pipeline::converge_full_regen_formatting`] formats on a full
/// regen. Each existing e2e language directory is added because `e2e::format::format_language`
/// names each `<output>/<lang>` directory DIRECTLY and runs from there, and poly's discovery answers differently for a
/// directory it was handed than for the same directory reached by walking down from a parent --
/// a gitignored tree is the ordinary case. Asking about both is what keeps the reader's notion
/// of coverage equal to the writer's rather than merely similar. Non-existent roots are dropped:
/// poly errors on a path that is not there, which would take the whole probe down with it. ~keep
fn poly_probe_roots(config: &ResolvedCrateConfig, base_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![base_dir.to_path_buf()];
    if let Some(e2e) = &config.e2e {
        for output in [e2e.output.as_str(), e2e.registry.output.as_str()] {
            let output_root = base_dir.join(output);
            let Ok(entries) = std::fs::read_dir(output_root) else {
                continue;
            };
            let mut language_roots: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect();
            language_roots.sort();
            for path in language_roots {
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
    }
    roots
}

/// Whether [`crate::cli::pipeline::normalize_content`]'s in-memory normalization is a faithful
/// PREDICTION of the bytes `alef generate`/`alef all` actually leave on disk at `path`.
///
/// `.rs`: only when poly is absent. `normalize_content` runs a real `rustfmt`, but poly owns the
/// final bytes when installed and may apply different project configuration.
///
/// `.md`: only when poly is ABSENT. With poly present, rumdl reformats the page afterwards and
/// alef models only its MD012 rule -- which the poly config alef itself generates has DISABLED,
/// while leaving MD022 enabled. See the module doc for the measured alef#478 evidence, and for
/// why the two extensions must keep being reasoned about separately rather than kept symmetric.
///
/// Everything else goes to [`formatting_owner`], which answers from the shared policy rather
/// than from an extension list. ~keep
fn render_predicts_final_bytes(path: &Path, poly_available: bool) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => !poly_available,
        Some("md") => !poly_available,
        _ => false,
    }
}

/// Print `alef verify`'s formatted-output drift coverage line.
///
/// Printed unconditionally, on every run including a clean one, for the same reason
/// `VerifyCoverage` reports scan coverage unconditionally: a check that examined nothing must
/// never render identically to one that found a clean tree (the `prove-the-check-fired` rule).
/// Every skip is named with its own remedy, because they differ: install `poly`, make the
/// checkout writable, or accept that alef cannot speak for a path it reformats with a tool it
/// does not model. ~keep
pub(crate) fn report_format_drift_coverage(stats: FormatDriftStats) {
    let mut notes: Vec<String> = Vec::new();
    if stats.skipped_missing_formatter > 0 {
        tracing::warn!(
            "poly is not installed; {} generated file(s) could not be checked for formatted-output \
             drift (install poly to close this gap)",
            stats.skipped_missing_formatter
        );
        notes.push(format!(
            "{} SKIPPED because poly is not installed (install poly to close this gap -- see poly-lint-format)",
            stats.skipped_missing_formatter
        ));
    }
    if stats.skipped_staging_error > 0 {
        tracing::warn!(
            "{} generated file(s) could not be checked for formatted-output drift: their temp copy \
             could not be written (read-only checkout?)",
            stats.skipped_staging_error
        );
        notes.push(format!(
            "{} SKIPPED because a temp copy could not be written beside them",
            stats.skipped_staging_error
        ));
    }
    if stats.skipped_no_faithful_prediction > 0 {
        notes.push(format!(
            "{} NOT CHECKED because alef reformats them with a tool whose output it does not model \
             (a [crates.e2e.format] override, `cargo sort`, or `mix format`) -- no honest byte-level \
             prediction exists for these",
            stats.skipped_no_faithful_prediction
        ));
    }
    let detail = if notes.is_empty() {
        "all examined.".to_string()
    } else {
        format!("{}.", notes.join("; "))
    };
    crate::bin_cli::output::line(format!(
        "Formatted-output drift check: {} file(s) compared via a real `poly fmt` pass, {} matched the \
         render with no formatter involved, {detail}",
        stats.compared, stats.matched_render_exactly
    ));
}

#[cfg(test)]
mod tests;
