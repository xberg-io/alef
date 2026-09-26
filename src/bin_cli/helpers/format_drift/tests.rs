//! Tests for [`super::real_formatter_drift`]/[`super::real_formatter_drift_with`] (alef#436's
//! second half) -- split into its own module for the same reason `helpers/drift_tests.rs` splits
//! out of `helpers/tests.rs`: this repository's 1,000-line file cap, and this concern (comparing
//! a non-`.rs`/`.md` render against disk through a real formatter) is self-contained. ~keep

use super::*;

fn candidate(full_path: &std::path::Path, disk_content: &str, rendered_content: &str) -> RealFormatCandidate {
    RealFormatCandidate {
        full_path: full_path.to_path_buf(),
        disk_content: disk_content.to_string(),
        rendered_content: rendered_content.to_string(),
    }
}

/// THE LOUD SKIP (see the module doc's `prove-the-check-fired` reference): with no seam into the
/// real `is_tool_available`, this must be provable without depending on whether the host running
/// the suite happens to have `poly` installed -- so it drives the injectable
/// `real_formatter_drift_with` directly, the same seam `format_generated_reporting_with`'s own
/// strict tests use for the identical reason.
#[test]
fn skips_and_counts_every_candidate_when_poly_is_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let candidates = vec![
        candidate(&dir.path().join("a.toml"), "[a]\nx = 1\n", "[a]\nx=1\n"),
        candidate(&dir.path().join("b.py"), "x = 1\n", "x=1\n"),
    ];

    let (drifted, stats) = real_formatter_drift_with(candidates, dir.path(), &|_tool| false);

    assert!(
        drifted.is_empty(),
        "a skipped comparison must never manufacture a drift finding: {drifted:?}"
    );
    assert_eq!(
        stats,
        FormatDriftStats {
            compared: 0,
            skipped_missing_formatter: 2,
            skipped_staging_error: 0,
        },
        "every candidate must be counted as skipped, never silently dropped, when poly is absent"
    );
}

/// An empty candidate list is a trivial 0/0 -- there was nothing to compare, and that must read
/// as "nothing to compare", not as either an examined-clean or a skipped tree.
#[test]
fn is_a_trivial_no_op_on_an_empty_candidate_list() {
    let dir = tempfile::tempdir().expect("tempdir");

    let (drifted, stats) = real_formatter_drift_with(Vec::new(), dir.path(), &|_tool| true);

    assert!(drifted.is_empty());
    assert_eq!(stats, FormatDriftStats::default());
}

/// THE CORE MECHANISM, against the real `poly` binary: a TOML file whose rendered content
/// genuinely differs from disk, even after real formatting, must be reported drifted --
/// something `normalize_content` could never catch for TOML (see the module doc for why this
/// runs the real formatter instead of predicting its output). Skips itself when `poly` is not on
/// the host running the suite, the same guard this codebase's own `alef fmt`/`mix format`
/// integration tests use (see `cli::pipeline::commands::lint::tests`) for a real-subprocess
/// assertion with no injectable seam underneath it.
#[test]
fn catches_a_toml_file_whose_rendered_value_genuinely_differs_from_disk() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let real_path = dir.path().join("pyproject.toml");
    std::fs::write(&real_path, "[project]\nname = \"old\"\n").unwrap();
    let candidates = vec![candidate(
        &real_path,
        "[project]\nname = \"old\"\n",
        "[project]\nname=\"new\"\n",
    )];

    let (drifted, stats) = real_formatter_drift(candidates, dir.path());

    assert_eq!(drifted, vec![real_path.display().to_string()]);
    assert_eq!(
        stats,
        FormatDriftStats {
            compared: 1,
            skipped_missing_formatter: 0,
            skipped_staging_error: 0,
        }
    );
}

/// THE CONTROL for the mechanism above: once poly's real formatting of the rendered content
/// converges to exactly what is already on disk, nothing must be reported -- otherwise every
/// up-to-date non-`.rs`/`.md` file would fail `alef verify` forever. Builds the disk fixture by
/// running the SAME rendered content through the SAME real formatter first (rather than
/// hand-formatting a fixture), so this stays correct regardless of which poly version or TOML
/// formatting opinion the host running the suite has installed.
#[test]
fn is_silent_once_poly_fmt_converges_the_render_to_match_disk() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let real_path = dir.path().join("pyproject.toml");
    let rendered = "[project]\nname=\"same\"\n";
    let (_drifted, _stats) = real_formatter_drift(
        vec![candidate(&real_path, "placeholder -- overwritten below", rendered)],
        dir.path(),
    );
    // The call above formats a temp copy, not `real_path` itself (it never exists on disk yet in
    // this test) -- reconstruct the converged disk fixture the same way `alef all` would, by
    // running the identical rendered bytes through the real formatter and writing the result.
    let formatted_once = format_once_for_test(rendered, dir.path());
    std::fs::write(&real_path, &formatted_once).unwrap();

    let (drifted, stats) = real_formatter_drift(vec![candidate(&real_path, &formatted_once, rendered)], dir.path());

    assert!(
        drifted.is_empty(),
        "an already-converged file must not be reported drifted: {drifted:?}"
    );
    assert_eq!(stats.compared, 1);
}

/// Helper for the control test above: format `content` once through the real `poly fmt --fix`,
/// the same way [`real_formatter_drift`] itself does internally, so the test's disk fixture is
/// built from the identical mechanism under test rather than a hand-written guess at poly's
/// canonical TOML layout.
fn format_once_for_test(content: &str, base_dir: &std::path::Path) -> String {
    let scratch = base_dir.join("scratch.toml");
    std::fs::write(&scratch, content).unwrap();
    crate::cli::pipeline::poly_format_strict(std::slice::from_ref(&scratch), base_dir).expect("poly fmt must succeed");
    let formatted = std::fs::read_to_string(&scratch).unwrap();
    std::fs::remove_file(&scratch).ok();
    formatted
}

/// The sibling temp file [`real_formatter_drift`] stages must never survive the call -- a stray
/// `.alef-verify-drift-*` file left behind in a consumer's working tree would itself show up as
/// an untracked file on the next `git status`, which is precisely the kind of side effect a
/// read-only `alef verify` must not have.
#[test]
fn leaves_no_temp_file_behind_after_comparing() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let real_path = dir.path().join("pyproject.toml");
    std::fs::write(&real_path, "[project]\nname = \"old\"\n").unwrap();
    let candidates = vec![candidate(
        &real_path,
        "[project]\nname = \"old\"\n",
        "[project]\nname=\"new\"\n",
    )];

    let _ = real_formatter_drift(candidates, dir.path());

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name != "pyproject.toml")
        .collect();
    assert!(
        leftovers.is_empty(),
        "no temp file may survive a comparison call, found: {leftovers:?}"
    );
}
