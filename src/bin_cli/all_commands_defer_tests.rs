//! Regression coverage for task #186's "all-or-nothing abort" defect: a single rejected
//! fixture, or a single crate's post-build failure, must never deny every other `alef all`
//! stage -- or every other crate -- its regeneration. Both tests drive the real `handle` entry
//! point (not a hand-built stub of the orchestration) because the defect lived in the CALLER's
//! `?`/`return Err`, not in any function these tests could exercise directly.

use super::handle;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::CwdGuard;

fn all_command() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
        skip_compile: false,
    }
}

fn all_command_skipping_compile() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
        skip_compile: true,
    }
}

fn expect_err(result: anyhow::Result<Option<Commands>>, message: &str) -> anyhow::Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{message}"),
    }
}

// ---------------------------------------------------------------------------
// Pre-flight snippet-coverage precondition must not abort the main loop
// ---------------------------------------------------------------------------

const PREFLIGHT_FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";

const PREFLIGHT_FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"preflightlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

const PREFLIGHT_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "preflightlib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["rust"]

[crates.e2e.call]
function = "greet"
module = "preflightlib"
result_var = "result"

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"

[crates.e2e.snippets]
output = "docs/snippets"
"#;

const PREFLIGHT_FIXTURE_DOCUMENTED: &str = r#"{
  "id": "greet_basic",
  "description": "Greets someone",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Ada"},
  "assertions": [{"type": "not_error"}],
  "docs": {"topic": "guides"}
}
"#;

/// No `"docs"` key at all -- `crate::e2e::snippets::generate_snippet_report` records this as
/// `coverage.missing` ("fixture has no documentation metadata") rather than rendering a
/// snippet. One such gap is enough to fail `ensure_snippet_coverage_complete`, which is the
/// same code path a rejected mock-harness-guard fixture reaches (both flow through
/// `evaluate_snippet_coverage`/`ensure_fresh_snippet_coverage_complete` in `handle`'s pre-flight
/// loop) -- this fixture reproduces the identical orchestration defect with a trigger that
/// needs no toolchain, no extension, and no fixture-generator internals to set up. ~keep
const PREFLIGHT_FIXTURE_UNDOCUMENTED: &str = r#"{
  "id": "greet_no_docs",
  "description": "Greets someone else",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Grace"},
  "assertions": [{"type": "not_error"}]
}
"#;

fn write_preflight_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), PREFLIGHT_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), PREFLIGHT_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/greet_basic.json"), PREFLIGHT_FIXTURE_DOCUMENTED)
        .expect("write documented fixture");
    std::fs::write(root.join("fixtures/greet_no_docs.json"), PREFLIGHT_FIXTURE_UNDOCUMENTED)
        .expect("write undocumented fixture");
    std::fs::write(root.join("alef.toml"), PREFLIGHT_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// THE DEFECT: before this fix, a coverage gap discovered by `handle`'s pre-flight loop
/// (`evaluate_snippet_coverage`/`ensure_fresh_snippet_coverage_complete`, run once per crate
/// before the main generation loop) propagated with `?` immediately -- aborting the run before
/// bindings, e2e suites, READMEs or docs for ANY crate were ever generated. A consumer repo hit
/// this for real: a single rejected fixture out of 264 meant `alef all` wrote nothing at all,
/// and every stage had to be invoked by hand to get a regen (task #186). ~keep
#[test]
fn a_preflight_coverage_gap_does_not_abort_the_main_generation_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_preflight_fixture_workspace(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_err(
        handle(all_command(), &context),
        "a real coverage gap must still fail the run -- writing everything else must not turn \
         this into a healthy exit code",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("snippet coverage precondition"),
        "the failure must name the pre-flight stage that found the gap: {message}"
    );
    assert!(
        message.contains("greet_no_docs"),
        "the failure must carry the underlying coverage-gap diagnostic verbatim: {message}"
    );

    let bindings = root.join("packages/python/preflightlib/__init__.py");
    assert!(
        bindings.is_file(),
        "python bindings must still have been generated despite the pre-flight coverage gap: {} is missing",
        bindings.display()
    );
    let rust_cargo_toml = root.join("e2e").join("rust").join("Cargo.toml");
    assert!(
        rust_cargo_toml.is_file(),
        "the e2e stage must still have run and written its rust suite despite the pre-flight \
         coverage gap: {} is missing",
        rust_cargo_toml.display()
    );
    let readme = root.join("packages/python/README.md");
    assert!(
        readme.is_file(),
        "the README stage must still have run despite the pre-flight coverage gap: {} is missing",
        readme.display()
    );
}

// ---------------------------------------------------------------------------
// A crate's post-build failure must not abort its own remaining stages
// ---------------------------------------------------------------------------

const POST_BUILD_FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";

const POST_BUILD_FIXTURE_CARGO_TOML: &str =
    "[package]\nname = \"postbuildlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `languages = ["ffi"]` plus a `[crates.extra_dependencies]` entry naming a local path that does
/// not exist. `complete_generated_artifacts` reaches
/// `crate::cli::pipeline::ensure_ffi_header_freshness` for any FFI-configured crate, the
/// generated header does not exist yet on a fresh tree, and the refresh closure it then calls is
/// `build_with_environment(.., [Ffi], ..)` -- a real `cargo build --manifest-path` of the
/// scaffolded FFI crate, which now declares a dependency on a directory that was never created.
/// Cargo fails while loading that dependency's manifest, before resolving anything else, so this
/// costs ~0.5s: no real compilation, no network, no dependency on which toolchains happen to be
/// installed beyond `cargo` itself. The failure therefore originates inside
/// `complete_generated_artifacts`, the only thing `stage_failures.record("[<crate>] post-build
/// processing", ..)` wraps.
///
/// This used to configure `[crates.build_commands.ffi] build = "exit 42"` instead -- 0.82.0
/// removed that table from the schema entirely (alef now owns the FFI build command), so a
/// deterministic failure has to come from cargo's own real, fast-failing error paths rather than
/// a hand-picked exit code. Before that override existed at all, this test relied on
/// `cargo build -p postbuildlib-ffi` failing with "package ID specification ... did not match any
/// packages", because the generated crate is not a member of any workspace -- that was never a
/// property worth depending on either, since it was alef's own defect (a `-p` spec resolves only
/// for a workspace member) and a fix to it silently turned this into a real, 7-second, networked
/// compile. A missing local path dependency is deterministic and offline for the same reason a
/// missing package ID was not: cargo cannot resolve it without ever touching the network or the
/// registry index, on any machine, regardless of what else the fix history above changes about
/// how the FFI crate gets built. ~keep
const POST_BUILD_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "postbuildlib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.extra_dependencies]
alef-test-nonexistent-path-dep = { path = "does-not-exist" }

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["rust"]

[crates.e2e.call]
function = "greet"
module = "postbuildlib"
result_var = "result"

[[crates.e2e.call.args]]
name = "name"
field = "input.name"
type = "string"
"#;

const POST_BUILD_FIXTURE_JSON: &str = r#"{
  "id": "greet_basic",
  "description": "Greets someone",
  "category": "smoke",
  "tags": ["smoke"],
  "input": {"name": "Ada"},
  "assertions": [{"type": "not_error"}]
}
"#;

fn write_post_build_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::create_dir_all(root.join("fixtures")).expect("create fixture fixtures directory");
    std::fs::write(root.join("src/lib.rs"), POST_BUILD_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), POST_BUILD_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("fixtures/greet_basic.json"), POST_BUILD_FIXTURE_JSON).expect("write fixture json");
    std::fs::write(root.join("alef.toml"), POST_BUILD_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// THE DEFECT: before this fix, `complete_generated_artifacts`'s `Err` propagated via
/// `return Err(error)` straight out of `handle` -- so a single crate's post-build failure
/// (real-world: a Dart `flutter_rust_bridge_codegen` break) meant stubs, e2e, READMEs and docs
/// for THAT crate never ran, and neither did any crate listed after it. A consumer repo hit
/// this for real (task #186): the run hard-stopped after the generate stage, with
/// e2e/test-apps files left at pre-session mtimes. This fixture reproduces the same shape
/// against alef's own tests. It holds `SKIP_COMMANDS_LOCK` for its whole duration so a
/// concurrent test cannot rewrite the process-global `ALEF_SKIP_COMMANDS` mid-run and change
/// which of this run's subprocesses execute (see `SkipCommandsGuard`'s doc); the failure this
/// test asserts on no longer depends on that var either way, since the real `cargo build` the
/// refresh closure runs is dispatched through `run_command_captured_with_env`, which that escape
/// hatch does not gate -- only `PostBuildStep::RunCommand` consults it.
///
/// This run has exactly one recorded stage failure, so `StageFailures::into_result` returns the
/// original `anyhow::Error` unchanged rather than wrapping it in a `"[crate] post-build
/// processing: ..."` summary line (see that type's own unit tests) -- the `"[postbuildlib] post-
/// build processing failed"` label is only guaranteed to appear in the real-time log, which
/// `#[traced_test]` captures below. ~keep
#[tracing_test::traced_test]
#[test]
fn a_crate_post_build_failure_does_not_abort_its_own_remaining_stages() {
    let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_post_build_fixture_workspace(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let error = expect_err(
        handle(all_command(), &context),
        "a genuine post-build failure must still fail the run -- writing everything else must \
         not turn this into a healthy exit code",
    );
    let message = format!("{error:#}");
    assert!(
        message.contains("failed to refresh the generated FFI header"),
        "the failure must be the one `complete_generated_artifacts` itself raises -- that is the \
         only thing the deferred `[<crate>] post-build processing` label wraps, so an error from \
         any other stage would satisfy every other assertion here while proving nothing: {message}"
    );
    assert!(
        message.contains("failed to load source for dependency"),
        "the failure must carry the underlying cargo diagnostic verbatim: {message}"
    );
    assert!(
        logs_contain("[postbuildlib] post-build processing"),
        "the post-build stage must still be named in the real-time log, even though the deferred \
         error's own text is returned unchanged"
    );

    let rust_cargo_toml = root.join("e2e").join("rust").join("Cargo.toml");
    assert!(
        rust_cargo_toml.is_file(),
        "the e2e stage must still have run and written its rust suite despite the post-build \
         failure: {} is missing",
        rust_cargo_toml.display()
    );
    let readme = root.join("crates/postbuildlib-ffi/README.md");
    assert!(
        readme.is_file(),
        "the README stage must still have run despite the post-build failure: {} is missing",
        readme.display()
    );
}

/// The generation-only mode, proved end-to-end against the same fixture the test above uses as
/// its positive control -- so the pair together shows the flag changes exactly one thing.
///
/// The missing local path dependency in `POST_BUILD_FIXTURE_ALEF_TOML` is what
/// `ensure_ffi_header_freshness`'s refresh closure's real `cargo build` fails on, and the test
/// above asserts that `alef all` without this flag reaches it and fails on it. With the flag,
/// `complete_generated_artifacts` must not invoke the refresh at all, so cargo's "failed to load
/// source for dependency" can never appear in this run: the fixture's own configuration is the
/// tripwire, and it costs no compilation, no network, and no host toolchain. The header is still
/// *checked* -- only never rebuilt -- so a stale one would still fail; this fresh tree has no
/// header at all, which `check_ffi_header_freshness` reports as a warning and allows. ~keep
#[tracing_test::traced_test]
#[test]
fn skip_compile_writes_source_without_invoking_the_ffi_build() {
    let _skip_guard = crate::test_support::SkipCommandsGuard::set("");
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_post_build_fixture_workspace(&root);
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    let message = match handle(all_command_skipping_compile(), &context) {
        Ok(_) => String::new(),
        Err(error) => format!("{error:#}"),
    };

    assert!(
        !message.contains("failed to load source for dependency"),
        "--skip-compile must not reach the FFI header refresh -- the only thing in this fixture \
         that can trigger cargo's dependency-resolution failure: {message}"
    );
    assert!(
        logs_contain("not building the FFI crate to refresh its cbindgen header"),
        "the skipped rebuild must be announced, so an operator can tell a skipped refresh from a \
         header that was already fresh"
    );

    let ffi_source = root.join("crates/postbuildlib-ffi/src/lib.rs");
    assert!(
        ffi_source.is_file(),
        "generation must still write its source with the compile skipped: {} is missing",
        ffi_source.display()
    );
}
