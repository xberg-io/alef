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

    let drifted = drifted_marked_paths(&files, dir.path());

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

    assert!(drifted_marked_paths(&files, dir.path()).is_empty());
}

/// An unmarked pre-existing file is `frozen_managed_paths`'s condition, not this one's --
/// reporting it here too would describe the same withheld write under two different headings.
#[test]
fn drifted_marked_paths_ignores_a_file_that_carries_no_marker_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("lib.rs"), "pub fn greet() {}\n").unwrap();
    let files = vec![gen_file("lib.rs", "pub fn greet() { /* changed */ }\n")];

    assert!(drifted_marked_paths(&files, dir.path()).is_empty());
}

/// THE MEASURED SCOPE LIMIT (see `drifted_marked_paths`'s doc for the full evidence): a marked
/// TOML file, even with genuinely different content, must not be reported. `alef all`'s
/// "Formatting generated files..." stage runs `poly fmt --fix` (taplo for TOML, ruff for
/// Python) over every path it just wrote, AFTER this in-memory render is computed --
/// `normalize_content` has no TOML/Python emulation of that pass, so comparing against it would
/// report every one of poly's own reformatting as drift on a tree `alef all` just produced
/// cleanly. Regression coverage for exactly that: `verify_reports_an_incomplete_generation_run_
/// instead_of_ordinary_staleness` (and two siblings) in `core_commands::tests` broke on a real
/// `alef all` fixture the first time this check ran over every extension. ~keep
#[test]
fn drifted_marked_paths_does_not_check_an_extension_poly_fmt_can_still_reformat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    std::fs::write(
        dir.path().join("pyproject.toml"),
        format!("{header}\n[project]\nname=\"old\"\n"),
    )
    .unwrap();
    let files = vec![gen_file("pyproject.toml", "[project]\nname=\"new\"\n")];

    assert!(
        drifted_marked_paths(&files, dir.path()).is_empty(),
        "TOML (and every other extension `normalize_content` does not fully normalize) is out of \
         scope for this check until it can predict poly fmt's final bytes"
    );
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

    let drifted = drifted_marked_paths(&files, dir.path());

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

    assert!(drifted_marked_paths(&files, dir.path()).is_empty());
}
