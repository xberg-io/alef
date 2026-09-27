//! Informational detection of create-once scaffold files whose on-disk content predates a
//! fix to the template that produced them.
//!
//! `generated_header: false` scaffold files (a zig `build.zig`, a kotlin test seed, a
//! params struct) are written once by `alef scaffold` and are user-owned from then on --
//! see [`super::scaffold::write_scaffold_files_report`]'s ownership guard for why they can
//! never be silently rewritten. That is by design: the consumer may have hand-edited the
//! file, and clobbering that edit would be a worse bug than any template defect. But it
//! also means a real template fix (a missing existence guard, a config-derived value, a
//! corrected struct shape) can never reach a consumer who already scaffolded the file --
//! `alef verify` had no way to even name that condition.
//!
//! # What this checks, and why not the obvious thing
//!
//! A create-once file is *expected* to differ from its template -- that is the entire
//! point of create-once. So "does the on-disk file differ from what alef would generate
//! today" is almost always true and reports nothing actionable; a report that fires on
//! every legitimately-edited file trains readers to skim past it, which is precisely the
//! failure mode `alef diff` hit with `gradle-wrapper.jar` (a binary comparison bug that
//! made one entry drift-report as changed on literally every run, forever, until the
//! comparison itself was fixed to compare bytes correctly).
//!
//! The only evidence available on the consumer's disk that discriminates "the template
//! changed since I scaffolded this" from "I edited this file" is the file's own git
//! history: **has the file ever been touched by a commit after the one that introduced
//! it?** If a file's entire commit history is exactly one commit, nothing in this
//! repository has changed the file since it first appeared here -- so if it *still*
//! differs from what today's template produces, the only remaining explanation is that
//! the template itself moved on. If the file has more than one commit, or was possibly
//! edited before that single commit, this module says nothing at all: the ambiguity is
//! resolved in favour of silence, not a guess.
//!
//! # False-positive / false-negative characteristics (stated honestly)
//!
//! * **False positive**: a consumer who hand-edits a freshly-scaffolded file *before*
//!   making its first commit (or who `git commit --amend`s / squash-merges an edit into
//!   that same commit) leaves exactly one commit in history despite the content being a
//!   real, intentional edit. Git offers no way to see "before that first commit" for a
//!   file that has always had exactly one commit, so this case cannot be distinguished
//!   from genuine template drift with this evidence. It is the class of false positive
//!   this detector accepts.
//! * **False negative** (the safer failure mode, and the deliberate default): any file
//!   with two or more commits is never reported, even when the second commit was a
//!   no-op formatting pass and the file was never meaningfully touched. Untracked files
//!   (zero commits -- not yet committed) are likewise never reported: there is no history
//!   to consult, so the honest answer is "unknown", not "drifted".
//! * A file whose on-disk content already matches what today's template produces is never
//!   reported, regardless of history -- there is nothing to fix.
//! * Binary create-once outputs (anything routed through
//!   [`super::binary::is_base64_binary_output`], e.g. `gradle-wrapper.jar`) are excluded
//!   entirely rather than risk repeating the exact permanently-noisy-entry bug this
//!   module exists to avoid for text files.
//!
//! # Why this is informational only
//!
//! This never fails `alef verify` and is never folded into
//! [`super::super::super::bin_cli::core_commands::verify_outcome::ensure_success`] (or any
//! other hard-fail gate): a create-once file differing from its template is the *expected*
//! steady state for a hand-maintained file, not a defect in the consumer's tree. Reporting
//! it as a hard failure would fail every consumer repo the day this check ships, blocking
//! releases that have nothing to do with the templates flagged. The remedy is also a
//! human decision (review the current template, decide whether to hand-port the fix), not
//! something `alef generate` can act on -- unlike stale bindings, there is no rerun that
//! closes this gap.

use super::normalization::{format_rust_content, normalize_content, normalize_whitespace};
use crate::cli::pipeline::format::poly_format_strict;
use crate::core::backend::GeneratedFile;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Create-once scaffold paths (relative to `base_dir`) whose content differs from what the
/// current template would produce, and whose git history offers no explanation other than
/// "the template changed since this file was scaffolded" -- see the module doc for exactly
/// what that means and what it does not.
///
/// `scaffold_files` is the freshly-regenerated scaffold set for one crate (as
/// [`super::scaffold::scaffold`] returns it) -- the same in-memory data `alef diff` already
/// computes, reused here rather than regenerated a second time.
pub fn find_create_once_template_drift(scaffold_files: &[GeneratedFile], base_dir: &Path) -> Vec<PathBuf> {
    scaffold_files
        .iter()
        .filter(|file| !file.generated_header)
        .filter(|file| !super::binary::is_base64_binary_output(&file.path))
        .filter(|file| differs_from_template(file, base_dir))
        .filter(|file| untouched_since_scaffold(base_dir, &file.path) == Some(true))
        .map(|file| file.path.clone())
        .collect()
}

/// Whether `file`'s on-disk content (under `base_dir`) differs from what the current
/// template would produce, once both sides go through the same normalization `alef diff`
/// uses (rustfmt for `.rs`, then trailing-whitespace/blank-line normalization) so
/// formatter-only noise is never mistaken for drift. A file that is not on disk at all is
/// not "drifted" -- that is a missing-file condition a different check already covers.
///
/// Rust is fully normalized right here via `rustfmt`, the one formatter alef always
/// requires. Every other create-once language (Ruby, Zig, Kotlin, ...) is instead run
/// through `poly fmt` -- see [`non_rust_content_still_differs_after_formatting`] -- because
/// `alef all`'s whole-tree pass already reformats those languages through poly after
/// scaffolding (`cli::pipeline::format::converge_full_regen`), and comparing raw generated
/// bytes against a poly-reformatted on-disk file mistook that reformatting for template
/// drift (RuboCop's one-line-array expansion on a scaffolded Ruby spec was the reported
/// instance, but the condition is generic to every language poly formats). ~keep
fn differs_from_template(file: &GeneratedFile, base_dir: &Path) -> bool {
    let full_path = base_dir.join(&file.path);
    let Ok(existing) = std::fs::read_to_string(&full_path) else {
        return false;
    };
    let is_rust = file.path.extension().is_some_and(|ext| ext == "rs");
    let generated = normalize_content(base_dir, &file.path, &file.content);
    let on_disk = if is_rust {
        format_rust_content(base_dir, &full_path, &existing)
    } else {
        existing
    };
    if normalize_whitespace(&on_disk) == normalize_whitespace(&generated) {
        return false;
    }
    if is_rust {
        return true;
    }
    non_rust_content_still_differs_after_formatting(&file.path, base_dir, &generated, &on_disk)
}

/// For a non-Rust create-once file whose raw content already differs from its template:
/// reformat BOTH sides with `poly fmt --fix` before concluding drift, so a language
/// formatter poly wraps (RuboCop, gofmt, ...) reshaping generated output the same way it
/// reshaped the on-disk copy is never mistaken for a real template change.
///
/// `false` -- no drift reported -- when `poly` cannot confirm the difference (not on PATH,
/// or the format attempt fails on either side): running a language's real formatter here can
/// be slow or require a toolchain a given host may lack, and a false negative (staying
/// silent about a genuine template fix) is far cheaper than reporting the formatter's own
/// output as drift. Same policy as the git-history ambiguity this module already resolves in
/// favour of silence, applied to a second source of ambiguity. ~keep
fn non_rust_content_still_differs_after_formatting(
    relative_path: &Path,
    base_dir: &Path,
    generated: &str,
    on_disk: &str,
) -> bool {
    let (Some(formatted_generated), Some(formatted_on_disk)) = (
        normalize_with_poly(relative_path, base_dir, generated),
        normalize_with_poly(relative_path, base_dir, on_disk),
    ) else {
        return false;
    };
    normalize_whitespace(&formatted_on_disk) != normalize_whitespace(&formatted_generated)
}

/// Reformat `content` with `poly fmt --fix`, or `None` if `poly` cannot run or the format
/// attempt fails. Delegates the actual invocation (and its exit-code/stderr success policy)
/// to [`poly_format_strict`] rather than re-deriving it, so this never disagrees with the
/// exact tool `alef all` itself calls -- including the argument shape: every other caller of
/// `poly_format_strict` hands it directories, never a lone file, so this does too (a fresh
/// temp directory under `base_dir`, holding one file named like the real one) rather than
/// being the one caller that finds out the CLI does not accept a bare file path.
///
/// The temp directory is created under `base_dir`, not the system temp root, so
/// `poly_format_strict`'s `config_start` (`base_dir`) resolves the consumer's actual
/// `poly.toml` exactly the way a real formatting pass would; the file keeps the real file's
/// name so poly's per-extension engine selection sees what it would for the real file.
fn normalize_with_poly(relative_path: &Path, base_dir: &Path, content: &str) -> Option<String> {
    let file_name = relative_path.file_name()?;
    let temp_dir = tempfile::Builder::new()
        .prefix("alef-drift-check-")
        .tempdir_in(base_dir)
        .ok()?;
    let temp_path = temp_dir.path().join(file_name);
    std::fs::write(&temp_path, content).ok()?;
    poly_format_strict(&[temp_dir.path().to_path_buf()], base_dir).ok()?;
    std::fs::read_to_string(&temp_path).ok()
}

/// `Some(true)`: `relative`'s entire git history under `base_dir` is exactly one commit --
/// nothing has touched the file since whatever commit introduced it, so a difference from
/// today's template can only be explained by the template having moved on.
///
/// `Some(false)`: zero commits (untracked -- no history to consult) or two-or-more commits
/// (possibly edited since scaffolding) -- ambiguous either way, so callers must not report.
///
/// `None`: git could not answer at all (no `git` binary, not a repository). Also must not
/// report -- an export tarball or container without git must never manufacture a claim it
/// has no evidence for.
fn untouched_since_scaffold(base_dir: &Path, relative: &Path) -> Option<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["log", "--follow", "--format=%H", "--"])
        .arg(relative)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit_count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Some(commit_count == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{git_commit_all, git_init as init_git_repo};

    fn seed_file(base_dir: &Path, relative: &str, content: &str) -> PathBuf {
        let full = base_dir.join(relative);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&full, content).expect("write seed");
        PathBuf::from(relative)
    }

    fn create_once_file(path: PathBuf, content: &str) -> GeneratedFile {
        GeneratedFile {
            path,
            content: content.to_string(),
            generated_header: false,
        }
    }

    /// The main "quiet by default" case: a file whose on-disk content already matches
    /// what the current template produces is never reported, no matter what its git
    /// history looks like -- there is nothing to fix. ~keep
    #[test]
    fn a_create_once_file_matching_the_current_template_is_silent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        let files = vec![create_once_file(relative, "const std = @import(\"std\");\n")];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "unchanged template must never be reported"
        );
    }

    /// The behavior this module exists to add: a file scaffolded once, never touched
    /// again (exactly one commit), that now differs from a template that has since been
    /// fixed -- this is exactly the zig/kotlin/kotlin_android situation the drift report
    /// was built for. `.zig` is a non-Rust extension, so this exercises
    /// [`non_rust_content_still_differs_after_formatting`] too (poly has no Zig engine, so
    /// its own formatting pass is a no-op here either way -- but confirming that still
    /// requires `poly` itself to be reachable; without it the check stays silent by design,
    /// see that function's doc). ~keep
    #[test]
    fn a_create_once_file_untouched_since_scaffold_and_differing_from_template_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        // The template has since gained an existence guard the scaffolded file predates.
        let files = vec![create_once_file(
            relative.clone(),
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        if crate::cli::pipeline::format::is_tool_available("poly") {
            assert_eq!(
                drift,
                vec![relative],
                "an untouched file that predates a template fix must be reported"
            );
        } else {
            assert_eq!(
                drift,
                Vec::<PathBuf>::new(),
                "without poly on PATH the check cannot confirm the difference isn't formatting-only, so \
                 it must stay silent (false negative preferred over false positive)"
            );
        }
    }

    /// The false-positive boundary this module deliberately declines to cross: once a
    /// consumer has committed a second time, the diff might be their own edit, so this
    /// must stay silent rather than guess. Pinned explicitly as a known limitation, not
    /// an oversight. ~keep
    #[test]
    fn a_create_once_file_edited_after_scaffolding_is_not_reported_as_template_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");
        std::fs::write(
            dir.path().join("build.zig"),
            "const std = @import(\"std\");\n// hand edit\n",
        )
        .expect("hand edit");
        git_commit_all(dir.path(), "consumer edit");

        let files = vec![create_once_file(
            relative,
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "a file with more than one commit must not be reported -- the second commit might be a \
             legitimate consumer edit, and this detector favors silence over a guess"
        );
    }

    /// An uncommitted (never-committed) create-once file has no history to consult at
    /// all, so it must read the same as "unknown", not "drifted". ~keep
    #[test]
    fn an_uncommitted_create_once_file_is_not_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        // Deliberately never committed.

        let files = vec![create_once_file(
            relative,
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "a file with zero commits has no history to prove drift from, so it must stay silent"
        );
    }

    /// `generated_header: true` files are on the marker/hash rail, not this one -- they
    /// are excluded outright regardless of content or history. ~keep
    #[test]
    fn a_marker_rail_file_is_never_considered_for_create_once_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "src/lib.rs", "// stale\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        let files = vec![GeneratedFile {
            path: relative,
            content: "// fresh\n".to_string(),
            generated_header: true,
        }];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "marker-rail files are never in scope for this check"
        );
    }

    /// The general non-Rust condition Finding B's Ruby report is one instance of. A
    /// scaffolded Python file that `alef all`'s whole-tree `poly fmt --fix` pass has already
    /// reformatted (`x=1` -> `x = 1\n`, the same transformation `format_generated_python_...`
    /// in `cli::pipeline::format::tests` already proves poly performs) must not be reported
    /// as template drift just because the raw, never-formatted template output looks
    /// different byte-for-byte. Passes identically whether or not `poly` is installed here:
    /// with poly, the difference is confirmed as formatting-only; without it, the check
    /// stays silent by design (see [`non_rust_content_still_differs_after_formatting`]). ~keep
    #[test]
    fn a_non_rust_create_once_file_reformatted_only_by_poly_is_not_reported_as_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        // On-disk content is what alef's own formatting pass already produced from the raw
        // template output below -- not a consumer edit.
        let relative = seed_file(dir.path(), "packages/python/scaffolded.py", "x = 1\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        let files = vec![create_once_file(relative, "x=1")];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "a difference poly's own formatter accounts for must never be reported as template drift"
        );
    }

    /// Control for the same condition: a genuine content change on a non-Rust create-once
    /// file must still be reported once poly can confirm the difference is not merely
    /// formatting. Guards against the Finding B fix regressing into "never report non-Rust
    /// drift". Requires `poly` on PATH to prove the positive case -- without it the module's
    /// documented policy is to stay silent rather than guess (a false negative), which is a
    /// different, already-covered code path, not this test's subject. ~keep
    #[test]
    fn a_non_rust_create_once_file_with_a_genuine_content_change_is_still_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "packages/python/scaffolded.py", "x = 1\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        // The template now assigns a different value -- a real content change, not a
        // reformatting-only difference.
        let files = vec![create_once_file(relative.clone(), "x=2")];
        let drift = find_create_once_template_drift(&files, dir.path());

        if crate::cli::pipeline::format::is_tool_available("poly") {
            assert_eq!(
                drift,
                vec![relative],
                "a genuine content change must still be reported once poly confirms it isn't just formatting"
            );
        } else {
            assert_eq!(
                drift,
                Vec::<PathBuf>::new(),
                "without poly on PATH the check cannot confirm the difference isn't formatting-only, so \
                 it must stay silent -- a false negative is accepted in exchange for never reporting a \
                 formatter's own output as drift"
            );
        }
    }
}
