//! Class gate for alef #474: every e2e-generated file on a **markable** extension must carry a
//! provenance marker.
//!
//! The freeze the issue reports is not a kotlin_android bug, it is a hole between the two
//! ownership rails. `cli::pipeline::generate::scaffold::write_scaffold_files_report` -- which
//! both e2e stages call, exactly as scaffold does -- records an ownership-manifest entry only
//! for paths `cli::pipeline::generate::write::marker_comment_style` answers `None` for, on the
//! reasoning that a markable file proves ownership through its own marker instead. A file that
//! is markable and yet ships unmarked therefore lands on neither rail: no manifest entry is
//! written, no marker is stamped, and the next template change is refused forever.
//!
//! That hole is invisible per-backend, so this sweep drives [`all_generators`] -- the same list
//! the real `alef e2e generate` dispatches through -- rather than a hand-written path list a new
//! backend would silently miss. Markers are preferred over manifest entries here for the reason
//! `write::marker_header_syntax`'s own doc gives: a marker cannot be separated from the file it
//! describes, while a record can be deleted, moved or gitignored away from it. ~keep

// ~keep The executed-file counts are reported to stderr deliberately: a sweep whose generators
// all returned zero files would pass identically to a real one, which is the vacuous-gate shape
// this file exists to prevent. `logging-tracing`'s documented carve-out for a test file applies.
#![allow(clippy::print_stderr)]

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{CallConfig, DependencyMode};
use crate::e2e::codegen::{E2eCodegen, all_generators};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// Generators whose output for *some* row carries no markable file at all, with the reason. A
/// row that examines nothing must be named rather than silently counted as a pass.
///
/// ~keep `r` emits only `.R`/`DESCRIPTION` in either mode; `php_ext` and `homebrew` emit only
/// `.php`/`Brewfile`/`README.md` in local mode and do contribute a markable `run_tests.sh` in
/// registry mode. `marker_comment_style` answers `None` for all of those extensions, so those
/// rows are covered by the ownership-manifest rail instead of by a marker.
const NO_MARKABLE_OUTPUT: &[&str] = &["r", "php_ext", "homebrew"];

/// Registry-mode generators that legitimately refuse to render without extra crate config, so
/// their registry row is skipped rather than treated as a finding. Both already contribute a
/// full local-mode row to the sweep.
///
/// ~keep Matched against the rendered error text, not just the language, so a generator that
/// starts failing for some *other* reason surfaces as a real failure instead of hiding here.
const REGISTRY_REQUIRES_GITHUB_REPO: &[&str] = &["zig", "r"];

fn sweep_groups() -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "marker_coverage".to_string(),
        fixtures: vec![Fixture {
            id: "marker_coverage_case".to_string(),
            description: "marker coverage sweep fixture".to_string(),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            }],
            ..Fixture::default()
        }],
    }]
}

/// ~keep `dep_mode` and a non-empty `[e2e.env]` are both load-bearing for coverage, not
/// decoration: php's `install.sh` is emitted only in registry mode and rust's
/// `.cargo/config.toml` only when `env` is non-empty, so a sweep that left either at its default
/// would never see those two files at all.
fn sweep_config(dep_mode: DependencyMode) -> E2eConfig {
    let mut config = E2eConfig {
        call: CallConfig {
            function: "marker_coverage_call".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        dep_mode,
        ..E2eConfig::default()
    };
    config.env.insert("ALEF_MARKER_COVERAGE".to_string(), "1".to_string());
    config
}

#[test]
fn every_markable_e2e_generated_file_carries_a_provenance_marker() {
    let crate_config = ResolvedCrateConfig::default();
    let groups = sweep_groups();
    let mut unmarked: Vec<String> = Vec::new();
    let mut markable_seen = 0usize;
    let mut total_seen = 0usize;
    let mut contributing: Vec<&str> = Vec::new();

    for (mode_label, e2e_config) in [
        ("local", sweep_config(DependencyMode::Local)),
        ("registry", sweep_config(DependencyMode::Registry)),
    ] {
        for generator in all_generators() {
            let language = generator.language_name();
            let files = match generator.generate(&groups, &e2e_config, &crate_config, &[], &[], &[], &[]) {
                Ok(files) => files,
                Err(error) => {
                    assert!(
                        mode_label == "registry"
                            && REGISTRY_REQUIRES_GITHUB_REPO.contains(&language)
                            && error.to_string().contains("github_repo"),
                        "{mode_label}/{language}: generator failed unexpectedly: {error}"
                    );
                    continue;
                }
            };
            assert!(
                !files.is_empty(),
                "{mode_label}/{language}: generator emitted no files at all, so this row proves nothing"
            );
            total_seen += files.len();
            let mut markable_here = 0usize;
            for file in files {
                if crate::cli::pipeline::generate::write::marker_comment_style(&file.path).is_none() {
                    continue;
                }
                markable_here += 1;
                markable_seen += 1;
                if !file.carries_alef_marker() {
                    unmarked.push(format!("{mode_label}/{language}: {}", file.path.display()));
                }
            }
            if markable_here > 0 && !contributing.contains(&language) {
                contributing.push(language);
            }
            assert!(
                markable_here > 0 || NO_MARKABLE_OUTPUT.contains(&language),
                "{mode_label}/{language}: emitted no markable file, so it silently contributes \
                 nothing to this gate -- add it to NO_MARKABLE_OUTPUT with a reason, or fix the sweep"
            );
        }
    }

    eprintln!(
        "marker sweep: {} generated file(s) across 2 dependency modes, {markable_seen} on a markable \
         extension, {} generator(s) contributing",
        total_seen,
        contributing.len()
    );
    assert!(
        markable_seen >= 60,
        "sweep examined only {markable_seen} markable file(s); it is no longer covering the surface \
         it was written against ({} generators contributed)",
        contributing.len()
    );
    assert!(
        unmarked.is_empty(),
        "{} e2e-generated file(s) land on a markable extension but carry no alef provenance marker, \
         so the scaffold writer records no ownership entry for them either (alef #474) -- stamp each \
         with crate::core::hash::SELF_MARKING_HEADER_LINE{{,_HASH}}:\n  {}",
        unmarked.len(),
        unmarked.join("\n  ")
    );
}

/// The filed instance of the class, driven end to end through the real write guard.
///
/// The sweep above proves the marker is *emitted*; this proves the marker is what unfreezes the
/// file, so a future change that swapped it for something `content_has_alef_marker` does not
/// accept would fail here rather than pass a content assertion. The two halves are both needed:
/// deleting the marker makes the second write a refusal, and that refusal is the whole of alef
/// #474 -- `alef verify` then reports the path as drifted and `alef adopt --write` bails on it as
/// a create-once seed. ~keep
#[test]
fn a_template_change_to_the_kotlin_android_build_file_is_not_refused_on_the_second_write() {
    use crate::core::backend::GeneratedFile;

    let temp = tempfile::tempdir().expect("tempdir");
    let base = temp.path();

    let generated = crate::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen
        .generate(
            &sweep_groups(),
            &sweep_config(DependencyMode::Local),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("kotlin_android e2e generation");
    let build_file = generated
        .iter()
        .find(|file| file.path.ends_with("build.gradle.kts"))
        .expect("kotlin_android emits build.gradle.kts");

    let first = crate::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(build_file), base, true)
        .expect("first write");
    assert!(
        first.refused_paths.is_empty(),
        "the first write creates the file and can never be refused: {:?}",
        first.refused_paths
    );
    let on_disk = base.join(&build_file.path);
    assert!(on_disk.is_file(), "first write must create {}", on_disk.display());

    // What an alef release that changes the Gradle template looks like from this file's side.
    let after_template_change = GeneratedFile {
        path: build_file.path.clone(),
        content: format!(
            "{}\n// a later alef release renders one more line\n",
            build_file.content
        ),
        generated_header: build_file.generated_header,
    };
    let second =
        crate::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&after_template_change), base, true)
            .expect("second write");

    assert!(
        second.refused_paths.is_empty(),
        "alef #474: the regenerated build.gradle.kts was refused, so the file is frozen -- \
         {:?}",
        second.refused_paths
    );
    let written = std::fs::read_to_string(&on_disk).expect("read back");
    assert_eq!(
        written, after_template_change.content,
        "the template change must actually reach disk, not merely avoid a refusal"
    );

    // Without the marker the same second write is refused -- and, because `.kts` is markable,
    // `write_scaffold_files_report` records no ownership entry either, so nothing else can
    // vouch for the file. This is the negative control for the assertion above. ~keep
    let unmarked_temp = tempfile::tempdir().expect("tempdir");
    let unmarked_base = unmarked_temp.path();
    let stripped: String = build_file
        .content
        .lines()
        .filter(|line| !line.contains("Generated by alef"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !crate::core::hash::content_has_alef_marker(&stripped),
        "the negative control must actually be unmarked, or it proves nothing"
    );
    let unmarked = GeneratedFile {
        path: build_file.path.clone(),
        content: stripped.clone(),
        generated_header: false,
    };
    crate::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&unmarked), unmarked_base, true)
        .expect("first unmarked write");
    let unmarked_changed = GeneratedFile {
        path: build_file.path.clone(),
        content: format!("{stripped}\n// a later alef release renders one more line\n"),
        generated_header: false,
    };
    let refused =
        crate::cli::pipeline::write_scaffold_files_report(std::slice::from_ref(&unmarked_changed), unmarked_base, true)
            .expect("second unmarked write");
    assert_eq!(
        refused.refused_paths.len(),
        1,
        "negative control: an unmarked, unrecorded .kts must be refused on the second write, \
         otherwise the positive assertion above passes for some reason other than the marker"
    );
}
