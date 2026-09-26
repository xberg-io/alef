//! The alef#436 marked-file drift comparison ("does this already-marked, already-present file
//! still match what this run would produce") -- alef#436's second half, and the home for
//! [`drifted_marked_paths`], the two-tier comparison `bin_cli::helpers::find_missing_and_frozen_
//! generated_files` calls into.
//!
//! `.rs` and `.md` predict `alef all`'s final bytes in memory (`normalize_content`'s real
//! `rustfmt` pass, and a blank-line policy deliberately matched to poly's `rumdl`, respectively
//! -- see [`render_predicts_final_bytes`]'s doc for why those two, and only those two, are
//! trustworthy predictions today). Every other poly-formatted extension has no such prediction:
//! `poly fmt --fix` (or whichever native tool poly delegates to when present -- `rubocop`,
//! `php-cs-fixer`, `styler`, ...) can reshape a file's bytes after `write_files` computes its
//! `alef:hash:`, and `normalize_content` has no emulation of any of those engines. The issue
//! comment on alef#436 considered and rejected predicting them byte-for-byte: growing a
//! per-language emulation of an engine alef does not own is a maintenance trap that grows every
//! time poly adds, drops, or reconfigures an engine.
//!
//! Instead, this module runs the REAL `poly fmt --fix` over the freshly rendered bytes and
//! compares the result to disk -- exact rather than approximate, and immune to poly gaining or
//! losing an engine for a given extension: a language poly has no engine for today (or has
//! deliberately excluded, like `.ex`/`.exs`/`.cs` -- see `cli::pipeline::format`'s
//! `POLY_ELIXIR_EXCLUDE_GLOBS`/`POLY_CSHARP_EXCLUDE_GLOBS`) degrades to a plain content
//! comparison for free, because running `poly fmt --fix` on such a file is a no-op.
//!
//! # Why the temp files live beside the real file, not in a scratch directory
//!
//! poly resolves `poly.toml` (and each engine's own project markers -- `pyproject.toml`,
//! `tsconfig.json`, ...) by walking up from the target file's own directory. A scratch directory
//! elsewhere would need every one of those config files mirrored alongside it to format
//! identically to the real tree, which is exactly the "reproduce poly's behaviour" trap this
//! module exists to avoid. A same-directory sibling, sharing the real file's extension so
//! poly's per-extension engine dispatch treats it identically, resolves every one of those
//! configs exactly as the real file would.
//!
//! # Why one batched `poly fmt --fix` invocation, not one per file
//!
//! `poly` is a subprocess; spawning one per candidate file turns an `alef verify` run with a few
//! hundred generated non-Rust/Markdown files into a few hundred process spawns. Every sibling
//! temp file this module creates is handed to a single [`crate::cli::pipeline::poly_format_strict`]
//! invocation instead. ~keep

use std::path::{Path, PathBuf};

/// One file whose fast (`.rs`/`.md`) prediction does not apply, paired with its current disk
/// content and this run's freshly rendered content -- already through
/// [`crate::cli::commands::adopt::managed_outputs`]'s normalization/header pass, i.e. the bytes
/// `write_files` would put on disk BEFORE `alef all`'s post-write `poly fmt --fix` pass touches
/// them.
pub(super) struct RealFormatCandidate {
    pub(super) full_path: PathBuf,
    pub(super) disk_content: String,
    pub(super) rendered_content: String,
}

/// How many candidates [`real_formatter_drift`] actually compared through a real `poly fmt`
/// pass, versus how many it had to skip because `poly` is not installed on this machine.
///
/// Surfaced all the way to `alef verify`'s own report (`bin_cli::core_commands::verify`) so a
/// missing formatter is a loud, counted skip rather than a silent pass -- this repo's dominant
/// defect shape is a check that examines nothing and still reports clean (the
/// `prove-the-check-fired` rule). `skipped_missing_formatter` is never folded into `compared`:
/// the two answer different questions -- "did the check run" and "what did it find" -- and a
/// report that only stated the sum could not tell a clean, fully-examined tree from one this
/// call gave up on entirely. ~keep
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormatDriftStats {
    pub(crate) compared: usize,
    pub(crate) skipped_missing_formatter: usize,
    /// Candidates whose temp copy could not be written at all -- a read-only checkout, a
    /// permission-denied directory. Counted apart from `skipped_missing_formatter` because the
    /// remedy is different and a report that blamed a missing `poly` for a read-only tree would
    /// send the reader after the wrong thing. ~keep
    pub(crate) skipped_staging_error: usize,
}

/// Compare every `candidates` entry against disk through a real `poly fmt --fix` pass over a
/// same-directory temp copy of its rendered bytes -- see the module doc for why colocated and
/// why batched. Returns the drifted subset (as `full_path.display()` strings, matching every
/// other list in [`super::MissingAndFrozenFiles`]) alongside [`FormatDriftStats`].
pub(super) fn real_formatter_drift(
    candidates: Vec<RealFormatCandidate>,
    base_dir: &Path,
) -> (Vec<String>, FormatDriftStats) {
    real_formatter_drift_with(candidates, base_dir, &crate::cli::pipeline::is_tool_available)
}

/// Testable seam for [`real_formatter_drift`]: gates the up-front "is poly even installed"
/// decision through `is_available` instead of PATH, so the skip-and-count branch is provable
/// without depending on whether the host running the suite happens to have `poly`. There is
/// deliberately no seam for the formatting pass itself once this gate passes -- emulating
/// poly's own output is exactly the prediction this module exists to avoid, so the "poly ran and
/// found real drift" branch is proven against the real binary instead, skipping itself when it
/// is absent (see this module's own tests for that pattern). ~keep
fn real_formatter_drift_with(
    candidates: Vec<RealFormatCandidate>,
    base_dir: &Path,
    is_available: &dyn Fn(&str) -> bool,
) -> (Vec<String>, FormatDriftStats) {
    if candidates.is_empty() {
        return (Vec::new(), FormatDriftStats::default());
    }
    if !is_available("poly") {
        return (
            Vec::new(),
            FormatDriftStats {
                compared: 0,
                skipped_missing_formatter: candidates.len(),
                skipped_staging_error: 0,
            },
        );
    }

    let temp_files: Vec<Option<tempfile::NamedTempFile>> = candidates
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

    let temp_paths: Vec<PathBuf> = temp_files
        .iter()
        .filter_map(|temp_file| temp_file.as_ref().map(|file| file.path().to_path_buf()))
        .collect();
    if let Err(error) = crate::cli::pipeline::poly_format_strict(&temp_paths, base_dir) {
        tracing::warn!("poly fmt over the drift-check temp copies failed (non-fatal): {error:#}");
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
            skipped_missing_formatter: 0,
            skipped_staging_error: staging_errors,
        },
    )
}

/// Write `content` into a securely-named temp file beside `real_path`, sharing its extension so
/// poly's per-extension engine dispatch treats it identically -- see the module doc for why this
/// must be a same-directory sibling rather than a scratch directory.
///
/// `NamedTempFile` is deliberate: it deletes itself on drop, so a panic, an early return, or an
/// interrupted `alef verify` run cannot leave a stray `.alef-verify-drift-*` file behind in a
/// consumer's working tree the way a manually paired write+remove could. ~keep
fn write_sibling_temp_file(real_path: &Path, content: &str) -> std::io::Result<tempfile::NamedTempFile> {
    let parent = real_path.parent().unwrap_or_else(|| Path::new("."));
    let suffix = real_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let temp_file = tempfile::Builder::new()
        .prefix(".alef-verify-drift-")
        .suffix(&suffix)
        .tempfile_in(parent)?;
    std::fs::write(temp_file.path(), content)?;
    Ok(temp_file)
}

/// Absolute paths of every file in `files` that already exists on disk, already carries alef's
/// own provenance marker, and yet no longer matches the bytes this run's fresh render would
/// produce.
///
/// THE GAP this closes (alef#436): a public type gaining a field left `alef docs`' rendered API
/// reference pages stale while `alef verify` stayed green, because every check `alef verify` ran
/// was structurally blind to it:
///
/// - [`super::stale_among`]'s per-file `alef:hash:` check recomputes a hash of the file's OWN
///   current bytes and compares it to the hash embedded in those same bytes — it catches a
///   hand-edit, never a file that is internally self-consistent but stale relative to a fresh
///   render.
/// - The crate-scoped `inputs_hash` record (`cache::generation_record`,
///   `bin_cli::core_commands::verify::run`) is written only by `alef generate`/`alef all`, never
///   by `alef docs` — so a docs-only regeneration gap never re-stamps it, but neither does
///   leaving docs untouched ever unstamp it: `alef generate` re-stamps the very baseline this
///   check would need to catch the drift, from a run that never touched a single docs page.
/// - `missing_managed_paths`/`frozen_managed_paths` only ever ask "does this path exist" and
///   "does it carry a marker" — never "do its bytes match", so a marked, present, stale file is
///   invisible to both.
///
/// Two-tier comparison, not a single one scoped to `.rs`/`.md`: an earlier version of this check
/// used ONLY the fast in-memory prediction below ([`render_predicts_final_bytes`]) and, when a
/// prior attempt widened that prediction to every extension, broke
/// `verify_reports_an_incomplete_generation_run_instead_of_ordinary_staleness` and two sibling
/// tests, all three asserting the established, pre-existing contract "a completed `alef all` run
/// leaves a tree `alef verify` reports clean." The false positives were `pyproject.toml`,
/// `poly.toml` (TOML, via poly's `taplo`) and a generated `.py` file (via poly's `ruff`): `alef
/// all`'s "Formatting generated files..." stage runs `poly fmt --fix`
/// (`cli::pipeline::format::poly_format`) over the paths it just wrote, AFTER
/// `collect_managed_surface`'s in-memory render is computed, and that external reformatting pass
/// changes bytes `normalize_content` never claims to predict. `normalize_content` only reproduces
/// two of poly's engines — `rustfmt` for `.rs` (`format_rust_content`) and a blank-line policy for
/// `.md` deliberately matched to poly's `rumdl` — so those two extensions are the only ones where
/// an in-memory render is a faithful PREDICTION of the bytes `alef all` actually leaves on disk.
///
/// Rather than growing that prediction language by language (the framing alef#436's own issue
/// comment considered and rejected -- a per-engine emulation of a formatter alef does not own,
/// and does not control the evolution of, is a maintenance trap that grows every time poly adds,
/// drops, or reconfigures an engine), every other marked, existing file is instead compared
/// through [`real_formatter_drift`]: the freshly rendered content is run through a REAL `poly fmt
/// --fix` pass and the result is compared to disk. This is exact rather than approximate, and it
/// degrades safely for an extension poly has no engine for (or deliberately excludes, like
/// `.ex`/`.exs`/`.cs`): running `poly fmt --fix` on such a file is a no-op, so the comparison
/// reduces to plain content equality -- the same answer `normalize_content` would have given, for
/// free, with no per-extension special-casing required here. See this module's own doc for the
/// full mechanism, including why the temp copies it stages must be same-directory siblings of the
/// real file rather than living in a scratch directory.
///
/// A missing `poly` binary is not silently treated as "no drift": [`FormatDriftStats`] counts
/// every candidate this call had to skip instead, and `alef verify`
/// (`bin_cli::core_commands::verify::run`) reports that count on every run -- see the
/// `prove-the-check-fired` rule for why a check that examined nothing must never look like one
/// that passed.
///
/// Deliberately excludes every file `frozen_managed_paths` would already report: an unmarked
/// file is that check's condition, not this one's (this function's own `content_has_alef_marker`
/// guard below is what enforces the split), and reporting the same withheld write under two
/// headings would describe it as two different findings with two different remedies. Also
/// excludes [`crate::cli::pipeline::is_base64_binary_output`] paths: poly fmt has no business
/// reformatting a binary payload, and no such path was ever in scope before this check existed
/// either.
///
/// Reuses [`crate::cli::commands::adopt::managed_outputs`] and
/// [`crate::cli::pipeline::matches_alef_output`] — the exact pairing `frozen_managed_paths`
/// already uses for its own `drifted` field — rather than a bespoke comparison, specifically
/// because of the caveat that pairing already solves: a self-marking backend (pyo3's `lib.rs`,
/// `docs::render`'s HTML-commented pages) or a `generated_header: true` file both legitimately
/// differ from a naive render by exactly the provenance header/hash line, and `matches_alef_output`
/// is the one place that discounts it. A hand-rolled byte comparison here would report permanent
/// drift for every one of those files. ~keep
pub(super) fn drifted_marked_paths(
    files: &[crate::core::backend::GeneratedFile],
    base_dir: &Path,
) -> (Vec<String>, FormatDriftStats) {
    let mut drifted = Vec::new();
    let mut real_format_candidates = Vec::new();
    for file in files {
        let full_path = base_dir.join(&file.path);
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
        let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(file), base_dir);
        let Some(output) = rendered.into_iter().next() else {
            continue;
        };
        if render_predicts_final_bytes(&full_path) {
            let up_to_date = crate::cli::pipeline::matches_alef_output(&full_path, &existing, &output.content);
            if !up_to_date {
                drifted.push(full_path.display().to_string());
            }
            continue;
        }
        real_format_candidates.push(RealFormatCandidate {
            full_path,
            disk_content: existing,
            rendered_content: output.content,
        });
    }
    let (real_drifted, stats) = real_formatter_drift(real_format_candidates, base_dir);
    drifted.extend(real_drifted);
    (drifted, stats)
}

/// Whether [`crate::cli::pipeline::normalize_content`]'s in-memory normalization is a faithful
/// PREDICTION of the bytes `alef generate`/`alef all` actually leave on disk at `path` -- the fast
/// path [`drifted_marked_paths`] takes for `.rs`/`.md`. Every other extension instead goes
/// through [`real_formatter_drift`], which runs the real formatter rather than predicting it --
/// see this module's own doc and [`drifted_marked_paths`]'s doc for the measured evidence that
/// only these two extensions can be predicted honestly today. ~keep
fn render_predicts_final_bytes(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "rs" || extension == "md")
}

/// Print `alef verify`'s formatted-output drift coverage line.
///
/// Printed unconditionally, on every run including a clean one, for the same reason
/// `VerifyCoverage` reports scan coverage unconditionally: a check that examined nothing must
/// never render identically to one that found a clean tree (the `prove-the-check-fired` rule).
/// Neither skip is silent -- each one means this run could not tell whether that many
/// non-`.rs`/`.md` generated files are stale, which is a real gap in what the run proved, not a
/// "nothing to report" outcome. The two skips are reported apart because their remedies differ:
/// install `poly`, versus make the checkout writable. ~keep
pub(crate) fn report_format_drift_coverage(compared: usize, skipped_missing_poly: usize, staging_errors: usize) {
    let detail = if skipped_missing_poly > 0 {
        tracing::warn!(
            "poly is not installed; {skipped_missing_poly} generated file(s) could not be checked \
             for formatted-output drift (install poly to close this gap)"
        );
        format!(
            "{skipped_missing_poly} file(s) SKIPPED because poly is not installed (install poly to close this gap -- see poly-lint-format)."
        )
    } else if staging_errors > 0 {
        tracing::warn!(
            "{staging_errors} generated file(s) could not be checked for formatted-output drift: \
             their temp copy could not be written (read-only checkout?)"
        );
        format!("{staging_errors} file(s) SKIPPED because a temp copy could not be written beside them.")
    } else {
        "all examined.".to_string()
    };
    crate::bin_cli::output::line(format!(
        "Formatted-output drift check: {compared} file(s) compared via a real `poly fmt` pass, {detail}"
    ));
}

#[cfg(test)]
mod tests;
