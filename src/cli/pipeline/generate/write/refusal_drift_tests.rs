//! The refusal tally's CONTENT, not its firing.
//!
//! A refused write is reported to an operator as a number. Two very different things produce
//! that number: a write that would only have stamped a provenance header onto content that is
//! already correct, and a write that would have replaced genuinely different bytes. The second
//! means the file on disk is stale for as long as it stays frozen; the first means nothing at
//! all. Reporting them identically is what let a generated test-app installer, whose bytes bake
//! in the release version, sit pinned to a stale release in three consumer repositories while
//! every run said only "N file(s) were NOT written".
//!
//! In-crate rather than under `tests/` for the reason `user_owned_disposition_tests`' own
//! header gives: `tracing_test`'s subscriber is filtered to the test crate's name, so log
//! assertions made from an integration test would capture nothing and pass vacuously. ~keep

use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use std::path::PathBuf;
use tracing_test::traced_test;

fn generated(relative: &str, content: &str, generated_header: bool) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(relative),
        content: content.to_owned(),
        generated_header,
    }
}

fn write_one(base_dir: &std::path::Path, file: GeneratedFile) -> super::WriteReport {
    super::write_files_report(&[(Language::Java, vec![file])], base_dir).expect("write")
}

/// The defect: a refused write that WOULD have changed real bytes is indistinguishable, in
/// everything alef reports, from one that would only have added a header. This is the drifted
/// half -- an unmarked file whose body no longer matches what the generator produces, which is
/// the shape a version string baked into generated content takes after a release. ~keep
#[test]
fn a_refused_write_whose_body_differs_is_recorded_as_drifted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();
    std::fs::write(
        base.join("Widget.java"),
        "final class Widget { String v = \"1.2.1\"; }\n",
    )
    .expect("seed");

    let report = write_one(
        base,
        generated("Widget.java", "final class Widget { String v = \"1.4.2\"; }\n", true),
    );

    assert_eq!(report.refused_count(), 1, "the unmarked file must still be refused");
    assert_eq!(
        report.refused_drifted_count(),
        1,
        "a refusal that withheld different bytes must be classified as drifted, not merely counted"
    );
    assert!(report.refused_drifted_paths.contains(&base.join("Widget.java")));
}

/// THE CONTROL, and without it "classify every refusal as drifted" would pass the test above.
///
/// A `generated_header: true` file whose body already equals generated output is refused all
/// the same -- the writers compare the header-stamped bytes, and an unmarked file can never
/// equal them -- so this is not a rare shape, it is the ordinary one for a whole consumer tree
/// that predates alef stamping its extension. Nothing about it is stale, and calling it drifted
/// would put every such file on a list whose entire value is that each line on it is real. ~keep
#[test]
fn a_refused_write_whose_body_already_matches_is_not_recorded_as_drifted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();
    let body = "final class Widget { String v = \"1.4.2\"; }\n";
    std::fs::write(base.join("Widget.java"), body).expect("seed");

    let report = write_one(base, generated("Widget.java", body, true));

    assert_eq!(
        report.refused_count(),
        1,
        "fixture precondition: the guard still refuses, because the file carries no marker and \
         the header alef would add is a byte change"
    );
    assert_eq!(
        report.refused_drifted_count(),
        0,
        "the only withheld difference is the provenance header alef itself would add -- there is \
         no stale content here and the report must not claim there is"
    );
}

/// The tally is what an operator reads, so the drifted count has to be IN it. A report that
/// classified correctly and then printed the same sentence as before would leave the defect
/// exactly where it was. ~keep
#[test]
#[traced_test]
fn the_refusal_report_states_how_many_of_the_withheld_writes_had_different_content() {
    let mut report = super::WriteReport::default();
    report.refuse_text(
        std::path::Path::new("/repo/stale.sh"),
        Some("VERSION=1.2.1\n"),
        "VERSION=1.4.2\n",
        false,
    );
    report.refuse_text(
        std::path::Path::new("/repo/settled.sh"),
        Some("VERSION=1.4.2\n"),
        "VERSION=1.4.2\n",
        false,
    );

    super::report_refused_writes(&report);

    assert!(
        logs_contain("2 file(s) were NOT written, 1 of them holding content that DIFFERS"),
        "the tally must separate the withheld writes that had different bytes to deliver from \
         the ones that did not"
    );
    assert!(
        logs_contain("/repo/stale.sh"),
        "and must name the drifted path, since no count can identify which file went stale"
    );
    assert!(
        logs_contain("content already matches generated output: /repo/settled.sh"),
        "the benign path must be named as benign -- a reader who cannot tell them apart has to \
         open every file to find the one that matters"
    );
}

/// A binary target reaches the guard only after an exact byte comparison already failed, and a
/// text target alef cannot read as text cannot equal alef's own UTF-8 output. Neither has a
/// narrower answer available, and neither may be quietly dropped from the drifted count -- an
/// unclassifiable refusal reported as benign is the same silence this whole change removes. ~keep
#[test]
fn a_refusal_with_no_readable_existing_content_counts_as_drifted() {
    let mut report = super::WriteReport::default();
    report.refuse_text(std::path::Path::new("/repo/opaque.bin"), None, "text output\n", false);

    assert_eq!(report.refused_count(), 1);
    assert_eq!(report.refused_drifted_count(), 1);
}

/// `absorb_unwritten` folds every phase's refusals into one run-level report. A split tally
/// that folded only the total would let one phase's stale withheld content vanish, which is the
/// same class of bug as the count-only wrapper that once dropped `refused_paths` wholesale for
/// an entire category of writes in `alef all`. ~keep
#[test]
fn absorbing_another_phase_carries_its_drifted_subset_too() {
    let mut phase = super::WriteReport::default();
    phase.refuse_drifted(std::path::Path::new("/repo/a.rs"), false);
    let mut run = super::WriteReport::default();

    run.absorb_unwritten(&phase);

    assert_eq!(run.refused_count(), 1);
    assert_eq!(
        run.refused_drifted_count(),
        1,
        "folding the total while dropping the drifted subset would report the refusal and lose \
         the only fact that makes it actionable"
    );
}

/// THE MEASURED DEFECT: `write_files_report` refused a create-once seed (`generated_header:
/// false`, no marker on disk) exactly like any other unmarked path, so `refused_create_once_paths`
/// stayed empty for it -- the guard had the original `GeneratedFile` in hand and never asked
/// `commands::adopt::is_create_once_seed`. Fixed by computing `create_once` from that same
/// `GeneratedFile` while it is still in scope (see `write_files_report`'s `prepared` map) and
/// carrying it through to every `refuse(...)` call, the same way `write_scaffold_files_report`
/// now does. ~keep
#[test]
fn write_files_report_classifies_an_unmarked_seed_as_create_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();
    std::fs::write(base.join("go.mod"), "module example\n\ngo 1.20\n").expect("seed");

    let report = write_one(base, generated("go.mod", "module example\n\ngo 1.22\n", false));

    assert_eq!(report.refused_count(), 1, "the unmarked seed must still be refused");
    assert!(
        report.refused_create_once_paths.contains(&base.join("go.mod")),
        "a generated_header: false path is exactly what \
         `commands::adopt::is_create_once_seed` classifies as a create-once seed, and the guard \
         must agree"
    );
}

/// THE MEASURED DEFECT, end to end: `alef generate` reported a create-once seed under the same
/// ADOPTABLE heading as a genuinely adoptable frozen file, both pointed at `alef adopt <path>` --
/// and `alef adopt` refused the seed by design, naming a flag
/// (`--clobber-create-once-seeds`) this warning never mentioned. Measured in a consumer repo: 13
/// of 17 refused writes were create-once seeds (`*.csproj`, `pubspec.yaml`, `mix.exs`, `go.mod`,
/// `pom.xml`, `build.gradle.kts`, `gradle-wrapper.properties`, `package.json`, `Gemfile`,
/// `Package.swift`, `build.zig`, `build.zig.zon`).
///
/// Both assertions matter together: a fix that only removed the seed from the ADOPTABLE list
/// without still stating its count would silently drop it from the report altogether, which is
/// the one shape `report_refused_writes` may never take -- see
/// `WriteReport::refused_create_once_paths`'s doc for why the count survives the heading split. ~keep
#[test]
#[traced_test]
fn a_create_once_seed_refusal_is_never_folded_into_the_adoptable_block() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();
    std::fs::write(base.join("go.mod"), "module example\n\ngo 1.20\n").expect("seed");
    std::fs::write(
        base.join("Widget.java"),
        "final class Widget { String v = \"1.2.1\"; }\n",
    )
    .expect("adoptable");

    let report = super::write_files_report(
        &[
            (
                Language::Go,
                vec![generated("go.mod", "module example\n\ngo 1.22\n", false)],
            ),
            (
                Language::Java,
                vec![generated(
                    "Widget.java",
                    "final class Widget { String v = \"1.4.2\"; }\n",
                    true,
                )],
            ),
        ],
        base,
    )
    .expect("write");
    let seed_path = base.join("go.mod").display().to_string();
    let adoptable_path = base.join("Widget.java").display().to_string();

    super::report_refused_writes(&report);

    // PRESENT in the count: the seed is not dropped, just reclassified.
    assert!(
        logs_contain("1 file(s) were NOT written because they are create-once seeds"),
        "the seed must still be counted -- dropping it silently is the one regression a fix here \
         must never reintroduce"
    );
    assert!(
        logs_contain(&format!("create-once seed, not rewritten: {seed_path}")),
        "and named, the same way `bin_cli::helpers::frozen::unmarked_create_once_seeds` names it \
         in `alef verify`'s coverage report"
    );
    // ABSENT from the adoptable report: the seed's per-path line under the ADOPTABLE heading
    // must never appear, and the ADOPTABLE tally must count only the genuinely adoptable file.
    assert!(
        logs_contain("1 file(s) were NOT written, 1 of them holding content that DIFFERS"),
        "the ADOPTABLE tally must count Widget.java alone -- a seed folded back in would read \
         \"2 file(s)\" here, silently reintroducing the defect"
    );
    assert!(
        !logs_contain(&format!("stale until adopted or deleted): {seed_path}")),
        "a create-once seed must never appear in the ADOPTABLE per-path list, and must never be \
         pointed at `alef adopt <path>` -- `alef adopt` refuses it by design"
    );
    assert!(
        logs_contain(&format!("stale until adopted or deleted): {adoptable_path}")),
        "control: the genuinely adoptable file must still be named there"
    );
}
