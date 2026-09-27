//! Tests for [`super::real_formatter_drift_with`], via this file's own `real_formatter_drift`
//! convenience wrapper bound to the real `is_tool_available` (alef#436's second half) -- split
//! into its own module for the same reason `helpers/drift_tests.rs` splits out of
//! `helpers/tests.rs`: this repository's 1,000-line file cap, and this concern (comparing a
//! non-`.rs`/`.md` render against disk through a real formatter) is self-contained. Also covers
//! [`super::render_predicts_final_bytes`] and [`super::drifted_marked_paths_with`]'s alef#458
//! poly-availability gate on the `.rs`/`.md` fast path -- the closely related concern of deciding
//! whether that fast path may run at all before falling back to the real-formatter tier this
//! module otherwise covers. ~keep

use super::*;

/// Test-only convenience wrapper around [`real_formatter_drift_with`], bound to the real
/// [`crate::cli::pipeline::is_tool_available`] -- production has no call site for this exact
/// binding since alef#458 threads a single `is_available` closure through both
/// [`drifted_marked_paths_with`]'s tiers instead of resolving it twice, so this exists purely to
/// keep the tests below (which only ever care about the real binary, not the injectable seam)
/// from repeating that argument at every call site. ~keep
fn real_formatter_drift(
    candidates: Vec<RealFormatCandidate>,
    base_dir: &std::path::Path,
) -> (Vec<String>, FormatDriftStats) {
    real_formatter_drift_with(candidates, base_dir, &crate::cli::pipeline::is_tool_available)
}

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
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
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
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
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
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
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

/// THE GATING LOGIC: exactly two extensions are predictable, and neither of them depends on
/// `poly` being installed -- the function takes no availability argument at all any more.
///
/// `.rs` was gated on poly by alef#458 for a real cause that alef#465 removed: `alef all` used
/// to write Rust bindings before the scaffold stage emitted the `rustfmt.toml` that governs
/// them, so only the later `poly fmt --fix` pass left the bytes this prediction assumes. The
/// pre-pass now writes that config ahead of the bindings stage, which
/// [`rust_binding_bytes_on_disk_match_a_fresh_render_with_no_poly`] measures end to end against
/// a real `alef all` run with `poly` off `PATH`.
///
/// `.md` was never gated (alef#469): nothing writes a markdown config late, so with poly absent
/// the on-disk bytes are exactly what `normalize_content` produces and the prediction is
/// byte-exact. alef#458 gated both extensions together on implementation symmetry rather than
/// on a measured cause, which inverted `.md` -- it predicted where the prediction is
/// approximate (poly present, rumdl having run, and alef modelling only its MD012 rule) and
/// skipped where the prediction is perfect. That cost a real check:
/// `drift_tests::drifted_marked_paths_reports_a_self_marking_file_whose_body_no_longer_matches`
/// went red on Windows CI, the one leg with no poly installed.
///
/// Pure and host-independent (no subprocess, no real `poly` binary), so every case is provable
/// on any machine regardless of what it has installed. ~keep
#[test]
fn render_predicts_final_bytes_clears_rust_and_markdown_and_nothing_else() {
    let cases = [
        ("lib.rs", true),
        ("reference.md", true),
        ("pyproject.toml", false),
        ("bindings.go", false),
        ("noextension", false),
    ];
    for (name, expected) in cases {
        assert_eq!(
            render_predicts_final_bytes(std::path::Path::new(name)),
            expected,
            "render_predicts_final_bytes({name:?}) must be {expected}"
        );
    }
}

/// alef#465 END TO END, against [`drifted_marked_paths_with`]'s own injectable seam: with poly
/// unavailable, an `.rs` candidate must still be COMPARED on the fast path, in both directions
/// -- silent when it matches a fresh render, reported when it does not -- never routed into the
/// counted-skip bucket. This is the repin of the alef#458 contract that asserted the exact
/// opposite; the gate it pinned was suppressing this coverage on the one CI leg with no poly
/// installed rather than avoiding a false positive, once alef#465 made the first write already
/// rustfmt-formatted (measured by
/// [`rust_binding_bytes_on_disk_match_a_fresh_render_with_no_poly`]).
///
/// `FormatDriftStats` stays at its default in both halves on purpose: it counts only the
/// real-formatter tier, and a fast-path file never reaches that tier. Drives `&|_| false`
/// directly, so this is provable on a host that does, in fact, have `poly` installed. ~keep
#[test]
fn drifted_marked_paths_compares_an_rs_candidate_on_the_fast_path_when_poly_is_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from("lib.rs"),
        content: "pub fn greet() -> &'static str { \"old\" }\n".to_string(),
        generated_header: true,
    };
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&file), dir.path());
    std::fs::write(dir.path().join("lib.rs"), &rendered[0].content).unwrap();

    let (drifted, stats) = drifted_marked_paths_with(std::slice::from_ref(&file), dir.path(), &|_tool| false);

    assert!(
        drifted.is_empty(),
        "an up-to-date .rs file must be compared and found clean without poly, not reported: {drifted:?}"
    );
    assert_eq!(
        stats,
        FormatDriftStats::default(),
        "a fast-path file must never be counted as a real-formatter skip just because poly is absent"
    );

    let changed = vec![crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from("lib.rs"),
        content: "pub fn greet() -> &'static str { \"new\" }\n".to_string(),
        generated_header: true,
    }];

    let (drifted, stats) = drifted_marked_paths_with(&changed, dir.path(), &|_tool| false);

    assert_eq!(
        drifted,
        vec![dir.path().join("lib.rs").display().to_string()],
        "a genuinely stale .rs file must be REPORTED with poly absent -- this is the finding the \
         alef#458 gate suppressed on the no-poly CI leg"
    );
    assert_eq!(stats, FormatDriftStats::default());
}

/// THE MEASUREMENT that licensed dropping the `poly_available` gate from
/// [`render_predicts_final_bytes`]'s `.rs` arm (alef#465 vs alef#458), and the regression test
/// that keeps that licence valid.
///
/// The gate existed because `alef all` wrote Rust bindings BEFORE the scaffold stage emitted
/// `rustfmt.toml`, so on a first run the bindings landed unformatted while an in-memory
/// re-render -- which resolves `rustfmt.toml` fresh -- predicted formatted bytes. With poly
/// absent nothing repaired the disk copy, so the prediction was wrong and the fast path
/// manufactured a false drift finding.
///
/// This drives a real, from-scratch first `alef all` (no committed `rustfmt.toml`) with `poly`
/// made to look absent for the run's duration -- without that guard `alef all`'s own whole-tree
/// `poly fmt --fix` pass would repair the bytes regardless of the write-time ordering, and this
/// test would pass for the wrong reason -- and then asserts byte equality between every marked
/// `.rs` file on disk and a fresh [`crate::cli::pipeline::normalize_content`] render of those
/// same bytes. That render is the exact half of the fast-path prediction the gate distrusted,
/// so equality is what makes the ungated `.rs` arm honest; should this ever go red, the gate
/// has to come back rather than the assertion being relaxed.
///
/// `compared > 0` is asserted separately and deliberately: a run that generated no marked `.rs`
/// file at all would satisfy every byte comparison vacuously and render identically to a real
/// pass (the `prove-the-check-fired` rule). ~keep
#[test]
fn rust_binding_bytes_on_disk_match_a_fresh_render_with_no_poly() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_first_run_fixture(&root);
    assert!(
        !root.join("rustfmt.toml").exists(),
        "fixture setup: this run must start with no committed rustfmt.toml, or it is not \
         exercising a project's first `alef all` run at all"
    );

    let _cargo_guard = crate::test_support::RealCargoGuard::acquire();
    let _no_poly = crate::test_support::PathWithoutToolGuard::exclude("poly");
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = crate::bin_cli::dispatch::DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    crate::bin_cli::all_commands::handle(first_run_all_command(), &context)
        .expect("alef all must succeed against a plain python fixture");

    let mut compared = 0usize;
    let mut mismatches = Vec::new();
    for entry in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let Ok(disk) = std::fs::read_to_string(path) else {
            continue;
        };
        if !crate::core::hash::content_has_alef_marker(&disk) {
            continue;
        }
        compared += 1;
        let rerendered = crate::cli::pipeline::normalize_content(&root, path, &disk);
        if rerendered != disk {
            mismatches.push(format!(
                "{}\n--- on disk ---\n{}\n--- fresh render ---\n{}",
                path.display(),
                first_differing_lines(&disk, &rerendered),
                first_differing_lines(&rerendered, &disk)
            ));
        }
    }

    assert!(
        compared > 0,
        "no marked .rs file was produced by this run, so every byte comparison above was \
         vacuous -- this check examined nothing"
    );
    assert!(
        mismatches.is_empty(),
        "{compared} marked .rs file(s) compared; {} differ from a fresh in-memory render on a \
         first run with no poly, so the .rs fast-path prediction is still unsound:\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

/// First few lines of `left` from the point it stops matching `right`, for the diff excerpt the
/// assertion above prints. Whole-file dumps of generated bindings are unreadable in test output.
fn first_differing_lines(left: &str, right: &str) -> String {
    let start = left
        .lines()
        .zip(right.lines())
        .position(|(l, r)| l != r)
        .unwrap_or_else(|| left.lines().count().min(right.lines().count()));
    left.lines().skip(start).take(6).collect::<Vec<_>>().join("\n")
}

const FIRST_RUN_FIXTURE_SOURCE: &str = "pub fn compute_widget_totals(\n    \
    alpha: String,\n    bravo: String,\n    charlie: String,\n    delta: String,\n    echo: String,\n    \
    foxtrot: String,\n    golf: String,\n    hotel: String,\n    india: String,\n    juliett: String,\n\
) -> String {\n    \
    format!(\"{alpha}{bravo}{charlie}{delta}{echo}{foxtrot}{golf}{hotel}{india}{juliett}\")\n\
}\n";

const FIRST_RUN_FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

const FIRST_RUN_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#;

fn write_first_run_fixture(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIRST_RUN_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIRST_RUN_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIRST_RUN_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

fn first_run_all_command() -> crate::bin_cli::args::Commands {
    crate::bin_cli::args::Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
        skip_compile: true,
    }
}
