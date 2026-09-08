use super::*;
use crate::core::config::Language;

fn resolved_test_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
command = "pytest"

[crates.test.rust]
e2e = "cargo test"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Seed one stamped file per name and return what the ownership walk actually opened.
fn scanned_names(names: &[&str]) -> Vec<String> {
    let directory = tempfile::tempdir().expect("temporary project");
    for name in names {
        let path = directory.path().join(name);
        let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
    }
    let mut found: Vec<String> = collect_alef_hashes(directory.path())
        .into_iter()
        .filter_map(|(path, _, _)| path.file_name()?.to_str().map(str::to_owned))
        .collect();
    found.sort();
    found
}

/// Seed one stamped file per relative path (creating parent directories) and return the
/// repository-relative paths the ownership walk actually opened, sorted.
fn scanned_relative_paths(relative_paths: &[&str]) -> Vec<String> {
    let directory = tempfile::tempdir().expect("temporary project");
    for relative in relative_paths {
        let path = directory.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("seeded path has a parent")).expect("seed parent directory");
        let marker = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        std::fs::write(&path, format!("{marker}\nseeded = true\n")).expect("seed stamped file");
    }
    let mut found: Vec<String> = collect_alef_hashes(directory.path())
        .into_iter()
        .filter_map(|(path, _, _)| {
            path.strip_prefix(directory.path())
                .ok()?
                .to_str()
                .map(|value| value.replace('\\', "/"))
        })
        .collect();
    found.sort();
    found
}

/// alef stamps files inside dot-directories (`.cargo/config.toml`, agent-skill `SKILL.md`s)
/// that the walk used to prune wholesale, so those stamps were written and never read: no
/// amount of drift could make `alef verify` report them.
///
/// Paired control in one run, because a walk that finds nothing and a walk that finds
/// everything are the same green: a stamped file in an ordinary directory must be found (the
/// walk works at all), a stamped file in `.cargo` must now also be found (the fix), and a
/// stamped file in `.venv` must still be missed (the prune still keeps the walk out of tool
/// caches — the fix is an allowlist, not a removal).
#[test]
fn the_ownership_walk_reaches_the_dot_directories_alef_stamps() {
    let found = scanned_relative_paths(&[
        "packages/reachable.toml",
        ".cargo/config.toml",
        ".github/skills/api/SKILL.md",
        ".venv/lib/cached.toml",
    ]);

    assert!(
        found.contains(&"packages/reachable.toml".to_string()),
        "control: a stamped file outside every dot-directory must be found, else this test \
         proves nothing about the dot-directory cases; walk returned {found:?}"
    );
    assert!(
        found.contains(&".cargo/config.toml".to_string()),
        "alef writes and stamps `.cargo/config.toml` itself; a stamp nothing ever reads back \
         is not a freshness check. Walk returned {found:?}"
    );
    assert!(
        found.contains(&".github/skills/api/SKILL.md".to_string()),
        "generated agent skills are stamped alef output and must be verifiable; walk returned \
         {found:?}"
    );
    assert!(
        !found.contains(&".venv/lib/cached.toml".to_string()),
        "the dot-directory prune must still keep the walk out of tool caches -- the fix is an \
         allowlist of the dot-directories alef writes into, not a removal of the prune. Walk \
         returned {found:?}"
    );
}

/// A nested git worktree is a second checkout of the same repository. It became reachable the
/// moment `.claude` came off the blanket prune, and walking it reports another branch's
/// stamps as this tree's.
#[test]
fn the_ownership_walk_does_not_descend_into_a_nested_worktree() {
    let found = scanned_relative_paths(&[".claude/skills/api/SKILL.md", ".claude/worktrees/other/config.toml"]);

    assert!(
        found.contains(&".claude/skills/api/SKILL.md".to_string()),
        "control: `.claude` must be walked, else the exclusion below is vacuous; walk \
         returned {found:?}"
    );
    assert!(
        !found.contains(&".claude/worktrees/other/config.toml".to_string()),
        "a nested worktree is a different checkout of this repository; its stamps are not \
         this tree's. Walk returned {found:?}"
    );
}

/// THE AGREEMENT CANARY. `alef verify`'s frozen-file report and `alef adopt`'s candidate
/// set are the report and the remedy for one fact, so a path in one and not the other
/// sends a reader to a command that refuses them. They diverged exactly that way: each
/// was built from its own hand-maintained stage list, adopt's missing service/public API
/// and both missing e2e, test apps, READMEs and docs — which is why `alef adopt` on an
/// e2e snippet glob bailed with "no alef-managed output matches" while 15,677 snippets
/// sat frozen and unreported.
///
/// Asserting on behaviour would need a full extraction + generation pass against a real
/// crate, which is what made the divergence invisible to the test suite in the first
/// place. This asserts on the structure that produced it instead: each consumer derives
/// its set from [`collect_managed_surface`] and enumerates no stage of its own. It fails
/// the moment either one grows a private list again, which is the regression. ~keep
#[test]
fn both_consumers_build_their_managed_set_only_from_the_shared_surface() {
    // Every stage entry point `collect_managed_surface` composes. A consumer that
    // names one inside its own region is re-deriving the surface instead of sharing
    // it. `generate(` carries its parenthesis because `generate_` prefixes several
    // of the others. ~keep
    let stage_calls = [
        "pipeline::generate(",
        "pipeline::generate_stubs(",
        "pipeline::generate_service_api(",
        "pipeline::generate_public_api(",
        "pipeline::scaffold(",
        "pipeline::readme(",
        "e2e::generate_e2e(",
        "e2e::generate_e2e_with_log(",
        "docs::generate_docs_stage(",
    ];
    // Each region is the consumer's own code, cut so it excludes the shared collector
    // itself, which must name every stage. `alef adopt`'s region is now its whole handler
    // module: since the adopt arm moved out of `aux_commands` it no longer has to be sliced
    // away from the unrelated `Commands::Init` arm that legitimately generates and writes. ~keep
    let regions = [
        (
            "alef verify's frozen report",
            include_str!("../helpers.rs")
                .split("pub(crate) fn collect_managed_surface")
                .next()
                .expect("helpers splits on the shared collector"),
        ),
        ("alef adopt's candidate set", include_str!("../adopt_command.rs")),
    ];
    for (name, region) in regions {
        for call in stage_calls {
            assert!(
                !region.contains(call),
                "{name} calls {call} directly -- the frozen report and the candidate set \
                 must not enumerate generation stages separately, or they disagree again"
            );
        }
        assert!(
            region.contains("collect_managed_surface("),
            "{name} must derive its managed set from the shared surface"
        );
    }
}

/// THE CANARY. Every name here is stamped by `marker_header_syntax` on the emit side, so a
/// file alef wrote carries a hash this walk must be able to re-read. Before the list was
/// widened these were stamped and then never opened — which reads as covered rather than as
/// missing, and is why the gap survived its own doc comment's warning. ~keep
#[test]
fn ownership_walk_opens_every_extension_the_emit_side_stamps() {
    assert_eq!(
        scanned_names(&[
            "foo-config.cmake",
            "app.csproj",
            "gem.gemspec",
            "build.zig.zon",
            "pom.xml"
        ]),
        vec![
            "app.csproj",
            "build.zig.zon",
            "foo-config.cmake",
            "gem.gemspec",
            "pom.xml"
        ],
    );
}

/// The makefiles and `Rakefile` have no extension at all, and `go.mod`'s is the far-too-broad
/// `mod` — shared with unrelated binary formats — so all of them are keyed on file name on the
/// emit side and must be keyed the same way here.
///
/// Every entry of `VERIFY_SCAN_FILENAMES` that is not a dotfile appears below, so an addition
/// to that list without a matching read-side check fails here rather than passing quietly.
///
/// `makefile` gets its own directory: macOS and Windows resolve it and `Makefile` to the same
/// path, so seeding both in one directory silently writes a single file and the lowercase
/// entry would look unscanned when it is only unwritten. ~keep
#[test]
fn ownership_walk_opens_the_filename_keyed_files_the_emit_side_stamps() {
    assert_eq!(scanned_names(&["makefile"]), vec!["makefile"]);
    assert_eq!(
        scanned_names(&[
            "Makefile",
            "GNUmakefile",
            "Rakefile",
            "Makevars",
            "Makevars.in",
            "Makevars.win.in",
            "go.mod"
        ]),
        vec![
            "GNUmakefile",
            "Makefile",
            "Makevars",
            "Makevars.in",
            "Makevars.win.in",
            "Rakefile",
            "go.mod"
        ],
    );
}

/// The other half of the predicate: widening the allowlist must not turn the walk into
/// "open everything". Without this, both tests above would still pass if the filter were
/// deleted outright. ~keep
#[test]
fn ownership_walk_still_skips_an_extension_alef_never_stamps() {
    assert!(scanned_names(&["notes.rtf", "archive.tar"]).is_empty());
}

#[test]
fn default_log_level_maps_verbosity_to_levels() {
    assert_eq!(default_log_level(0, false), "info");
    assert_eq!(default_log_level(1, false), "debug");
    assert_eq!(default_log_level(2, false), "trace");
    assert_eq!(default_log_level(9, false), "trace");
    // --quiet wins over any -v count.
    assert_eq!(default_log_level(0, true), "error");
    assert_eq!(default_log_level(3, true), "error");
}

#[test]
fn resolve_test_languages_allows_explicit_test_only_language() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, Some(&["rust".to_string()]), true).unwrap();
    assert_eq!(langs, vec![Language::Rust]);
}

#[test]
fn resolve_test_languages_appends_e2e_only_languages() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, None, true).unwrap();
    assert_eq!(langs, vec![Language::Python, Language::Rust]);
}

#[test]
fn resolve_test_languages_omits_e2e_only_languages_without_e2e() {
    let config = resolved_test_config();
    let langs = resolve_test_languages(&config, None, false).unwrap();
    assert_eq!(langs, vec![Language::Python]);
}

fn gen_file(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: true,
    }
}

#[test]
fn generated_files_match_disk_true_when_bodies_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
    let files = vec![gen_file("binding.go", "package x\n\nvar a = 1\n")];
    assert!(generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_ignores_embedded_hash_line() {
    let dir = tempfile::tempdir().unwrap();
    let generated = "// This file is auto-generated by alef — DO NOT EDIT.\npackage x\n\nvar a = 1\n";
    std::fs::write(
        dir.path().join("binding.go"),
        "// This file is auto-generated by alef — DO NOT EDIT.\n// alef:hash:deadbeef\npackage x\n\nvar a = 1\n",
    )
    .unwrap();
    let files = vec![gen_file("binding.go", generated)];
    assert!(generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_false_when_body_differs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("binding.go"), "package x\n\nvar a = 1\n").unwrap();
    let files = vec![gen_file("binding.go", "package x\n\nimport \"fmt\"\n\nvar a = 1\n")];
    assert!(!generated_files_match_disk(&files, dir.path()));
}

#[test]
fn generated_files_match_disk_false_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![gen_file("binding.go", "package x\n")];
    assert!(!generated_files_match_disk(&files, dir.path()));
}

fn gen_file_unheadered(rel: &str, content: &str) -> crate::core::backend::GeneratedFile {
    crate::core::backend::GeneratedFile {
        path: std::path::PathBuf::from(rel),
        content: content.to_string(),
        generated_header: false,
    }
}

/// The defect this closes: a backend that would produce a file (e.g. one
/// Java/C# file per public type) is invisible to a pure disk walk when the
/// file was never written — `alef verify` must catch that, not just an
/// existing file whose hash drifted.
#[test]
fn missing_managed_paths_reports_an_absent_headered_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    let missing = missing_managed_paths(&files, dir.path());

    assert_eq!(missing, vec![dir.path().join("SomeType.java").display().to_string()]);
}

/// Positive control: an up-to-date tree (every generated path already
/// present on disk) must report nothing missing, regardless of the file's
/// actual content — content drift is `verify_walk`'s job, not this check's.
#[test]
fn missing_managed_paths_reports_nothing_when_every_headered_file_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("SomeType.java"),
        "final class SomeType { /* stale */ }\n",
    )
    .unwrap();
    let files = vec![gen_file("SomeType.java", "final class SomeType {}\n")];

    assert!(missing_managed_paths(&files, dir.path()).is_empty());
}

/// The required negative control: a legitimately user-owned, unheadered
/// scaffold-once file (`Cargo.toml`, `package.json`, gemspec, lockfiles —
/// see `verify_walk`'s doc comment) that is absent must NOT be reported
/// missing. Getting this wrong would fail verify on every clean repo whose
/// scaffold-once files simply haven't been (re-)generated locally.
#[test]
fn missing_managed_paths_ignores_an_absent_unheadered_scaffold_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = vec![gen_file_unheadered("Cargo.toml", "[package]\nname = \"demo\"\n")];

    assert!(missing_managed_paths(&files, dir.path()).is_empty());
}

#[test]
fn marker_line_finds_the_line_carrying_the_provenance_marker() {
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);

    assert_eq!(
        marker_line(&header),
        Some("// This file is auto-generated by alef — DO NOT EDIT.")
    );
}

#[test]
fn marker_line_finds_nothing_in_content_without_a_marker() {
    assert_eq!(marker_line("final class SomeType {}\n"), None);
}

#[test]
fn verify_walk_detects_an_edited_generated_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("binding.rs");
    let original = "// This file is auto-generated by alef — DO NOT EDIT.\nfn value() -> u8 { 1 }\n";
    let hash = crate::core::hash::compute_file_hash(original);
    let finalized = crate::core::hash::inject_hash_line(original, &hash);
    std::fs::write(&path, finalized.replace("{ 1 }", "{ 2 }")).expect("edit generated file");

    let stale = verify_walk(directory.path()).expect("verify generated files");

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, path.display().to_string());
}

/// Regression coverage for a hole reported against `alef verify`: "the inputs-hash
/// covers generation inputs, not output bytes, so a dependency bumped inside a
/// stamped, alef-generated manifest reports fresh." That does not hold for markable,
/// stamped files -- `compute_file_hash` is a pure function of the file's own content
/// (see its doc and `core::hash`'s module doc), so a hand-edited dependency version --
/// e.g. `cargo upgrade --incompatible` bumping `base64` in place inside a generated
/// JNI/FFI `Cargo.toml` -- is exactly the "content changed" case this test pins down.
/// If `compute_file_hash`/`verify_walk` ever regress to skipping content and comparing
/// generation inputs alone, this must start failing. ~keep
#[test]
fn verify_walk_detects_a_hand_edited_dependency_version_in_a_generated_manifest() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("Cargo.toml");
    let original = "# This file is auto-generated by alef — DO NOT EDIT.\n\
                     [dependencies]\n\
                     base64 = \"0.22\"\n";
    let hash = crate::core::hash::compute_file_hash(original);
    let finalized = crate::core::hash::inject_hash_line(original, &hash);
    // Only the on-disk bytes are hand-edited, as `cargo upgrade --incompatible` would do
    // to a generated manifest.
    std::fs::write(&path, finalized.replace("0.22", "0.23")).expect("hand-edit generated manifest");

    let stale = verify_walk(directory.path()).expect("verify generated files");

    assert_eq!(
        stale.len(),
        1,
        "a hand-edited dependency version must be reported stale"
    );
    assert_eq!(stale[0].path, path.display().to_string());
}

#[test]
fn verify_walk_detects_a_mixed_stamped_and_unstamped_generated_tree() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("unstamped.rs");
    std::fs::write(
        &path,
        "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
    )
    .expect("write generated file");
    let stamped_path = directory.path().join("stamped.rs");
    let stamped_body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn stamped() {}\n";
    let stamped_hash = crate::core::hash::compute_file_hash(stamped_body);
    std::fs::write(
        &stamped_path,
        crate::core::hash::inject_hash_line(stamped_body, &stamped_hash),
    )
    .expect("write stamped generated file");

    let stale = verify_walk(directory.path()).expect("verify generated files");

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, path.display().to_string());
    assert_eq!(stale[0].embedded, "<missing>");
}

/// `find_stamp_disagreement` walks `collect_alef_hashes`, which only yields files that
/// carry an `alef:hash:` line — so a fixture bearing only a stamp is invisible to it and
/// every assertion over it passes vacuously. Both lines must be injected, in that order,
/// to produce a file shaped like one a backend actually emits. ~keep
fn write_stamped(dir: &std::path::Path, name: &str, key: &str, value: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let body = "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n";
    let stamped = crate::core::hash::inject_stamp_line(body, key, value);
    let hash = crate::core::hash::compute_file_hash(&stamped);
    std::fs::write(&path, crate::core::hash::inject_hash_line(&stamped, &hash)).expect("write stamped file");
    path
}

/// Guards the fixture itself, because the bug this replaces was a fixture bug, not a
/// logic bug: if `write_stamped` ever stops producing a file the hash walk can see, the
/// disagreement tests below go quietly vacuous instead of failing. ~keep
#[test]
fn write_stamped_produces_a_file_the_hash_walk_actually_collects() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "1");

    let collected = collect_alef_hashes(dir.path());
    assert_eq!(
        collected.len(),
        1,
        "the stamped fixture must be visible to the hash walk"
    );
    assert_eq!(
        crate::core::hash::extract_stamp(&collected[0].2, "handle-abi").as_deref(),
        Some("1"),
        "the stamp must survive alongside the hash line"
    );
}

/// The concrete cross-artifact ABI straddle this closes: an FFI-side file
/// stamped for one ABI generation coexisting with a binding-side file
/// stamped for a different one must be reported, even though each file's
/// own `alef:hash:` may be perfectly fresh relative to current inputs.
#[test]
fn find_stamp_disagreement_reports_two_distinct_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "1");
    write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

    let disagreement =
        find_stamp_disagreement(dir.path(), "handle-abi").expect("two distinct stamp values must be reported");

    assert_eq!(disagreement.key, "handle-abi");
    assert_eq!(disagreement.examples.len(), 2);
    let values: Vec<&str> = disagreement.examples.iter().map(|(_, v)| v.as_str()).collect();
    assert!(values.contains(&"1"));
    assert!(values.contains(&"2"));
}

/// Positive control: every stamped file agreeing must not be reported —
/// this is the healthy, fully-regenerated-together state.
#[test]
fn find_stamp_disagreement_is_none_when_every_stamped_file_agrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "handle-abi", "2");
    write_stamped(dir.path(), "binding.zig", "handle-abi", "2");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

/// The required negative control for the rollout gap the task describes:
/// a tree where no backend has started emitting the stamp yet (every
/// consumer repo today) must not be reported as disagreeing — there is
/// nothing to compare, not a proven mismatch.
#[test]
fn find_stamp_disagreement_is_none_when_nothing_is_stamped() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("header.h"),
        "// This file is auto-generated by alef — DO NOT EDIT.\nfn generated() {}\n",
    )
    .expect("write unstamped file");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

/// A file stamped under a different key must not be mistaken for a
/// `handle-abi` disagreement — `find_stamp_disagreement` is keyed, not a
/// blanket "does this file have any stamp" check.
#[test]
fn find_stamp_disagreement_ignores_a_different_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stamped(dir.path(), "header.h", "some-other-marker", "1");
    write_stamped(dir.path(), "binding.zig", "some-other-marker", "2");

    assert!(find_stamp_disagreement(dir.path(), "handle-abi").is_none());
}

fn stage_failure(paths: &[&str]) -> StageFailure {
    StageFailure {
        stage: "e2e",
        message: "56 e2e assertion(s) reference a field the availability oracle cannot resolve".to_owned(),
        paths: paths.iter().map(std::path::PathBuf::from).collect(),
    }
}

/// THE REGRESSION. `alef adopt packages/dart/rust/Cargo.toml` deadlocked because a
/// pending e2e strict-assertion failure aborted `collect_managed_surface` before the
/// ownership-only `Cargo.toml` target was ever considered, even though that target
/// has no relationship to e2e. `affects_any` is the predicate that now lets `Commands::Adopt`
/// tell the two cases apart: this asserts the tolerant half -- an e2e failure whose
/// rendered paths are all snippet/test-app output must not be judged to affect an
/// unrelated `Cargo.toml` target, whatever glob shape the operator typed. ~keep
#[test]
fn a_stage_failure_confined_to_e2e_paths_does_not_affect_an_unrelated_ownership_target() {
    let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

    assert!(!failure.affects_any(&["packages/dart/rust/Cargo.toml".to_owned()]));
    assert!(!failure.affects_any(&["packages/**/*.gemspec".to_owned()]));
}

/// The control for the test above: when a requested target genuinely falls under the
/// failing stage's own output, `affects_any` must say so, literal path or glob alike,
/// so `alef adopt` still refuses to answer for a target it cannot render correctly
/// rather than silently tolerating every e2e failure regardless of relevance.
#[test]
fn a_stage_failure_that_rendered_the_requested_target_does_affect_it() {
    let failure = stage_failure(&["e2e/python/test_smoke.py", "e2e/go/smoke_test.go"]);

    assert!(failure.affects_any(&["e2e/python/test_smoke.py".to_owned()]));
    assert!(failure.affects_any(&["e2e/python/*.py".to_owned()]));
    // Mixed: one target unrelated, one that matches -- still affects, because a
    // multi-target `alef adopt` run answers for every target it was given. ~keep
    assert!(failure.affects_any(&[
        "packages/dart/rust/Cargo.toml".to_owned(),
        "e2e/go/smoke_test.go".to_owned(),
    ]));
}

/// `alef verify` passes no targets at all -- every tolerated failure is unconditional
/// debt for a read-only report, never excused by "no target asked for it". An empty
/// `targets` slice must therefore never affect anything, which is the same fact
/// `Commands::Adopt` relies on for a target list that turned out to filter down to
/// nothing upstream.
#[test]
fn a_stage_failure_never_affects_an_empty_target_list() {
    let failure = stage_failure(&["e2e/python/test_smoke.py"]);

    assert!(!failure.affects_any(&[]));
}

fn swift_only_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// THE REGRESSION (latent): `alef verify` never runs post-build steps
/// (`complete_generated_artifacts` is `Commands::Generate`/`Commands::All`-only), so a path a
/// post-build step owns unguarded -- see `PostBuildStep::owned_paths` -- can never appear in
/// `collect_managed_surface`'s in-memory surface. Left out of `managed_paths`, that path would
/// misreport as an orphan on every single `alef verify` run the moment such a step writes an
/// alef-marked file there. Swift's `MaterializeSwiftBridge` is the real post-build step this
/// exercises: `SwiftBridgeCore.swift` is a path it owns (`PostBuildStep::owned_paths`) but that
/// `collect_managed_surface`'s in-band bindings stage never emits as a `GeneratedFile` -- see
/// `emit_swift_bridge_files`'s doc, which reads real `target/` build output only when called
/// from the post-build step, never from `alef generate`'s own in-memory render. No shipped
/// backend's `owned_paths` output actually carries an alef marker today (this file's own writer
/// never headers it), which is why this is latent rather than a live false positive -- but a
/// marked file at this exact path must still resolve as owned once one does. ~keep
#[test]
fn a_post_build_owned_path_not_produced_in_band_is_not_reported_as_an_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = swift_only_config();
    let api = crate::core::ir::ApiSurface::default();
    let config_path = dir.path().join("alef.toml");

    // Matches `SwiftBackend::build_config_with_config`'s `package_root` for the default (no
    // `[crates.output] swift` override) layout: `<base_dir>` IS the package root, so this is
    // `packages/swift/Sources/RustBridge/SwiftBridgeCore.swift` regardless of what has or has not
    // been built on disk yet -- see `swift_package_root` in `backends::swift::gen_bindings`.
    let owned_path = dir
        .path()
        .join("packages/swift/Sources/RustBridge/SwiftBridgeCore.swift");
    std::fs::create_dir_all(owned_path.parent().unwrap()).expect("create Sources/RustBridge");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let marked = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&owned_path, marked).expect("write post-build-owned file");

    let found = find_missing_and_frozen_generated_files(&[Language::Swift], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed over a swift-only crate");

    assert!(
        found.managed_paths.contains(&owned_path),
        "post-build-owned paths must be folded into the managed surface: {:?}",
        found.managed_paths
    );

    let orphans = super::super::verify_orphans::find_orphaned_generated_files(
        dir.path(),
        &found.managed_paths,
        &Default::default(),
    );
    assert!(
        orphans.is_empty(),
        "a path a post-build step owns unguarded must never be reported as an orphan just \
         because `alef verify` cannot run that step itself: {orphans:?}"
    );
}

/// THE DEFECT this closes: a managed path that is absent from disk AND excluded by a
/// project-level `.gitignore` rule used to report exactly like an ordinary "never generated
/// yet" absence -- same list, same `alef generate` remedy. Running generate can never close a
/// gitignored gap: the file gets written, the ignore rule discards it again before it can be
/// committed, and the very next `alef verify` (on a fresh clone or in CI, which never had the
/// file to begin with) reports it missing again, forever. `find_missing_and_frozen_generated_files`
/// must split such a path into `missing_gitignored` instead of leaving it in plain `missing`.
///
/// Runs the real `collect_managed_surface` pipeline (not a hand-built `GeneratedFile` list) so
/// this is a true end-to-end proof that the split reaches this function's return value, not
/// just `split_missing_by_gitignore` in isolation (already covered by
/// `verify_gitignore::tests`).
#[test]
fn find_missing_and_frozen_generated_files_splits_out_a_gitignored_managed_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = swift_only_config();
    let api = crate::core::ir::ApiSurface::default();
    let config_path = dir.path().join("alef.toml");

    let baseline = find_missing_and_frozen_generated_files(&[Language::Swift], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed over a swift-only crate");
    assert!(
        !baseline.missing.is_empty(),
        "fixture precondition: a fresh directory with no generated output must have at least one \
         managed path outstanding, or this test cannot prove anything -- got: {:?}",
        baseline.missing
    );
    assert!(
        baseline.missing_gitignored.is_empty(),
        "no .gitignore exists yet, so nothing should split out: {:?}",
        baseline.missing_gitignored
    );

    let target = baseline.missing.first().cloned().expect("checked non-empty above");
    let target_relative = std::path::Path::new(&target)
        .strip_prefix(dir.path())
        .expect("missing paths are joined onto base_dir")
        .to_owned();

    let git_init_status = crate::test_support::git_command(dir.path())
        .args(["init", "--quiet"])
        .status()
        .expect("git init must run");
    if !git_init_status.success() {
        // No git on `$PATH` in this environment -- the fallback behavior itself is covered
        // directly by `verify_gitignore::tests`'s own outside-a-work-tree fallback test.
        return;
    }
    let ignore_parent = dir
        .path()
        .join(target_relative.parent().unwrap_or_else(|| std::path::Path::new(".")));
    std::fs::create_dir_all(&ignore_parent).expect("create the target's parent directory");
    std::fs::write(
        ignore_parent.join(".gitignore"),
        format!(
            "{}\n",
            target_relative
                .file_name()
                .expect("a managed path has a file name")
                .to_string_lossy()
        ),
    )
    .expect("write a .gitignore excluding exactly the chosen target");

    let found = find_missing_and_frozen_generated_files(&[Language::Swift], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed over a swift-only crate");

    assert!(
        found.missing_gitignored.contains(&target),
        "the gitignored managed path must move to missing_gitignored, got: {:?}",
        found.missing_gitignored
    );
    assert!(
        !found.missing.contains(&target),
        "the gitignored managed path must not remain in plain missing (its remedy differs -- \
         `alef generate` cannot fix it), got: {:?}",
        found.missing
    );
}

/// alef-tasks #318, MEASURED before this test existed: a crate with an `[e2e]` block always
/// runs the registry-mode test-app stage as part of `alef verify`'s managed surface, regardless
/// of whether the consumer commits that output. A consumer whose `.gitignore` excludes the
/// whole `test_apps/` tree -- because it is ephemeral, regenerated per CI run, and deliberately
/// never committed -- got every one of those paths routed into `missing_gitignored`, a HARD,
/// PERMANENT failure `alef generate` can never fix (the file is written, then discarded by the
/// ignore rule before it can be committed). A minimal repro (one fixture, one language) measured
/// 3 of 3 registry-mode files landing there with no config to say otherwise.
///
/// `[crates.verify].ignore_ephemeral` (`crate::core::config::VerifyConfig`) is the fix:
/// `partition_ephemeral` excludes exactly the paths the glob names, no others, from
/// `missing`/`missing_gitignored` -- proved here directly against `find_missing_and_frozen_generated_files`'s
/// real output rather than a hand-built fixture, so this cannot pass by the two drifting apart.
fn registry_test_apps_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::create_dir_all(dir.path().join("fixtures")).expect("create fixtures dir");
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: String) -> String { format!(\"hi {name}\") }\n",
    )
    .expect("write lib.rs");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"measurelib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(
        dir.path().join("fixtures/greet_basic.json"),
        r#"{
  "id": "greet_basic",
  "description": "greet",
  "category": "smoke",
  "tags": ["smoke"],
  "call": "_default",
  "input": { "name": "world" },
  "assertions": [{ "type": "not_error" }]
}
"#,
    )
    .expect("write fixture json");
    // Whole-directory ignore, mirroring a consumer that treats registry-mode `test_apps/` as
    // ephemeral, regenerated-per-run output it deliberately never commits.
    std::fs::write(dir.path().join(".gitignore"), "/test_apps/\n").expect("write .gitignore");
    let git_init_status = crate::test_support::git_command(dir.path())
        .args(["init", "--quiet"])
        .status()
        .expect("git init must run");
    assert!(git_init_status.success(), "git init must succeed in this environment");
    let config_path = dir.path().join("alef.toml");
    (dir, config_path)
}

const REGISTRY_TEST_APPS_CALL_BLOCK: &str = r#"
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["python"]

[crates.e2e.call]
function = "greet"
module = "measurelib"
result_var = "result"

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"
"#;

#[test]
fn registry_test_apps_output_under_a_whole_directory_gitignore_is_a_hard_failure_without_ignore_ephemeral() {
    let (dir, config_path) = registry_test_apps_workspace();
    let config_toml = format!(
        "[workspace]\nlanguages = [\"python\"]\n\n[[crates]]\nname = \"measurelib\"\nsources = [\"src/lib.rs\"]\n{REGISTRY_TEST_APPS_CALL_BLOCK}"
    );
    std::fs::write(&config_path, &config_toml).expect("write alef.toml");
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&config_toml).expect("config parses");
    let config = cfg.resolve().expect("config resolves").remove(0);
    assert!(
        config.verify.ignore_ephemeral.is_empty(),
        "fixture precondition: no opt-out configured"
    );
    let api = crate::core::ir::ApiSurface::default();
    let _cwd = crate::test_support::CwdGuard::enter(dir.path());

    let found = find_missing_and_frozen_generated_files(&[Language::Python], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed");

    let test_apps_missing_gitignored = found
        .missing_gitignored
        .iter()
        .filter(|path| path.contains("test_apps"))
        .count();
    assert!(
        test_apps_missing_gitignored > 0,
        "fixture precondition: registry-mode output must exist and land in missing_gitignored \
         with no opt-out configured, got: {:?}",
        found.missing_gitignored
    );
}

#[test]
fn ignore_ephemeral_excludes_registry_test_apps_output_from_missing_and_missing_gitignored() {
    let (dir, config_path) = registry_test_apps_workspace();
    let config_toml = format!(
        "[workspace]\nlanguages = [\"python\"]\n\n[[crates]]\nname = \"measurelib\"\nsources = [\"src/lib.rs\"]\n{REGISTRY_TEST_APPS_CALL_BLOCK}\n[crates.verify]\nignore_ephemeral = [\"test_apps/**\"]\n"
    );
    std::fs::write(&config_path, &config_toml).expect("write alef.toml");
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&config_toml).expect("config parses");
    let config = cfg.resolve().expect("config resolves").remove(0);
    assert_eq!(config.verify.ignore_ephemeral, vec!["test_apps/**".to_string()]);
    let api = crate::core::ir::ApiSurface::default();
    let _cwd = crate::test_support::CwdGuard::enter(dir.path());

    let found = find_missing_and_frozen_generated_files(&[Language::Python], &api, &config, &config_path, dir.path())
        .expect("collect_managed_surface must succeed");
    assert!(
        found.missing_gitignored.iter().any(|path| path.contains("test_apps")),
        "fixture precondition: registry-mode output must still be gitignored-missing BEFORE the \
         opt-out is applied, got: {:?}",
        found.missing_gitignored
    );

    let (missing, missing_excluded) = config.verify.partition_ephemeral(found.missing, dir.path());
    let (missing_gitignored, gitignored_excluded) =
        config.verify.partition_ephemeral(found.missing_gitignored, dir.path());

    assert!(
        !missing_gitignored.iter().any(|path| path.contains("test_apps")),
        "ignore_ephemeral must remove every test_apps path from missing_gitignored: {missing_gitignored:?}"
    );
    assert!(
        !missing.iter().any(|path| path.contains("test_apps")),
        "ignore_ephemeral must remove every test_apps path from missing: {missing:?}"
    );
    assert!(
        gitignored_excluded > 0,
        "the exclusion must be counted, not just applied silently"
    );
    assert_eq!(
        missing_excluded, 0,
        "no plain-missing entries exist under test_apps in this fixture"
    );
}
