//! Tests for [`super::drifted_managed_paths`] (alef#436) -- split out of `helpers/tests.rs`
//! rather than added to it: that file sits at this repository's 1,000-line cap, and this
//! concern (deciding which already-marked, present files are drifted from a fresh render) is
//! self-contained enough to own its own module, mirroring how `helpers/frozen/tests.rs` already
//! splits the neighbouring frozen-file concern out for the same reason. ~keep

use super::*;
use base64::Engine;

/// A minimal resolved config for the drift tests: no `[crates.e2e]` block, so
/// `formatting_owner` never reaches its override branch and every path is classified by poly
/// coverage and the residual table alone -- which is what these tests are about. ~keep
fn drift_test_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    cfg.resolve().expect("resolvable").remove(0)
}

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

fn e2e_owned_output_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "run"
[crates.e2e.registry]
output = "test_apps"
"#,
    )
    .expect("valid config");
    cfg.resolve().expect("resolvable").remove(0)
}

fn seed_owned_output(base: &std::path::Path, relative: &str, content: &[u8]) {
    let full_path = base.join(relative);
    std::fs::create_dir_all(full_path.parent().expect("output parent")).expect("create output parent");
    std::fs::write(&full_path, content).expect("seed owned output");
    crate::cli::cache::record_scaffold_owned_path(base, &full_path).expect("record output ownership");
}

/// Markerless Maven wrapper bytes are authoritative under the rewritten E2E root once the
/// committed ownership record proves alef wrote them. Both the current and stale paths must be
/// compared in the same call so a vacuous implementation cannot satisfy the silence control. ~keep
#[test]
fn drifted_managed_paths_compares_current_and_stale_owned_maven_wrappers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let current = "e2e/java/mvnw";
    let stale = "e2e/java/mvnw.cmd";
    seed_owned_output(dir.path(), current, b"#!/bin/sh\nexec mvn\n");
    seed_owned_output(dir.path(), stale, b"@echo old\r\n");
    let files = vec![
        gen_file_unheadered(current, "#!/bin/sh\nexec mvn\n"),
        gen_file_unheadered(stale, "@echo new\r\n"),
    ];

    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &e2e_owned_output_config());

    assert_eq!(drifted, vec![dir.path().join(stale).display().to_string()]);
    assert_eq!(
        stats.content_verified_paths,
        std::collections::HashSet::from([dir.path().join(current), dir.path().join(stale)])
    );
}

/// Gradle's markerless script and properties file use the same ownership-record rail as Maven.
/// The properties path is deliberately stale while the script is current. ~keep
#[test]
fn drifted_managed_paths_compares_current_and_stale_owned_gradle_wrappers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let current = "test_apps/kotlin_android/gradlew";
    let stale = "test_apps/kotlin_android/gradle/wrapper/gradle-wrapper.properties";
    seed_owned_output(dir.path(), current, b"#!/bin/sh\nexec gradle\n");
    seed_owned_output(dir.path(), stale, b"distributionUrl=old\n");
    let files = vec![
        gen_file_unheadered(current, "#!/bin/sh\nexec gradle\n"),
        gen_file_unheadered(stale, "distributionUrl=new\n"),
    ];

    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &e2e_owned_output_config());

    assert_eq!(drifted, vec![dir.path().join(stale).display().to_string()]);
    assert_eq!(
        stats.content_verified_paths,
        std::collections::HashSet::from([dir.path().join(current), dir.path().join(stale)])
    );
}

/// Binary wrapper verification must compare decoded bytes, not the base64 carrier text. ~keep
#[test]
fn drifted_managed_paths_compares_owned_gradle_wrapper_jar_bytes() {
    const JAR: &str = "test_apps/kotlin_android/gradle/wrapper/gradle-wrapper.jar";
    let dir = tempfile::tempdir().expect("tempdir");
    let generated = [0x50, 0x4b, 0x03, 0x04, 0xff];
    seed_owned_output(dir.path(), JAR, &generated);
    let file = gen_file_unheadered(JAR, &base64::engine::general_purpose::STANDARD.encode(generated));

    let (current, current_stats) =
        drifted_managed_paths(std::slice::from_ref(&file), dir.path(), &e2e_owned_output_config());
    assert!(current.is_empty());
    assert_eq!(
        current_stats.content_verified_paths,
        std::collections::HashSet::from([dir.path().join(JAR)])
    );

    std::fs::write(dir.path().join(JAR), [0x50, 0x4b, 0x03]).expect("mutate wrapper jar");
    let (stale, stale_stats) =
        drifted_managed_paths(std::slice::from_ref(&file), dir.path(), &e2e_owned_output_config());
    assert_eq!(stale, vec![dir.path().join(JAR).display().to_string()]);
    assert_eq!(
        stale_stats.content_verified_paths,
        std::collections::HashSet::from([dir.path().join(JAR)])
    );
}

/// Coincidental bytes never establish ownership: without `.alef-ownership.toml`, a markerless
/// wrapper stays outside the fresh-render verdict even when the current render differs. ~keep
#[test]
fn drifted_managed_paths_ignores_unowned_markerless_wrapper() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative = "e2e/java/mvnw";
    let full_path = dir.path().join(relative);
    std::fs::create_dir_all(full_path.parent().expect("output parent")).expect("create output parent");
    std::fs::write(&full_path, "old\n").expect("seed unowned wrapper");

    let (drifted, stats) = drifted_managed_paths(
        &[gen_file_unheadered(relative, "new\n")],
        dir.path(),
        &e2e_owned_output_config(),
    );

    assert!(drifted.is_empty());
    assert!(stats.content_verified_paths.is_empty());
}

/// A committed ownership record cannot turn unreadable text into a successful verification.
/// Invalid UTF-8 is a hard drift finding and remains absent from content-verified coverage. ~keep
#[test]
fn drifted_managed_paths_reports_invalid_utf8_owned_text_as_drift() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative = "e2e/java/mvnw";
    seed_owned_output(dir.path(), relative, &[0xff, 0xfe, 0xfd]);

    let (drifted, stats) = drifted_managed_paths(
        &[gen_file_unheadered(relative, "#!/bin/sh\nexec mvn\n")],
        dir.path(),
        &e2e_owned_output_config(),
    );

    assert_eq!(drifted, vec![dir.path().join(relative).display().to_string()]);
    assert!(stats.content_verified_paths.is_empty());
}

/// Ownership does not make a create-once scaffold authoritative. A recorded seed outside every
/// rewritten E2E root remains consumer-owned after creation and must not be compared. ~keep
#[test]
fn drifted_managed_paths_preserves_create_once_exclusion_for_owned_seed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative = "Cargo.toml";
    seed_owned_output(dir.path(), relative, b"[package]\nname = \"consumer\"\n");

    let (drifted, stats) = drifted_managed_paths(
        &[gen_file_unheadered(relative, "[package]\nname = \"generated\"\n")],
        dir.path(),
        &e2e_owned_output_config(),
    );

    assert!(drifted.is_empty());
    assert!(stats.content_verified_paths.is_empty());
}

/// Production-path regression: build the real managed surface from a Java E2E configuration,
/// seed the emitted Maven wrapper as committed-owned output, mutate it, then require the same
/// `find_missing_and_frozen_generated_files` call used by `alef verify` to report drift. ~keep
#[test]
fn managed_surface_reports_a_mutated_owned_java_wrapper() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("create source directory");
    std::fs::create_dir_all(dir.path().join("fixtures")).expect("create fixtures directory");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn noop() {}\n").expect("write source");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"sample_crate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(
        dir.path().join("fixtures/smoke.json"),
        r#"{
  "id": "wrapper_smoke",
  "description": "wrapper smoke",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {},
  "assertions": []
}
"#,
    )
    .expect("write fixture");
    let config_toml = r#"
[workspace]
languages = ["java"]

[workspace.package_metadata]
repository = "https://github.com/example/sample-crate"
authors = ["Example Author <author@example.invalid>"]
license = "MIT"

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"
ffi_style = "panama"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "noop"
result_var = "result"
"#;
    let config_path = dir.path().join("alef.toml");
    std::fs::write(&config_path, config_toml).expect("write config");
    let config: crate::core::config::NewAlefConfig = toml::from_str(config_toml).expect("config parses");
    let resolved = config.resolve().expect("config resolves").remove(0);
    let _cwd = crate::test_support::CwdGuard::enter(dir.path());
    let api = crate::cli::pipeline::extract(&resolved, &config_path, false).expect("extract source API");
    let (surface, stage_failures) = collect_managed_surface(
        &[crate::core::config::Language::Java],
        &api,
        &resolved,
        &config_path,
        dir.path(),
    )
    .expect("collect real managed surface");
    assert!(
        stage_failures.is_empty(),
        "Java render must succeed without stage failures"
    );
    let wrapper = surface
        .iter()
        .find(|file| file.path == std::path::Path::new("e2e/java/mvnw"))
        .expect("real Java E2E surface must include mvnw");
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(wrapper), dir.path())
        .pop()
        .expect("one managed wrapper output");
    let full_path = dir.path().join(&wrapper.path);
    std::fs::create_dir_all(full_path.parent().expect("wrapper parent")).expect("create wrapper parent");
    std::fs::write(&full_path, rendered.content).expect("seed rendered wrapper");
    crate::cli::cache::record_scaffold_owned_path(dir.path(), &full_path).expect("record wrapper ownership");
    std::fs::write(&full_path, "tampered wrapper\n").expect("mutate wrapper");

    let found = find_missing_and_frozen_generated_files(
        &[crate::core::config::Language::Java],
        &api,
        &resolved,
        &config_path,
        dir.path(),
    )
    .expect("verify managed surface");

    assert_eq!(found.drifted, vec![full_path.display().to_string()]);
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
/// shape gets its own pair of tests below.
///
/// With poly present this runs the real formatter tier; without it the no-poly Rust fast path
/// remains covered by `format_drift::tests::drifted_managed_paths_compares_an_rs_candidate_on_the_
/// fast_path_when_poly_is_unavailable`. ~keep
#[test]
fn drifted_managed_paths_reports_a_marked_file_whose_body_no_longer_matches() {
    let _poly = crate::test_support::tool_available_with_stable_path("poly");
    let dir = tempfile::tempdir().expect("tempdir");
    let old_file = gen_file("lib.rs", "pub fn greet() -> &'static str { \"old\" }\n");
    let old_rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&old_file), dir.path());
    std::fs::write(dir.path().join("lib.rs"), &old_rendered[0].content).unwrap();
    let files = vec![gen_file("lib.rs", "pub fn greet() -> &'static str { \"new\" }\n")];

    let (drifted, _stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());

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
/// `drifted_managed_paths` itself uses, rather than hand-assembling the header/body layout, so
/// this stays correct regardless of whether `rustfmt` is on `$PATH` or which version it is: both
/// the fixture and the function under test go through the identical normalization, so there is
/// nothing for a formatter difference to disagree about. ~keep
#[test]
fn drifted_managed_paths_is_silent_once_the_marked_file_matches_the_fresh_render() {
    let _poly = crate::test_support::tool_available_with_stable_path("poly");
    let dir = tempfile::tempdir().expect("tempdir");
    let file = gen_file("lib.rs", "pub fn greet() -> &'static str { \"new\" }\n");
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&file), dir.path());
    std::fs::write(dir.path().join("lib.rs"), &rendered[0].content).unwrap();
    let files = vec![file];

    let (drifted, _stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());
    assert!(drifted.is_empty());
}

#[test]
fn drifted_managed_paths_ignores_a_declared_user_owned_file_even_when_it_still_has_an_old_marker() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("test")).expect("test directory");
    std::fs::write(
        dir.path().join("alef.toml"),
        "[workspace]\nlanguages = [\"rust\"]\n\
         [workspace.ownership]\nuser_owned = [\"test/manual.rs\"]\n\
         [[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .expect("ownership config");
    let old_file = gen_file("test/manual.rs", "pub fn manual() -> u8 { 1 }\n");
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&old_file), dir.path());
    std::fs::write(dir.path().join("test/manual.rs"), &rendered[0].content).expect("marked legacy file");
    let fresh = vec![gen_file("test/manual.rs", "pub fn manual() -> u8 { 2 }\n")];

    let (drifted, stats) = drifted_managed_paths(&fresh, dir.path(), &drift_test_config());

    assert!(
        drifted.is_empty(),
        "a declared user-owned path is outside alef's drift verdict"
    );
    assert_eq!(stats, FormatDriftStats::default());
}

/// An unmarked pre-existing file is `frozen_managed_paths`'s condition, not this one's --
/// reporting it here too would describe the same withheld write under two different headings.
#[test]
fn drifted_managed_paths_ignores_a_file_that_carries_no_marker_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("lib.rs"), "pub fn greet() {}\n").unwrap();
    let files = vec![gen_file("lib.rs", "pub fn greet() { /* changed */ }\n")];

    let (drifted, _stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());
    assert!(drifted.is_empty());
}

/// THE alef#436 SECOND-HALF FIX: a marked TOML file whose content genuinely changed must be
/// reported drifted, even though `normalize_content` has no TOML emulation at all --
/// `drifted_managed_paths` no longer relies on that prediction for a non-`.rs`/`.md` extension. It
/// instead runs a REAL `poly fmt --fix` pass over the rendered bytes (see
/// `format_drift::real_formatter_drift_with`'s module doc) and compares the result to disk, which is
/// exact rather than approximate and needs no per-language emulation to get right. Skips itself
/// when `poly` is not on the host running the suite -- the branch for that case is proven
/// host-independently via `format_drift::tests::skips_and_counts_every_candidate_when_poly_is_
/// unavailable`'s injectable seam instead.
#[test]
fn drifted_managed_paths_now_catches_a_toml_file_via_the_real_poly_fmt_pass() {
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    std::fs::write(
        dir.path().join("pyproject.toml"),
        format!("{header}\n[project]\nname = \"old\"\n"),
    )
    .unwrap();
    let files = vec![gen_file("pyproject.toml", "[project]\nname=\"new\"\n")];

    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());

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
fn drifted_managed_paths_is_silent_once_a_toml_file_matches_the_real_formatters_output() {
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let rendered_raw = "[project]\nname=\"same\"\n";
    // Build the on-disk fixture by running the SAME rendered content through the SAME real
    // `poly fmt` pass `drifted_managed_paths` itself uses, rather than hand-formatting a fixture,
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

    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());

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
fn drifted_managed_paths_reports_a_self_marking_file_whose_body_no_longer_matches() {
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

    let (drifted, _stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());

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
fn drifted_managed_paths_is_silent_once_a_self_marking_file_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let content = format!("{header}Describes `greet`, which says hi warmly.\n");
    std::fs::write(dir.path().join("reference.md"), &content).unwrap();
    let files = vec![gen_file_unheadered("reference.md", &content)];

    let (drifted, _stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());
    assert!(drifted.is_empty());
}

/// A resolved config carrying a `[crates.e2e.format]` override for `dart`, so the override tier
/// of `formatting_owner` can be exercised end to end through `drifted_managed_paths`.
fn e2e_override_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "run"

[crates.e2e.format]
dart = "cd {dir} && dart format ."
"#,
    )
    .expect("valid config");
    cfg.resolve().expect("resolvable").remove(0)
}

/// THE alef#478 REGRESSION, with its own anti-vacuity control in the same call.
///
/// Two marked files, one `drifted_managed_paths` call, opposite expected verdicts:
///
/// - `pyproject.toml` is excluded from poly by the fixture's own `poly.toml`, so nothing alef
///   runs reformats it and the raw render IS the bytes on disk. It must be SILENT. Before the
///   `formatting_owner` seam this file was reported drifted on every run forever: the check
///   staged `.alef-verify-drift-<rand>.toml` beside it, and because poly's excludes are keyed on
///   the file's NAME the sibling escaped the `**/pyproject.toml` entry, came back
///   taplo-reformatted, and no longer matched the (correctly unformatted) disk copy. In a real
///   consumer that mechanism produced 197 findings against a tree two consecutive `alef all`
///   runs left byte-identical.
/// - `lib.rs` genuinely differs from its fresh render after the real poly pass. It must be
///   REPORTED, and counted as the one formatter-backed comparison.
///
/// Asserting both in one call is deliberate. A "fix" that simply stopped reporting anything
/// would satisfy the first assertion alone, and every other test in this file that checks for
/// silence -- so silence is only evidence when something in the same tree is still heard. ~keep
#[test]
fn a_poly_excluded_path_is_silent_while_a_genuinely_drifted_one_is_still_reported() {
    let Some(_poly) = crate::test_support::tool_available_with_stable_path("poly") else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("poly.toml"),
        "[discovery]\nexclude = [\"**/pyproject.toml\"]\n",
    )
    .unwrap();

    // Deliberately NOT taplo-canonical (`name=` with no spaces): poly would reformat it if it
    // ever saw it, which is exactly what makes this a real test of the exclusion rather than of
    // a file poly happens to agree with. ~keep
    let excluded_render = "[project]\nname=\"same\"\n";
    let excluded = gen_file("pyproject.toml", excluded_render);
    let rendered = crate::cli::commands::adopt::managed_outputs(std::slice::from_ref(&excluded), dir.path());
    std::fs::write(dir.path().join("pyproject.toml"), &rendered[0].content).unwrap();

    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    std::fs::write(
        dir.path().join("lib.rs"),
        format!("{header}\npub fn greet() -> &'static str {{\n    \"old\"\n}}\n"),
    )
    .unwrap();
    let drifted_rs = gen_file("lib.rs", "pub fn greet() -> &'static str {\n    \"new\"\n}\n");

    let files = vec![excluded, drifted_rs];
    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &drift_test_config());

    assert_eq!(
        drifted,
        vec![dir.path().join("lib.rs").display().to_string()],
        "exactly the genuinely-drifted file must be reported: the poly-excluded one is not \
         reformatted by anything alef runs, so its raw render IS its disk content"
    );
    assert_eq!(
        stats.matched_render_exactly, 1,
        "only the excluded file MATCHED the render; the drifted one was settled against the render \
         too but did not match, which is why it is in `drifted` above and not counted here"
    );
    assert_eq!(
        stats.compared, 1,
        "only the Rust path poly owns may be staged; no temp copy may be staged for the excluded \
         TOML path poly never reaches"
    );
}

/// The same shape for the other tier alef cannot predict: a `[crates.e2e.format]` override
/// REPLACES the poly pass for its language (`e2e::format::format_language` returns early), so
/// alef runs opaque user shell over the file and has no honest byte-level prediction for it.
///
/// It must be COUNTED, never silently passed and never reported as drift. The counter is the
/// whole point: a verify that quietly checks nothing is this repository's dominant defect, so
/// the report has to say how many files it could not speak for. ~keep
#[test]
fn a_file_owned_by_an_e2e_format_override_is_counted_rather_than_judged() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("e2e/dart/test")).unwrap();
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    std::fs::write(
        dir.path().join("e2e/dart/test/smoke_test.dart"),
        format!("{header}\nvoid main() {{}}\n"),
    )
    .unwrap();
    // Content that genuinely differs from disk, so a silent pass cannot be mistaken for the
    // file simply being up to date.
    let files = vec![gen_file(
        "e2e/dart/test/smoke_test.dart",
        "void main() { print('x'); }\n",
    )];

    let (drifted, stats) = drifted_managed_paths(&files, dir.path(), &e2e_override_config());

    assert!(
        drifted.is_empty(),
        "alef cannot predict what `dart format` leaves on disk, so it must not claim drift: {drifted:?}"
    );
    assert_eq!(
        stats.skipped_no_faithful_prediction, 1,
        "the path must be COUNTED as unexaminable, not dropped"
    );
    assert_eq!(stats.compared, 0);
    assert_eq!(stats.matched_render_exactly, 0);
}
