//! Regression coverage for the FFI-header-refresh-vs-staging order inside
//! `complete_generated_artifacts` (`alef generate`/`alef all`'s only build step; see alef #456
//! for the original `PostBuildStep::StageFfiLibrary` fix this continues).
//!
//! Before this fix, `run_required_post_builds` (which runs `PostBuildStep::StageFfiLibrary` for
//! Go/Java/C#) ran BEFORE `ensure_ffi_header_freshness`'s conditional cbindgen-header rebuild.
//! When the on-disk header was stale, that rebuild is a real `cargo build` of the `-ffi` crate,
//! which also drops a fresh cdylib in `target/debug/` -- but staging had already run against
//! whatever was on disk *before* that build, so the artifact this run just produced was never
//! copied into the binding package's native-library directory until some LATER run happened to
//! find the header already fresh and skip the rebuild.
//!
//! These tests drive the real `complete_generated_artifacts` entry point end to end (not a
//! hand-rolled reimplementation of its internal ordering) against a fake build command standing
//! in for `cargo build`, so no real compile is required and no host toolchain assumptions are
//! made beyond `sh` (hence `unix`-only). 0.82.0 removed `[crates.build_commands.ffi]` from
//! `alef.toml`; the stand-in is now set directly on the resolved config via
//! `ResolvedCrateConfig::build_commands`, alef's own `#[cfg(test)]` hermetic-test hook (see that
//! field's doc comment).

use super::*;
use crate::core::backend::CompilePolicy;
use crate::core::config::Language;
use crate::core::config::output::{BuildCommandConfig, StringOrVec};

/// A minimal Go+FFI crate config with an explicit `[crates.ffi]` prefix/lib_name (so the
/// expected shared-library and header file names are fixed strings, not resolver defaults that
/// could silently drift) and a `ResolvedCrateConfig::build_commands` override standing in for the
/// real `cargo build` `ensure_ffi_header_freshness` would otherwise invoke.
fn go_ffi_config(build_script: &str) -> crate::core::config::ResolvedCrateConfig {
    let alef_toml = r#"
[workspace]
languages = ["go", "ffi"]

[[crates]]
name = "samplelib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "samplelib"
lib_name = "samplelib_ffi"
"#;
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(alef_toml).expect("parse fixture alef.toml");
    let mut config = alef_cfg.resolve().expect("resolve fixture crate").remove(0);
    config.build_commands.insert(
        Language::Ffi.to_string(),
        BuildCommandConfig {
            precondition: Some("true".to_string()),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(build_script.to_string())),
            build_release: None,
            timeout_seconds: None,
        },
    );
    config
}

fn host_target() -> crate::publish::platform::RustTarget {
    crate::publish::platform::host_target().expect("host target must resolve on the test machine")
}

fn ffi_crate_root(root: &std::path::Path) -> std::path::PathBuf {
    root.join("crates/samplelib-ffi")
}

/// Seed the FFI crate's generated source (one exported symbol, `samplelib_current`) and a
/// header. `header_body` controls whether `ensure_ffi_header_freshness` sees the header as
/// fresh or stale.
fn write_ffi_crate_fixture(root: &std::path::Path, header_body: &str) {
    let ffi_root = ffi_crate_root(root);
    std::fs::create_dir_all(ffi_root.join("src")).expect("create ffi src dir");
    std::fs::create_dir_all(ffi_root.join("include")).expect("create ffi include dir");
    std::fs::write(
        ffi_root.join("src/lib.rs"),
        "#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn samplelib_current() {}\n",
    )
    .expect("write ffi source");
    std::fs::write(ffi_root.join("include/samplelib.h"), header_body).expect("write ffi header");
}

/// THE FIX: a stale header forces `ensure_ffi_header_freshness` to rebuild the FFI crate, and
/// this run's own rebuild must be what gets staged into the Go package -- not whatever (nothing,
/// here, matching a fresh checkout) predates the rebuild.
#[test]
fn stale_header_rebuild_stages_the_freshly_built_debug_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let target = host_target();
    let shared_lib = target.shared_lib_name("samplelib_ffi");
    let build_script = format!(
        "mkdir -p target/debug && printf 'FRESH-DEBUG-BUILD' > target/debug/{shared_lib} && \
         printf 'void samplelib_current(void);' > crates/samplelib-ffi/include/samplelib.h"
    );
    let config = go_ffi_config(&build_script);
    // Stale: the source below exports `samplelib_current`, but the header still declares the
    // previous run's symbol -- this is what forces the rebuild. ~keep
    write_ffi_crate_fixture(&root, "void samplelib_previous(void);\n");

    let result = complete_generated_artifacts(&[Language::Go, Language::Ffi], &config, &root, CompilePolicy::Allowed);
    assert!(
        result.is_ok(),
        "a successful rebuild and staging must not fail the run: {result:?}"
    );

    let header = std::fs::read_to_string(ffi_crate_root(&root).join("include/samplelib.h")).expect("read header");
    assert!(
        header.contains("samplelib_current"),
        "the header must actually have been rebuilt by this run, or the assertion below proves \
         nothing about the reorder: {header}"
    );

    let staged_path = crate::publish::ffi_stage::staging_dir(&config, Language::Go, &target, &root)
        .expect("resolve Go staging dir")
        .join(&shared_lib);
    let staged = std::fs::read(&staged_path).unwrap_or_else(|error| {
        panic!(
            "the freshly rebuilt cdylib must have been staged to {}: {error}",
            staged_path.display()
        )
    });
    assert_eq!(
        staged, b"FRESH-DEBUG-BUILD",
        "staging must run AFTER the header refresh so it copies THIS run's own rebuild"
    );
}

/// THE SAFETY PROPERTY: a real `alef build --release` artifact already on disk must never be
/// displaced by this run's own debug-only header-refresh rebuild. `NoBuildRequested` staging
/// always prefers a release artifact over any debug artifact; this proves that preference still
/// holds once the debug artifact this run produces is strictly newer than the release one --
/// exactly the situation the reorder creates that did not previously exist.
#[test]
fn stale_header_rebuild_never_displaces_an_existing_release_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let target = host_target();
    let shared_lib = target.shared_lib_name("samplelib_ffi");
    let build_script = format!(
        "mkdir -p target/debug && printf 'FRESH-DEBUG-BUILD' > target/debug/{shared_lib} && \
         printf 'void samplelib_current(void);' > crates/samplelib-ffi/include/samplelib.h"
    );
    let config = go_ffi_config(&build_script);
    write_ffi_crate_fixture(&root, "void samplelib_previous(void);\n");

    let release_dir = root.join("target/release");
    std::fs::create_dir_all(&release_dir).expect("create target/release");
    std::fs::write(release_dir.join(&shared_lib), b"REAL-RELEASE-BUILD").expect("seed release artifact");

    let result = complete_generated_artifacts(&[Language::Go, Language::Ffi], &config, &root, CompilePolicy::Allowed);
    assert!(result.is_ok(), "rebuild and staging must succeed: {result:?}");

    // Positive control: the debug rebuild genuinely ran this run -- otherwise "the release
    // artifact was staged" would be true for the trivial reason that nothing else exists. ~keep
    let debug_artifact_path = root.join("target/debug").join(&shared_lib);
    let debug_artifact = std::fs::read(&debug_artifact_path).expect("the refresh must have built a debug artifact");
    assert_eq!(
        debug_artifact, b"FRESH-DEBUG-BUILD",
        "sanity: the header refresh must have actually run"
    );

    let staged_path = crate::publish::ffi_stage::staging_dir(&config, Language::Go, &target, &root)
        .expect("resolve Go staging dir")
        .join(&shared_lib);
    let staged = std::fs::read(&staged_path).expect("release artifact must have been staged");
    assert_eq!(
        staged, b"REAL-RELEASE-BUILD",
        "a release build already on disk must never be displaced by this run's debug-only \
         refresh, even though the debug artifact is strictly newer"
    );
}

/// Control: when the header is already fresh, the reorder changes nothing observable -- no
/// rebuild happens, and staging picks up whatever is already on disk exactly as it did before
/// this fix. The build override is a tripwire (`exit 99`) that fails the whole run if it is ever
/// invoked, so an accidental rebuild on the fresh path would fail this test, not silently pass.
#[test]
fn fresh_header_never_triggers_a_rebuild_and_stages_the_existing_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let config = go_ffi_config("exit 99");
    write_ffi_crate_fixture(&root, "void samplelib_current(void);\n");

    let target = host_target();
    let shared_lib = target.shared_lib_name("samplelib_ffi");
    let release_dir = root.join("target/release");
    std::fs::create_dir_all(&release_dir).expect("create target/release");
    std::fs::write(release_dir.join(&shared_lib), b"EXISTING-RELEASE-BUILD").expect("seed release artifact");

    let result = complete_generated_artifacts(&[Language::Go, Language::Ffi], &config, &root, CompilePolicy::Allowed);
    assert!(
        result.is_ok(),
        "a fresh header must never invoke the 'exit 99' tripwire build: {result:?}"
    );
    assert!(
        !root.join("target/debug").exists(),
        "no rebuild should have happened for an already-fresh header"
    );

    let staged_path = crate::publish::ffi_stage::staging_dir(&config, Language::Go, &target, &root)
        .expect("resolve Go staging dir")
        .join(&shared_lib);
    let staged = std::fs::read(&staged_path).expect("the existing release artifact must have been staged");
    assert_eq!(staged, b"EXISTING-RELEASE-BUILD");
}

/// `complete_generated_artifacts` only returns one `Result`, so a header failure must not mask a
/// simultaneous post-build failure (or vice versa) -- see that function's own doc for why a bare
/// `header_result.and(post_build_result)` would drop the post-build side. These exercise
/// `combine_artifact_results` directly rather than reconstructing a double-failure scenario
/// through the full pipeline.
#[test]
fn combine_artifact_results_passes_through_a_single_header_failure_unchanged() {
    let error =
        combine_artifact_results(Err(anyhow::anyhow!("header drift")), Ok(())).expect_err("must still be an error");
    assert_eq!(format!("{error:#}"), "header drift");
}

#[test]
fn combine_artifact_results_passes_through_a_single_post_build_failure_unchanged() {
    let error =
        combine_artifact_results(Ok(()), Err(anyhow::anyhow!("post-build broke"))).expect_err("must still be an error");
    assert_eq!(format!("{error:#}"), "post-build broke");
}

#[test]
fn combine_artifact_results_names_both_errors_on_a_double_failure() {
    let error = combine_artifact_results(
        Err(anyhow::anyhow!("header drift")),
        Err(anyhow::anyhow!("post-build broke")),
    )
    .expect_err("a double failure must still be an error");
    let message = format!("{error:#}");
    assert!(
        message.contains("header drift"),
        "must name the header failure: {message}"
    );
    assert!(
        message.contains("post-build broke"),
        "must name the post-build failure: {message}"
    );
}
