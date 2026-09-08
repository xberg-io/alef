//! Compiles the one shape none of `generated_output_downstream_gate`'s existing lanes reach: a
//! cfg-gated enum variant owned by a FOREIGN crate (merged into `toolkit`'s binding surface via
//! `[[crates.source_crates]].roots`, not declared in `toolkit` itself), under a feature set that
//! proves the variant unreachable.
//!
//! Alef drops foreign cfg-gated variants when the configured feature set proves them unreachable.
//! This module first compiles that clean emitted shape, then proves the gate is sensitive to both
//! conversion directions: one sabotage removes the reachable `Accent` arm from the wrapper-to-core
//! match, and the other adds an unreachable catch-all to the exhaustive core-to-wrapper match.
//! The expected `non_exhaustive_patterns` and `unreachable_patterns` diagnostics demonstrate that
//! the gate is compiling the generated code and examining the intended match blocks. ~keep
//!
//! Fixture text lives in `fixture.rs` (`FOREIGN_CRATE_CARGO_TOML`, `FOREIGN_CRATE_SOURCE`, the
//! `[[crates.source_crates]]` / `[crates.extra_dependencies]` entries in `FIXTURE_ALEF_TOML`, and
//! the `foreign_core` path dependency in `FIXTURE_CARGO_TOML`). [`write_fixture_workspace`] --
//! moved here in full, not just extended, so `tests/generated_output_downstream_gate.rs` (already
//! at its `file_size_baseline.txt` ceiling) does not have to grow to wire it in -- writes that
//! text to disk with the one substitution none of it can be a fixed string for: the absolute
//! path to the sibling `foreign_core` crate, known only once a fixture tempdir exists.
//!
//! Targets pyo3 specifically, not every clippy-lane language: `Swatch`'s exact merged
//! `rust_path` (`foreign_core::Swatch`) and per-arm text (`Self::Base`, not a full path -- see
//! `codegen::conversions::helpers::enum_arms`) are verified against source, matching the
//! existing pyo3 end-to-end regression test's own fixture shape (`RoutingStrategy`/`dep_crate`).
//! wasm is affected by the same historical bug and is also clippy-lane-covered, but is not
//! independently proven here -- one real compile of the real shape is the point; a second
//! backend would be marginal coverage for a large jump in fragility (a different per-arm text
//! format to keep in sync by hand). If wasm's handling of this shape regresses independently of
//! pyo3's, `emitted_tree_passes_clippy` (unmodified, now exercising `Swatch` for every
//! clippy-lane language because `fixture.rs`'s fixture source is shared) still catches it -- just
//! without naming the exact defect the way this module's sabotage tests do. ~keep

use std::path::{Path, PathBuf};

use super::fixture::{
    FIXTURE_ALEF_TOML, FIXTURE_CARGO_TOML, FIXTURE_SOURCE, FOREIGN_CRATE_CARGO_TOML, FOREIGN_CRATE_SOURCE,
};
use super::{CARGO, EmittedTree, LaneOutcome, Sabotage, emit_tree, fixture_language_list, resolve_tools, run_tool};

/// Write the whole fixture workspace to `root`: `toolkit` (the primary fixture crate) plus its
/// sibling `foreign_core` (see the module doc). Called from `emit_tree`
/// (`generated_output_downstream_gate.rs`) before `alef generate` runs.
pub(crate) fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    // `trim_start`: the raw literal opens with a newline, which poly reports as a reformat of
    // the fixture's own source rather than of anything alef produced. ~keep
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE.trim_start()).expect("write fixture source");

    let foreign_core_dir = write_foreign_crate(root);
    let dep_path = foreign_core_dir.display().to_string();
    let source_path = foreign_core_dir.join("src/lib.rs").display().to_string();

    let cargo_toml = FIXTURE_CARGO_TOML.replace("__FOREIGN_CORE_DEP_PATH__", &dep_path);
    std::fs::write(root.join("Cargo.toml"), cargo_toml).expect("write fixture Cargo.toml");

    let config = FIXTURE_ALEF_TOML
        .replace("__ALEF_VERSION__", env!("CARGO_PKG_VERSION"))
        .replace("__LANGUAGES__", &fixture_language_list())
        .replace("__FOREIGN_CORE_DEP_PATH__", &dep_path)
        .replace("__FOREIGN_CORE_SOURCE_PATH__", &source_path);
    std::fs::write(root.join("alef.toml"), config).expect("write fixture alef.toml");
}

/// Write the `foreign_core` crate into the fixture workspace at `<root>/foreign_core` and
/// return its directory.
///
/// `[[crates.source_crates]]` needs a real file to parse and every clippy-lane binding crate
/// needs a real path dependency to compile against, so this runs before `alef generate`.
fn write_foreign_crate(root: &Path) -> PathBuf {
    let dir = root.join("foreign_core");
    std::fs::create_dir_all(dir.join("src")).expect("create foreign_core src directory");
    std::fs::write(dir.join("src/lib.rs"), FOREIGN_CRATE_SOURCE.trim_start()).expect("write foreign_core source");
    std::fs::write(dir.join("Cargo.toml"), FOREIGN_CRATE_CARGO_TOML).expect("write foreign_core Cargo.toml");
    dir
}

/// The emitted pyo3 (python) binding crate's directory.
///
/// Scoped to `-py` specifically rather than reusing `clippy_manifest_dirs`'s full set: this
/// module's sabotages know the exact generated text pyo3 emits for `Swatch`, not every
/// clippy-lane language's.
fn python_crate_dir(tree: &EmittedTree) -> PathBuf {
    tree.manifest_dirs()
        .into_iter()
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("-py"))
        })
        .expect("emitted tree has no `*-py` crate directory; `alef generate` may have changed the pyo3 output layout")
        .to_path_buf()
}

/// The emitted pyo3 binding crate's `src/lib.rs`, where `Swatch`'s conversion `impl`s live.
fn python_lib_rs(tree: &EmittedTree) -> PathBuf {
    python_crate_dir(tree).join("src/lib.rs")
}

/// The byte range `[start, end)` of the `impl From<...> for ... {` block starting at `header`,
/// through its own closing brace -- the first `"\n}"` after `header`, which is the outer impl
/// block's brace because every brace nested inside it (the `fn`'s, the `match`'s) is indented
/// and so never appears at column 0 right after a newline. Mirrors
/// `backends::pyo3::gen_bindings::cfg_variant_e2e_tests`'s `core_to_binding_conversion` /
/// `binding_to_core_conversion` helpers, which established this exact technique against the
/// same template output (`src/codegen/templates/conversions/enum_from_*_to_*.jinja`). ~keep
fn impl_block_range(source: &str, header: &str) -> (usize, usize) {
    let start = source
        .find(header)
        .unwrap_or_else(|| panic!("emitted pyo3 lib.rs has no `{header}` block:\n{source}"));
    let end = source[start..]
        .find("\n}")
        .map(|offset| start + offset + 2)
        .unwrap_or_else(|| panic!("`{header}` block never closes in emitted pyo3 lib.rs"));
    (start, end)
}

/// Run `cargo clippy -- -D warnings` in `dir` alone, not the full multi-language `clippy_lane`
/// -- this module's tests are about one specific generated block in one specific backend, so a
/// failure should not also pay for compiling ffi/node/wasm/jni.
fn run_python_clippy(tree: &EmittedTree) -> LaneOutcome {
    run_tool(CARGO.program, CARGO.check_args, &python_crate_dir(tree))
}

/// Sabotage 1, the E0004 shape: drop a compiled variant arm from the binding-to-core conversion.
fn drop_binding_to_core_variant_arm(tree: &EmittedTree) {
    let path = python_lib_rs(tree);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let (start, end) = impl_block_range(&source, "impl From<Swatch> for foreign_core::Swatch {");
    let block = &source[start..end];
    let target = "Swatch::Accent => Self::Accent,";
    assert!(
        block.matches(target).count() == 1,
        "the binding-to-core block has no Accent arm to remove: {block}"
    );
    let sabotaged_block = block.replacen(target, "", 1);
    let sabotaged = format!("{}{sabotaged_block}{}", &source[..start], &source[end..]);
    std::fs::write(&path, sabotaged).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

/// Sabotage 2, the `unreachable_patterns` shape -- the opposite direction from sabotage 1, and
/// historically the shape fixing sabotage 1 regressed INTO (see `generated_output_downstream_gate.rs`'s
/// module doc: "fixing one caused the other"): add a redundant catch-all to `impl
/// From<foreign_core::Swatch> for Swatch` (core -> binding), which correctly has none. That
/// match is over the REAL, 2-variant compiled `foreign_core::Swatch` (built with `spot-colors`
/// off), so it is already exhaustive and a trailing wildcard is dead code.
fn add_redundant_core_to_binding_catch_all(tree: &EmittedTree) {
    let path = python_lib_rs(tree);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let (start, end) = impl_block_range(&source, "impl From<foreign_core::Swatch> for Swatch {");
    let block = &source[start..end];
    assert!(
        !block.contains("_ => Default::default(),"),
        "the core-to-binding block already has a catch-all, so adding a second one would not \
         prove this sabotage examines anything new:\n{block}"
    );
    let last_arm = "foreign_core::Swatch::Accent => Self::Accent,";
    assert_eq!(
        block.matches(last_arm).count(),
        1,
        "core-to-binding block must have exactly one Accent arm: {block}"
    );
    let sabotaged_block = block.replacen(last_arm, &format!("{last_arm} _ => Default::default(),"), 1);
    let sabotaged = format!("{}{sabotaged_block}{}", &source[..start], &source[end..]);
    std::fs::write(&path, sabotaged).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

#[test]
#[ignore = "compiles the emitted pyo3 crate; run via the CI gate job"]
fn clippy_lane_catches_a_missing_binding_to_core_variant_arm() {
    resolve_tools(&[&CARGO]);
    let clean = emit_tree(Sabotage::None);
    let control = run_python_clippy(&clean);
    assert!(
        control.passed,
        "the control tree must be green, or the sabotage proves nothing:\n{}",
        control.output
    );

    let sabotaged = emit_tree(Sabotage::None);
    drop_binding_to_core_variant_arm(&sabotaged);
    let outcome = run_python_clippy(&sabotaged);
    assert!(
        !outcome.passed,
        "dropping a compiled binding-to-core variant arm did not fail `cargo clippy`, so this \
         lane is not examining the emitted pyo3 conversion"
    );
    assert!(
        outcome.output.contains("E0004") || outcome.output.contains("non-exhaustive"),
        "the build failed, but not with the expected `error[E0004]: non-exhaustive patterns` -- \
         this sabotage may be failing for an unrelated reason:\n{}",
        outcome.output
    );
}

#[test]
#[ignore = "compiles the emitted pyo3 crate; run via the CI gate job"]
fn clippy_lane_catches_a_redundant_foreign_cfg_wrapper_catch_all() {
    resolve_tools(&[&CARGO]);
    let clean = emit_tree(Sabotage::None);
    let control = run_python_clippy(&clean);
    assert!(
        control.passed,
        "the control tree must be green, or the sabotage proves nothing:\n{}",
        control.output
    );

    let sabotaged = emit_tree(Sabotage::None);
    add_redundant_core_to_binding_catch_all(&sabotaged);
    let outcome = run_python_clippy(&sabotaged);
    assert!(
        !outcome.passed,
        "adding a redundant catch-all to the already-exhaustive core-to-binding match did not \
         fail `cargo clippy -- -D warnings`, so this lane is not examining the emitted pyo3 \
         conversion"
    );
    assert!(
        outcome.output.contains("unreachable"),
        "the build failed, but not with the expected `unreachable_patterns` -- this sabotage may \
         be failing for an unrelated reason:\n{}",
        outcome.output
    );
}

/// [`impl_block_range`] is the load-bearing piece every sabotage above depends on to locate the
/// right block and nothing else; this pins its brace-matching behavior directly, independent of
/// a real `alef generate` run.
#[test]
fn impl_block_range_stops_at_the_outer_closing_brace() {
    let source = "prefix
impl From<A> for B {
    fn from(val: A) -> Self {
        match val {
            A::X => Self::X,
        }
    }
}
suffix
";
    let (start, end) = impl_block_range(source, "impl From<A> for B {");
    let block = &source[start..end];
    assert!(block.starts_with("impl From<A> for B {"));
    assert!(block.ends_with('}'), "block must end at the outer brace: {block:?}");
    assert!(!block.contains("suffix"));
    assert_eq!(&source[end..], "\nsuffix\n");
}
