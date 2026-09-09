//! The two things a caller must do around a formatting pass, because `poly` will not format a
//! file that alef has already stamped.
//!
//! `poly` skips any file whose leading lines carry a well-formed `<tool>:hash:<hex>` line, under
//! `--fix` **and** under `--check`. Measured against the `poly` binary this repository builds
//! against (0.21.x): a file carrying `alef:hash:<64 hex>` is reported as
//! `skipped <path>: hash-stamped generated file (pass --fix-generated to format)`, and `--check`
//! exits 0 on a tree containing nothing else. That skip is correct for third-party invocations --
//! a formatter must not fight a generator, and rewriting generated output can silence the very
//! diagnostic that was the evidence of a generator bug. It is a lock-out for alef's *own*
//! formatting pass, which is a stage of generation and re-stamps everything it formatted moments
//! later.
//!
//! Two distinct failures follow from it, and they need different remedies:
//!
//! 1. **This run stamps too early.** A phase that stamps its output before the format pass makes
//!    that pass a no-op for its files, which then ship in whatever shape the generator emitted.
//!    Remedy: stamp once, after formatting -- which is what [`super::super::finalize_hashes`]'
//!    own doc already required. [`unstamp_before_formatting`] also covers this case, so the
//!    ordering discipline is defence in depth rather than the load-bearing fix.
//! 2. **A previous run stamped too early.** Those files' generated bodies have not changed since,
//!    so `write_files_report` (which compares hash-stripped bodies) does not rewrite them, they
//!    keep the stamp they were given, and poly skips them again -- on every future run, forever.
//!    Reordering cannot reach them. Measured on a neutral eight-language fixture: an alef that
//!    only reordered its stamp canonicalised 6 of the 21 affected files and left the other 15
//!    exactly as it found them. [`unstamp_before_formatting`] is what reaches them, and
//!    [`generated_tree_needs_formatting`] is what makes a run whose writers changed nothing still
//!    notice they are there.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Strip the `alef:hash:` line from every path in `paths` that still carries one, so the
/// formatting pass that follows can actually see those files. Returns how many were rewritten.
///
/// See the module doc for why a stamped file is invisible to the formatter, and for the two
/// failure modes this addresses.
///
/// **The caller must re-stamp at least `paths` after formatting** (`finalize_hashes*`), or those
/// files ship carrying no hash line at all and `alef verify` cannot speak for them. Stripping and
/// re-stamping an already-canonical file is a no-op in content terms: the recomputed hash covers
/// the same body it did before, so the file's final bytes are unchanged.
pub fn unstamp_before_formatting(paths: &HashSet<PathBuf>) -> usize {
    use rayon::prelude::*;

    let unstamped = std::sync::atomic::AtomicUsize::new(0);
    paths.par_iter().for_each(|path| {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        if !crate::core::hash::content_has_alef_marker(&content) {
            return;
        }
        let stripped = crate::core::hash::strip_hash_line(&content);
        if stripped == content {
            return;
        }
        match super::super::atomic_write(path, stripped.as_bytes()) {
            Ok(()) => {
                unstamped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // Best-effort, like every other step in this module's neighbourhood: a path that
            // cannot be rewritten keeps its stamp and is skipped by the formatter, which is the
            // behaviour that shipped before this function existed. Failing the run here would
            // turn a read-only file into a hard generation failure. ~keep
            Err(error) => warn!("could not unstamp {} before formatting: {error:#}", path.display()),
        }
    });
    unstamped.into_inner()
}

/// Whether anything under `base_dir` would be rewritten by a formatting pass, **counting files
/// that already carry an `alef:hash:` line**.
///
/// The gate `alef all` puts in front of its format pass is "did this run write anything", which
/// is the right question for freshly emitted output and the wrong one for failure mode 2 in the
/// module doc: nothing was written, and the tree is still non-canonical. Asking poly directly is
/// the only way to tell that tree apart from a settled one.
///
/// `--fix-generated` is what makes the answer meaningful -- without it poly declines to inspect a
/// stamped file and reports the tree clean. Nothing is written either way; this is `--check`.
/// Only the exit status is read, never poly's output text, so the gate does not depend on the
/// spelling of poly's report. Best-effort: a missing or failing `poly` answers "no", leaving the
/// caller's own change-detection as the sole trigger, which is the behaviour that shipped before
/// this existed. ~keep
pub(crate) fn generated_tree_needs_formatting(base_dir: &Path) -> bool {
    if !super::is_tool_available("poly") {
        return false;
    }
    let path_str = base_dir.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec![
        "fmt".to_owned(),
        "--check".to_owned(),
        "--fix-generated".to_owned(),
        path_str,
    ];
    super::push_poly_format_excludes(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    super::run_formatter("poly", &arg_refs, base_dir).is_err()
}

#[cfg(test)]
#[path = "stamp_gate_tests.rs"]
mod tests;
