//! Tests for [`super::drifted_marked_paths`] (alef#436) -- split out of `helpers/tests.rs`
//! rather than added to it: that file sits at this repository's 1,000-line cap, and this
//! concern (deciding which already-marked, present files are drifted from a fresh render) is
//! self-contained enough to own its own module, mirroring how `helpers/frozen/tests.rs` already
//! splits the neighbouring frozen-file concern out for the same reason. ~keep

use super::*;

fn gen_file(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: true,
    }
}

fn gen_file_unheadered(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: false,
    }
}

/// alef#436's core mechanism: a marked, present file whose bytes are internally
/// self-consistent (so the per-file `alef:hash:` walk sees nothing wrong) but no
/// longer match what a fresh render would produce must still be reported.
///
/// `.rs`, not `.md`: `render_predicts_final_bytes` only clears `.rs`/`.md` for this check (see
/// its doc for the measured reason), and a `.md` `GeneratedFile` in practice is always
/// self-marking -- `provenance_header_for_path` returns `None` for it, since
/// `docs::render`/`generate_docs_stage_impl` bake their own HTML-commented header straight into
/// `content` rather than relying on `ensure_generated_header` -- so `.md` cannot exercise the
/// `generated_header: true`-inserts-a-comment-header shape this test is about; the self-marking
/// shape gets its own pair of tests below. ~keep
#[test]
fn drifted_marked_paths_reports_a_marked_file_whose_body_no_longer_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old_file = gen_file("lib.rs", "pub fn greet() -> &'static str { \"old\" }\n");
    let old_rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&old_file), dir.path());
    std::fs::write(dir.path().join("lib.rs"), &old_rendered[0].content).unwrap();
    let files = vec![gen_file("lib.rs", "pub fn greet() -> &'static str { \"new\" }\n")];

    let (drifted, _stats) = drifted_marked_paths(&files, dir.path());

    assert_eq!(
        drifted,
        vec![dir.path().join("lib.rs").display().to_string()],
        "a marked file whose source changed since it was last written must be reported drifted"
    );
}

/// THE CONTROL. A marked file whose disk content already matches a fresh render must not be
/// reported — otherwise every up-to-date tree would fail `alef verify` forever.
///
/// Builds the on-disk fixture through the exact same `managed_outputs` call
/// `drifted_marked_paths` itself uses, rather than hand-assembling the header/body layout, so
/// this stays correct regardless of whether `rustfmt` is on `$PATH` or which version it is: both
/// the fixture and the function under test go through the identical normalization, so there is
/// nothing for a formatter difference to disagree about. ~keep
#[test]
fn drifted_marked_paths_is_silent_once_the_marked_file_matches_the_fresh_render() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = gen_file("lib.rs", "pub fn greet() -> &'static str { \"new\" }\n");
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&file), dir.path());
    std::fs::write(dir.path().join("lib.rs"), &rendered[0].content).unwrap();
    let files = vec![file];

    let (drifted, _stats) = drifted_marked_paths(&files, dir.path());
    assert!(drifted.is_empty());
}

/// An unmarked pre-existing file is `frozen_managed_paths`'s condition, not this one's --
/// reporting it here too would describe the same withheld write under two different headings.
#[test]
fn drifted_marked_paths_ignores_a_file_that_carries_no_marker_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("lib.rs"), "pub fn greet() {}\n").unwrap();
    let files = vec![gen_file("lib.rs", "pub fn greet() { /* changed */ }\n")];

    let (drifted, _stats) = drifted_marked_paths(&files, dir.path());
    assert!(drifted.is_empty());
}

/// THE alef#436 SECOND-HALF FIX: a marked TOML file whose content genuinely changed must be
/// reported drifted, even though `normalize_content` has no TOML emulation at all --
/// `drifted_marked_paths` no longer relies on that prediction for a non-`.rs`/`.md` extension. It
/// instead runs a REAL `poly fmt --fix` pass over the rendered bytes (see
/// `format_drift::real_formatter_drift`'s module doc) and compares the result to disk, which is
/// exact rather than approximate and needs no per-language emulation to get right. Skips itself
/// when `poly` is not on the host running the suite -- the branch for that case is proven
/// host-independently via `format_drift::tests::skips_and_counts_every_candidate_when_poly_is_
/// unavailable`'s injectable seam instead.
#[test]
fn drifted_marked_paths_now_catches_a_toml_file_via_the_real_poly_fmt_pass() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    std::fs::write(
        dir.path().join("pyproject.toml"),
        format!("{header}\n[project]\nname = \"old\"\n"),
    )
    .unwrap();
    let files = vec![gen_file("pyproject.toml", "[project]\nname=\"new\"\n")];

    let (drifted, stats) = drifted_marked_paths(&files, dir.path());

    assert_eq!(
        drifted,
        vec![dir.path().join("pyproject.toml").display().to_string()],
        "a marked TOML file whose source changed must now be reported drifted via the real \
         formatter pass, not silently predicted-and-missed"
    );
    assert_eq!(
        stats.compared, 1,
        "the real-formatter pass must have actually examined this file"
    );
    assert_eq!(stats.skipped_missing_formatter, 0);
}

/// THE CONTROL for the fix above: a marked TOML file that is already up to date -- even after a
/// real `poly fmt --fix` pass reformats the fresh render -- must not be reported. Without this,
/// every already-formatted TOML file in a clean tree would fail `alef verify` forever, which is
/// exactly the regression `verify_reports_an_incomplete_generation_run_instead_of_ordinary_
/// staleness` (and two siblings) caught the first time this check ran over every extension using
/// a byte-for-byte prediction instead of the real formatter.
#[test]
fn drifted_marked_paths_is_silent_once_a_toml_file_matches_the_real_formatters_output() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let rendered_raw = "[project]\nname=\"same\"\n";
    // Build the on-disk fixture by running the SAME rendered content through the SAME real
    // `poly fmt` pass `drifted_marked_paths` itself uses, rather than hand-formatting a fixture,
    // so this stays correct regardless of which poly version or TOML formatting opinion the host
    // running the suite has installed.
    let scratch = dir.path().join("scratch.toml");
    std::fs::write(&scratch, format!("{header}\n{rendered_raw}")).unwrap();
    crate::cli::pipeline::poly_format_strict(std::slice::from_ref(&scratch), dir.path())
        .expect("poly fmt must succeed");
    let converged = std::fs::read_to_string(&scratch).unwrap();
    std::fs::write(dir.path().join("pyproject.toml"), &converged).unwrap();
    std::fs::remove_file(&scratch).ok();
    let files = vec![gen_file("pyproject.toml", rendered_raw)];

    let (drifted, stats) = drifted_marked_paths(&files, dir.path());

    assert!(
        drifted.is_empty(),
        "an already up-to-date TOML file must not be reported drifted: {drifted:?}"
    );
    assert_eq!(stats.compared, 1);
}

/// THE CAVEAT this whole check has to get right: a self-marking backend (pyo3's `lib.rs`,
/// `docs::render`'s HTML-commented pages) templates its own provenance line straight into
/// `content` and is emitted with `generated_header: false`, so the header is never added a
/// second time by `ensure_generated_header`. A naive comparison of raw bytes would still work
/// here (the header is identical on both sides), but only because `matches_alef_output` is the
/// thing doing the comparing -- this proves the drifted case is still caught under that shape,
/// not just the `generated_header: true` shape the tests above cover.
///
/// A `.md` path, not `.rs`: `normalize_content` runs a real `rustfmt` subprocess over `.rs`
/// content, which would make this test's outcome depend on `rustfmt`'s canonical formatting of a
/// hand-written fixture (and on `rustfmt` being on `$PATH` at all) rather than on the marker
/// normalisation this test exists to prove -- see `frb_generated_drift_tests`'s identical
/// `is_tool_available("rustfmt")` guard for the same hazard elsewhere in this file's sibling
/// module. `docs::render`'s own self-marking pages are `.md` regardless, so this is not a
/// contrived extension for the shape under test. ~keep
#[test]
fn drifted_marked_paths_reports_a_self_marking_file_whose_body_no_longer_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    std::fs::write(
        dir.path().join("reference.md"),
        format!("{header}Describes `greet`, which says hi.\n"),
    )
    .unwrap();
    let files = vec![gen_file_unheadered(
        "reference.md",
        &format!("{header}Describes `greet`, which says hi warmly.\n"),
    )];

    let (drifted, _stats) = drifted_marked_paths(&files, dir.path());

    assert_eq!(
        drifted,
        vec![dir.path().join("reference.md").display().to_string()],
        "a self-marking file (marker baked into content, generated_header: false) whose body \
         changed must still be reported drifted"
    );
}

/// THE CONTROL for the self-marking shape above: once the baked-in body matches, the marker
/// normalisation must not manufacture a false difference from the header appearing in `content`
/// rather than being added by `ensure_generated_header`.
#[test]
fn drifted_marked_paths_is_silent_once_a_self_marking_file_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let content = format!("{header}Describes `greet`, which says hi warmly.\n");
    std::fs::write(dir.path().join("reference.md"), &content).unwrap();
    let files = vec![gen_file_unheadered("reference.md", &content)];

    let (drifted, _stats) = drifted_marked_paths(&files, dir.path());
    assert!(drifted.is_empty());
}
