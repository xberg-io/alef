use super::{VerifyCoverage, VerifyCoverageCounts};
use crate::bin_cli::verify_scan::ScanCoverage;
use std::collections::HashSet;
use std::path::PathBuf;

fn paths(directory: &std::path::Path, names: &[&str]) -> HashSet<PathBuf> {
    names.iter().map(|name| directory.join(name)).collect()
}

const ZERO_COVERAGE_COUNTS: VerifyCoverageCounts = VerifyCoverageCounts {
    create_once_unmarked: 0,
    create_once_drifted: 0,
    ephemeral_excluded: 0,
    declared_user_owned: 0,
};

/// The three managed buckets must partition the managed surface, and each must be measured
/// from what is actually true of the path -- marked, merely present, or absent.
///
/// The defect this closes is a report that counted only what it found: a run where 1 of 3
/// managed paths was content-verified read exactly like a run where all 3 were, because
/// nothing printed the denominator. ~keep
#[test]
fn managed_paths_split_into_verified_present_and_absent() {
    let directory = tempfile::tempdir().expect("temporary project");
    std::fs::write(directory.path().join("stamped.toml"), "x\n").expect("seed");
    std::fs::write(directory.path().join("unmarked.json"), "{}\n").expect("seed");

    let managed = paths(directory.path(), &["stamped.toml", "unmarked.json", "never_written.rs"]);
    let marked = paths(directory.path(), &["stamped.toml"]);

    let coverage = VerifyCoverage::measure(
        &managed,
        &marked,
        &HashSet::new(),
        ScanCoverage::default(),
        ZERO_COVERAGE_COUNTS,
    );
    assert_eq!(coverage.managed_total, 3);
    assert_eq!(coverage.managed_content_verified, 1);
    assert_eq!(coverage.managed_present_only, 1);
    assert_eq!(coverage.managed_absent, 1);
    assert_eq!(
        coverage.managed_content_verified + coverage.managed_present_only + coverage.managed_absent,
        coverage.managed_total,
        "the three buckets must partition the managed surface, or the report understates its own gap"
    );
}

/// A marked file the managed surface does not claim is counted apart from the surface, never
/// folded into it -- otherwise `managed_content_verified` could exceed `managed_total` and the
/// partition assertion above would silently become unsatisfiable. ~keep
#[test]
fn marked_files_outside_the_surface_are_counted_separately() {
    let directory = tempfile::tempdir().expect("temporary project");
    let managed = paths(directory.path(), &["a.rs"]);
    let marked = paths(directory.path(), &["a.rs", "legacy_visitor.py"]);

    let coverage = VerifyCoverage::measure(
        &managed,
        &marked,
        &HashSet::new(),
        ScanCoverage::default(),
        ZERO_COVERAGE_COUNTS,
    );
    assert_eq!(coverage.marked_outside_surface, 1);
    assert_eq!(coverage.managed_total, 1);
}

/// Markerless ownership-record paths count as content-verified only when the fresh-render
/// comparison actually examined their bytes. Merely being present and owned is insufficient. ~keep
#[test]
fn fresh_render_comparison_promotes_only_examined_markerless_paths() {
    let directory = tempfile::tempdir().expect("temporary project");
    for name in ["checked.jar", "owned-but-skipped.properties"] {
        std::fs::write(directory.path().join(name), "bytes").expect("seed managed path");
    }
    let managed = paths(directory.path(), &["checked.jar", "owned-but-skipped.properties"]);
    let compared = paths(directory.path(), &["checked.jar"]);

    let coverage = VerifyCoverage::measure(
        &managed,
        &HashSet::new(),
        &compared,
        ScanCoverage::default(),
        ZERO_COVERAGE_COUNTS,
    );

    assert_eq!(coverage.managed_content_verified, 1);
    assert_eq!(coverage.managed_present_only, 1);
}

/// The report must NAME the narrowness, not just print numbers. A reader who sees only counts
/// still has no reason to doubt "verify passed" means "the tree is fresh".
///
/// Each assertion below names one claim the report has to make; a rewording that drops any of
/// them fails here rather than quietly shrinking what the report admits to. ~keep
#[test]
fn report_states_that_presence_only_paths_are_not_content_checked() {
    let coverage = VerifyCoverage {
        managed_total: 10,
        managed_content_verified: 4,
        managed_present_only: 5,
        managed_absent: 1,
        marked_outside_surface: 2,
        files_opened: 40,
        files_unexamined: 900,
        create_once_unmarked: 0,
        create_once_drifted: 0,
        ephemeral_excluded: 0,
        declared_user_owned: 0,
    };
    let report = coverage.report_lines().join("\n");

    assert!(report.contains("content-verified"), "{report}");
    assert!(
        report.contains("present-but-wrong file passes"),
        "the report must say what a presence-only check fails to catch: {report}"
    );
    assert!(
        report.contains("never examined"),
        "the report must account for files the walk never opened: {report}"
    );
    for number in ["10", "4", "5", "40", "900", "2"] {
        assert!(
            report.contains(number),
            "every measured count must reach the report, missing {number}: {report}"
        );
    }
}

/// The orphan line is conditional; the four coverage claims are not. A run with a clean
/// surface must still state its scope. ~keep
#[test]
fn report_is_emitted_even_when_nothing_is_outside_the_surface() {
    let coverage = VerifyCoverage {
        managed_total: 3,
        managed_content_verified: 3,
        ..VerifyCoverage::default()
    };
    let lines = coverage.report_lines();
    assert_eq!(lines.len(), 6, "no orphan or seed line when neither applies: {lines:?}");
    assert!(lines[0].contains("Verify coverage"));
}

/// Part 1's regression, on the reporting side.
///
/// The unmarked create-once seeds must be STATED -- with their count, and as the documented
/// user-owned steady state rather than as drift -- and the report must not send the reader at
/// `--clobber-create-once-seeds`, which is the destructive escape the old frozen heading
/// offered for files alef never rewrites. ~keep
#[test]
fn report_states_create_once_seeds_as_coverage_not_as_drift() {
    let coverage = VerifyCoverage {
        managed_total: 100,
        managed_present_only: 72,
        create_once_unmarked: 72,
        ..VerifyCoverage::default()
    };
    let report = coverage.report_lines().join("\n");

    assert!(
        report.contains("72 are create-once seeds"),
        "the seed count must survive the removal of the frozen heading: {report}"
    );
    assert!(
        report.contains("not drift"),
        "a user-owned seed must not be described as drift: {report}"
    );
    assert!(
        !report.contains("clobber"),
        "verify must not point a user at --clobber-create-once-seeds for a file it never rewrites: {report}"
    );
}

/// A run with no seeds must not print the line at all, so the seed statement stays evidence
/// that seeds were found rather than boilerplate. ~keep
#[test]
fn report_omits_the_seed_line_when_there_are_no_seeds() {
    let report = VerifyCoverage::default().report_lines().join("\n");
    assert!(!report.contains("create-once seeds with no marker"), "{report}");
}

/// The coverage block asserted "the missing marker is the documented user-owned steady state,
/// not drift" over EVERY unmarked seed, having compared none of them. For the seeds alef
/// re-renders on every run that claim is simply false, and it was false for the exact files a
/// consumer found frozen at a stale release. A coverage report exists to stop a green result
/// being read as more than it is; it may not itself over-claim. ~keep
#[test]
fn report_does_not_call_a_re_attempted_drifted_seed_a_settled_steady_state() {
    let coverage = VerifyCoverage {
        managed_total: 100,
        managed_present_only: 72,
        create_once_unmarked: 72,
        create_once_drifted: 3,
        ..VerifyCoverage::default()
    };
    let report = coverage.report_lines().join("\n");

    assert!(
        report.contains("69 are create-once seeds"),
        "the steady-state claim may only cover the seeds alef does not re-attempt: {report}"
    );
    assert!(
        report.contains("and 3 more that alef DOES re-render every run"),
        "the drifted seeds must be counted separately rather than folded into the settled ones: {report}"
    );
    assert!(
        report.contains("stale, not settled"),
        "and must not be described as the documented steady state: {report}"
    );
}

/// The control: with nothing drifted, the second line must not appear at all, so its presence
/// stays evidence rather than boilerplate. ~keep
#[test]
fn report_omits_the_drifted_seed_line_when_no_seed_drifted() {
    let coverage = VerifyCoverage {
        managed_total: 100,
        managed_present_only: 72,
        create_once_unmarked: 72,
        ..VerifyCoverage::default()
    };
    let report = coverage.report_lines().join("\n");
    assert!(!report.contains("DOES re-render"), "{report}");
}
