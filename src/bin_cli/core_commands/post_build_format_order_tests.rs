//! Regression coverage for the `alef generate` post-build/format ordering defect: a post-build
//! step that writes straight to disk, bypassing `write_files_report` (Swift's
//! `MaterializeSwiftBridge`, which materializes `RustBridgeC.h` from swift-bridge's own build
//! output), used to run AFTER the only formatting pass `Commands::Generate` performed. The file
//! this run shipped was therefore stamped (`finalize_hashes`) over whatever raw bytes the
//! post-build tool produced, never run through `poly fmt` at all.
//!
//! That is not merely cosmetic. `poly`'s built-in "hash-stamped generated file" skip is
//! content-pattern-based (see `core::hash::POLY_GENERATED_SCAN_LINES`): once a file carries a
//! well-formed `alef:hash:<hex>` line in its leading lines, poly leaves the file untouched on
//! every later invocation, canonical or not -- verified directly against the `poly` binary this
//! suite runs against, both in isolation and through this exact fixture. So a file stamped BEFORE
//! ever being formatted ships non-canonical forever: poly's own protection is what prevents any
//! later pass -- another `alef all` run, or a standalone `poly fmt --fix .` a consumer runs
//! before committing -- from ever bringing it into line, since poly cannot tell "stamped and
//! canonical" apart from "stamped and never formatted".
//!
//! An earlier revision of this comment claimed `alef all` "already runs post-build before its
//! whole-tree format pass and does not have this defect". The first half was true and the second
//! half was false, and asserting it in prose is how it went unchecked: `alef all` ran ten
//! `finalize_hashes(&current_gen_paths, ..)` checkpoints -- one per write phase, bindings through
//! docs -- all of them BEFORE its single format pass, so poly's skip locked out every file the
//! command emitted, not just post-build-owned ones. Measured on a neutral eight-language fixture:
//! 21 of the 93 files `alef all` emitted were files `poly fmt` would have rewritten.
//!
//! The `alef all` coverage below is therefore an executable version of that claim rather than a
//! restatement of it. It targets the **scaffold** phase specifically (`packages/python/
//! pyproject.toml`), because scaffold output is written by a different writer
//! (`write_scaffold_files_report`, with its create-once branch) than the binding phases and was
//! the phase the prose claim was most wrong about.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::SkipCommandsGuard;
use std::path::{Path, PathBuf};

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
"#;

/// Same fixture as [`write_fixture_workspace`], but with `[workspace.scaffold]` metadata --
/// `alef all` (unlike `alef generate`) also runs README generation, which needs it; the plain
/// `alef generate`-only fixture above deliberately omits it to keep that harness's tree small
/// (mirrors [`ALL_FIXTURE_ALEF_TOML`] below, which the existing `alef all` scaffold-phase tests
/// already rely on for the same reason). ~keep
const ALL_SWIFT_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["swift"]

[workspace.scaffold]
repository = "https://example.invalid/sample/sample-lib"
license = "MIT"
authors = ["Sample Author <sample@example.invalid>"]
description = "Sample fixture library"

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
"#;

fn write_all_swift_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), ALL_SWIFT_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
    std::fs::write(root.join("Cargo.lock"), "").expect("write fixture Cargo.lock");
    // See `write_fixture_workspace`'s matching comment above it: pre-creates
    // `packages/swift/Sources` so `emit_swift_bridge_files`'s `package_root` resolution finds the
    // real tree on its first candidate instead of the buggy repo-root fallback for a project with
    // no `Sources/` directory yet.
    std::fs::create_dir_all(root.join("packages/swift/Sources")).expect("create packages/swift/Sources");
}

/// The fixed binding crate name `MaterializeSwiftBridge` derives from the fixture's crate name
/// (`format!("{}-swift", config.name)`), pinned here as a constant instead of recomputed inline
/// so the fake `target/` layout below and the assertions after `alef generate` runs can never
/// drift against each other.
const BINDING_CRATE_NAME: &str = "test-lib-swift";

/// Non-canonical (2-space) struct-field indentation, deliberately NOT matching poly's own C
/// style. Measured directly against the `poly` binary this suite runs against: an otherwise
/// identical file with no `alef:hash:` line gets exactly these two `typedef struct` blocks
/// reindented to 4 spaces by `poly fmt --fix`. Using content poly would genuinely rewrite -- not
/// content that happens to already be canonical -- is what makes the fixed-point assertions below
/// load-bearing rather than vacuously true either way.
const NON_CANONICAL_CORE_H: &str = "typedef struct RustStr {\n  uint8_t *const start;\n  uintptr_t len;\n} RustStr;\n";
const NON_CANONICAL_CRATE_H: &str = "typedef struct DemoOpaque {\n  void *const ptr;\n} DemoOpaque;\n";

fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
    // `find_swift_bridge_out_dir` (`backends::swift::gen_bindings::bridge_artifacts`) walks
    // ancestors of the process cwd looking for a `Cargo.lock` to anchor its `target/` search --
    // content is never read, only existence, so an empty stub is sufficient.
    std::fs::write(root.join("Cargo.lock"), "").expect("write fixture Cargo.lock");
    // Works around a SEPARATE, pre-existing defect this fixture is not testing: both
    // `emit_swift_bridge_files` call sites (`gen_bindings/mod.rs`'s in-process call and
    // `MaterializeSwiftBridge`'s post-build call) resolve `package_root` via
    // `PathBuf::from(base_dir).ancestors().find(|p| p.join("Sources").is_dir())`, falling back to
    // `.parent().and_then(|p| p.parent())` when nothing matches. For `base_dir =
    // "packages/swift"` that fallback is `Some("packages").and_then(|p| p.parent())` --
    // `Path::new("packages").parent()` is `Some("")`, NOT `None` -- so the chain resolves to
    // `PathBuf::from("")` (the repo root) on a project with no `packages/swift/Sources` yet,
    // never reaching the final `unwrap_or_else(|| PathBuf::from("packages/swift"))`. On a
    // completely fresh project (this fixture, before this workaround) that misplaces the whole
    // `Sources/` tree at the repo root instead of under `packages/swift/`. Pre-creating an empty
    // `packages/swift/Sources` directory makes the primary `ancestors().find` branch match on its
    // very first candidate, sidestepping the buggy fallback entirely -- this is a workaround for
    // this fixture, not a fix; the underlying defect is out of scope for the post-build/format
    // ordering bug this module tests and is unrelated to it. ~keep
    std::fs::create_dir_all(root.join("packages/swift/Sources")).expect("create packages/swift/Sources");
}

/// Pre-seed `target/release/build/<binding_crate_name>-<hash>/out/` with the swift-bridge build
/// output `MaterializeSwiftBridge` would otherwise only get from a real `cargo build` invoking
/// the `swift-bridge-build` crate's build script -- a genuine build needs network-fetched
/// dependencies and a full swift-bridge toolchain neither available nor desirable in a unit test.
/// `find_swift_bridge_out_dir` only checks for a directory name prefix and a
/// `SwiftBridgeCore.swift` marker file inside `out/`, so a hand-built directory with the right
/// shape is indistinguishable from a real one to the code under test.
fn seed_fake_swift_bridge_build_output(root: &Path) {
    seed_fake_swift_bridge_build_output_in(&root.join("target"));
}

/// [`seed_fake_swift_bridge_build_output`] against an arbitrary cargo target root, so a fixture
/// can plant the build output somewhere the project directory does not contain -- which is the
/// whole point of the alef #477 coverage below.
fn seed_fake_swift_bridge_build_output_in(target_root: &Path) {
    let out_dir = target_root
        .join("release/build")
        .join(format!("{BINDING_CRATE_NAME}-fakehash/out"));
    let binding_out_dir = out_dir.join(BINDING_CRATE_NAME);
    std::fs::create_dir_all(&binding_out_dir).expect("create fake swift-bridge out dir");
    std::fs::write(out_dir.join("SwiftBridgeCore.swift"), "import Foundation\n")
        .expect("write fake SwiftBridgeCore.swift");
    std::fs::write(out_dir.join("SwiftBridgeCore.h"), NON_CANONICAL_CORE_H).expect("write fake SwiftBridgeCore.h");
    std::fs::write(
        binding_out_dir.join(format!("{BINDING_CRATE_NAME}.swift")),
        "import Foundation\n",
    )
    .expect("write fake crate swift file");
    std::fs::write(
        binding_out_dir.join(format!("{BINDING_CRATE_NAME}.h")),
        NON_CANONICAL_CRATE_H,
    )
    .expect("write fake crate h file");
}

fn header_path(root: &Path) -> std::path::PathBuf {
    root.join("packages/swift/Sources/RustBridgeC/RustBridgeC.h")
}

/// Run `alef generate --lang swift` against `root`. `ALEF_SKIP_COMMANDS=cargo` skips the
/// `RunCommand` step that would otherwise `cargo build` a real swift-bridge crate before
/// `MaterializeSwiftBridge` runs -- `run_run_command` treats a listed command as a deterministic,
/// non-fatal `Ok(false)` skip (see that function's doc), not an error, so `MaterializeSwiftBridge`
/// still runs immediately after and finds the fake `out/` directory `seed_fake_swift_bridge_build_output`
/// planted. Guarded by `SkipCommandsGuard` because `ALEF_SKIP_COMMANDS` is process-global. ~keep
fn run_generate(root: &Path) {
    try_generate(root).expect("alef generate must succeed against the fixture");
}

/// [`run_generate`] without the success assertion, for the cases that must fail.
///
/// `ClearedCargoTargetDirGuard` makes the fixture hermetic: since alef #477,
/// `find_swift_bridge_out_dir` asks `cargo metadata` where build output goes, and cargo honours
/// an inherited `CARGO_TARGET_DIR` -- so a suite run with that variable exported would send every
/// fixture here looking in the test runner's own target directory instead of the tempdir where
/// it planted its fake `out/`. Clearing it leaves cargo to resolve the fixture's own tree, which
/// is what `point_cargo_at_an_external_target_dir` then redirects via `.cargo/config.toml`. ~keep
fn try_generate(root: &Path) -> anyhow::Result<()> {
    let _skip_guard = SkipCommandsGuard::set("cargo");
    let _target_dir_guard = crate::test_support::ClearedCargoTargetDirGuard::clear();
    let _cwd = crate::test_support::CwdGuard::enter(root);
    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: Some(vec!["swift".to_owned()]),
            clean: false,
            skip_frb: false,
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .map(|_| ())
}

/// The regression test itself: `alef generate` must leave the post-build-owned header both
/// poly-canonical AND correctly stamped in the SAME pass -- no separate `poly fmt --fix .` step
/// in between. Before the fix (post-build running after the only format pass), this failed on
/// the `poly_fmt_check_reports_the_file_clean` assertion: the header shipped with the raw
/// 2-space `NON_CANONICAL_*` indentation, `poly fmt --check` reported it as needing a rewrite
/// (not skipped-as-stamped, since a freshly-generated file with a fresh but never-formatted body
/// still gets a fresh, well-formed hash line from `finalize_hashes` -- the bug is that the body
/// under that hash was never canonical to begin with). After the fix, post-build runs before the
/// format pass, so the body the hash covers IS the canonical one, and `poly fmt --check` finds
/// nothing to do. ~keep
#[test]
fn generate_formats_post_build_output_before_stamping_it() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    seed_fake_swift_bridge_build_output(&root);

    run_generate(&root);

    let header = header_path(&root);
    let content = std::fs::read_to_string(&header).expect("post-build must have materialized RustBridgeC.h");
    assert!(
        content.contains("alef:hash:"),
        "sanity: the header must actually be alef-stamped, or every assertion below is vacuous: {content}"
    );
    assert!(
        content.contains("typedef struct RustStr {\n    uint8_t"),
        "the header's body must be poly-canonical (4-space struct fields) by the time it ships -- \
         got:\n{content}"
    );

    // The literal regression assertion: `poly fmt --check` must find nothing to do. Before the
    // fix this failed here -- the shipped header still carried the RAW 2-space body under a
    // hash that matched it (self-consistent, but never canonical), and poly does not
    // distinguish "correctly stamped" from "stamped over non-canonical bytes".
    //
    // `--fix-generated` is load-bearing, not decoration: the sanity assertion above proves the
    // header is stamped, and poly refuses to even inspect a hash-stamped file under a plain
    // `--check` -- measured directly against the `poly` binary this suite runs against, a
    // stamped file with deliberately non-canonical bytes still reports `--check` clean and exits
    // 0. Without the flag this call would tell us nothing about `RustBridgeC.h` at all; the
    // `content.contains(...)` assertion above would be the ONLY thing standing between this test
    // and a silent pass on the pre-fix ordering. With the flag, poly inspects the stamped body
    // regardless of the hash line, so this assertion is a genuine second check on the same
    // property, not a check that cannot fail. ~keep
    let check = std::process::Command::new("poly")
        .args(["fmt", "--check", "--fix-generated", "."])
        .current_dir(&root)
        .output()
        .expect("run poly fmt --check");
    let check_output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        check.status.success(),
        "`poly fmt --check --fix-generated .` must report the tree clean immediately after `alef \
         generate` -- a post-build-owned file was formatted after being stamped, not before. \
         Output:\n{check_output}"
    );
}

/// The fixed-point property required alongside the regression test above: running `alef
/// generate` a second time, with nothing about the sources or fixture changed, must leave the
/// post-build-owned header byte-for-byte unchanged. Before the fix this still held for a
/// SINGLE run (the file was self-consistently stamped even though never formatted), so this test
/// alone would not have caught the defect -- it exists to prove the fix does not merely paper
/// over the first run while leaving a later reformat (a whole-tree `alef all`, or a standalone
/// `poly fmt --fix .`) able to invalidate the stamp again. ~keep
#[test]
fn generate_reaches_a_fixed_point_on_post_build_output_across_repeat_runs() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    seed_fake_swift_bridge_build_output(&root);

    run_generate(&root);
    let header = header_path(&root);
    let first_pass = std::fs::read_to_string(&header).expect("first alef generate must materialize the header");

    run_generate(&root);
    let second_pass = std::fs::read_to_string(&header).expect("second alef generate must leave the header in place");

    assert_eq!(
        first_pass, second_pass,
        "a second `alef generate` run over an unchanged tree must leave the post-build-owned \
         header byte-for-byte unchanged -- any diff here means the pipeline has not reached a \
         fixed point"
    );

    // Run once more through poly directly, outside alef entirely -- the exact "standalone `poly
    // fmt --fix .` before committing" trigger the observed incident described. A file that
    // reached a genuine fixed point on the first `alef generate` run must survive this
    // untouched too.
    let fix = std::process::Command::new("poly")
        .args(["fmt", "--fix", "."])
        .current_dir(&root)
        .output()
        .expect("run poly fmt --fix");
    assert!(
        fix.status.success(),
        "poly fmt --fix must exit 0 on an already-canonical tree"
    );
    let after_standalone_fmt = std::fs::read_to_string(&header).expect("header must still exist");
    assert_eq!(
        second_pass, after_standalone_fmt,
        "a standalone `poly fmt --fix .` run after `alef generate` must not change a byte of an \
         already-canonical, correctly-stamped post-build-owned file"
    );
}

/// The `alef all` scaffold-phase fixture: one language whose scaffold output poly has a genuine
/// opinion about. Python is chosen because `packages/python/pyproject.toml` is scaffold-phase
/// output (not a binding), and `.toml` is handled by poly's bundled taplo engine, which is
/// available wherever poly itself is -- no extra host toolchain has to be installed for the
/// assertions below to mean anything.
const ALL_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[workspace.scaffold]
repository = "https://example.invalid/sample/sample-lib"
license = "MIT"
authors = ["Sample Author <sample@example.invalid>"]
description = "Sample fixture library"

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
"#;

const ALL_FIXTURE_CARGO_TOML: &str = "[package]\nname = \"sample-lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

/// The scaffold-phase path both `alef all` tests below are written against.
const SCAFFOLD_TARGET: &str = "packages/python/pyproject.toml";

/// The probe file name handed to [`poly_would_reformat`] for [`SCAFFOLD_TARGET`]: poly routes by
/// extension, so the probe must keep the target's own name to reach the same engine.
const SCAFFOLD_TARGET_FILE_NAME: &str = "pyproject.toml";

fn write_all_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), ALL_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), ALL_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
    std::fs::write(root.join("Cargo.lock"), "").expect("write fixture Cargo.lock");
}

/// Run `alef all` against `root`. `ALEF_SKIP_COMMANDS=cargo` for the same reason as
/// [`run_generate`]: no real toolchain build is available or wanted here.
///
/// `ClearedCargoTargetDirGuard` is held for the same reason as in [`try_generate`].
///
/// `ALEF_SKIP_COMMANDS` does not reach `alef all`'s own full-regen `converge_full_regen`
/// residuals (`cargo fmt --all`, `cargo sort -n -w`), which run for real whenever `root` has a
/// Cargo.toml -- this fixture does. `RealCargoGuard` serializes that against every other test in
/// the crate that also spawns a real `cargo` subprocess, closing the `Blocking waiting for file
/// lock on package cache` flake this fixture measured under parallel load. See
/// `test_support::REAL_CARGO_LOCK`'s doc. ~keep
fn run_all(root: &Path) {
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let _skip_guard = SkipCommandsGuard::set("cargo");
    let _target_dir_guard = crate::test_support::ClearedCargoTargetDirGuard::clear();
    let _cwd = crate::test_support::CwdGuard::enter(root);
    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    crate::bin_cli::all_commands::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            skip_frb: true,
            strict: false,
            skip_snippet_validation: true,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed against the fixture");
}

/// Whether `poly` would rewrite `content` if it were saved under `file_name`.
///
/// `--fix-generated` is passed deliberately. Without it, poly refuses to inspect a file carrying
/// an `alef:hash:` line at all and reports the tree clean -- measured against the `poly` binary
/// this suite runs against: `skipped <path>: hash-stamped generated file (pass --fix-generated to
/// format)`, exit 0, under `--check` just as much as under `--fix`. A probe that inherited that
/// skip would answer "already canonical" for every stamped input regardless of its body, i.e. it
/// would be a check that cannot fail -- which is the exact defect shape this module exists for.
/// With the flag, poly inspects the bytes either way and the answer is about the content. ~keep
///
/// Spawned via [`crate::test_support::spawn_from_stable_dir`], not a bare `Command::new`, even
/// though `path` is already absolute: `poly` resolves its own config/repo-root context from the
/// process's ambient current directory, not from its argument paths (verified directly against
/// the `poly` binary this suite runs against -- `poly doctor`'s reported cache path changes with
/// cwd alone). `cargo test --lib` runs every test as a thread in one process sharing one cwd, and
/// several tests in this crate (including this module's own [`run_generate`]/[`run_generate_python`])
/// enter and restore that cwd via `CwdGuard` around a tempdir they then delete. An unpinned spawn
/// here inherits whatever the shared cwd happens to be at that instant -- including another
/// test's already-deleted tempdir -- so `poly` would resolve config/cache context against an
/// unrelated (or gone) directory instead of behaving as a self-contained probe of `content`. That
/// is exactly the failure `--test-threads`-parallel `cargo test --lib` runs reproduced: this probe
/// answering "would reformat" for bytes that are canonical when checked from a stable cwd. Every
/// other subprocess this module spawns already pins `.current_dir(&root)` explicitly; this one
/// must pin it too. ~keep
fn poly_would_reformat(file_name: &str, content: &str) -> bool {
    let probe = tempfile::tempdir().expect("probe tempdir");
    let path = probe.path().join(file_name);
    std::fs::write(&path, content).expect("write probe file");
    let output = crate::test_support::spawn_from_stable_dir("poly")
        .args(["fmt", "--check", "--fix-generated", "--no-cache"])
        .arg(&path)
        .output()
        .expect("run poly fmt --check on the probe file");
    !output.status.success()
}

/// The scaffold generator's own in-memory bytes for `relative_path`, before any writer or
/// formatter has touched them.
///
/// This is the anti-vacuity control both tests below open with: if the generator's raw output for
/// the target path were already poly-canonical, "the shipped file is canonical" would hold no
/// matter when the stamp was applied, and the regression test would pass against the very
/// ordering it exists to reject. ~keep
fn raw_scaffold_content(root: &Path, relative_path: &str) -> String {
    let _cwd = crate::test_support::CwdGuard::enter(root);
    let config_path = root.join("alef.toml");
    let (_, resolved) = crate::bin_cli::helpers::load_config(&config_path).expect("load fixture alef.toml");
    let config = resolved.first().expect("fixture declares exactly one crate");
    let languages = crate::bin_cli::helpers::resolve_languages(config, None).expect("resolve fixture languages");
    let api = crate::cli::pipeline::extract(config, &config_path, true).expect("extract fixture API surface");
    let files = crate::cli::pipeline::scaffold(&api, config, &languages, &config_path).expect("scaffold fixture");
    let content = files
        .iter()
        .find(|file| file.path == Path::new(relative_path))
        .unwrap_or_else(|| panic!("the scaffold stage must emit {relative_path}"))
        .content
        .clone();
    crate::cli::pipeline::ensure_generated_header(&PathBuf::from(relative_path), &content)
}

/// The regression test for `alef all`'s scaffold phase: a scaffold-phase file the formatter would
/// change must ship canonical, in the same run that stamps it.
///
/// Before the fix, `alef all` stamped `current_gen_paths` immediately after the scaffold writer
/// ran -- roughly 480 lines ahead of its only `format_generated_reporting` call -- and poly's
/// hash-stamped-generated-file skip turned that format pass into a no-op for the file, which
/// shipped with whatever the scaffold template emitted. ~keep
#[test]
fn all_formats_scaffold_output_before_stamping_it() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_all_fixture_workspace(&root);

    let raw = raw_scaffold_content(&root, SCAFFOLD_TARGET);
    assert!(
        poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &raw),
        "control failed: the scaffold generator now emits poly-canonical bytes for \
         {SCAFFOLD_TARGET}, so this test can no longer tell a formatted pipeline from an \
         unformatted one. Point it at a scaffold path poly still has an opinion about. Raw \
         content was:\n{raw}"
    );

    run_all(&root);

    let shipped = std::fs::read_to_string(root.join(SCAFFOLD_TARGET))
        .unwrap_or_else(|error| panic!("alef all must emit {SCAFFOLD_TARGET}: {error}"));
    assert!(
        shipped.contains("alef:hash:"),
        "sanity: the shipped scaffold file must actually be stamped, or the claim below is about \
         a file alef does not own: {shipped}"
    );
    assert!(
        !poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &shipped),
        "`alef all` shipped a scaffold-phase file poly would still rewrite -- it was stamped \
         before the format pass, so poly skipped it. Content:\n{shipped}"
    );
}

/// The heal half, and the half a pure reordering does not deliver.
///
/// A repository generated by a pre-fix alef holds files that are stamped AND non-canonical. Their
/// generated bodies have not changed, so `write_files_report` (which compares hash-stripped
/// bodies) does not rewrite them, so they keep the stamp, so poly keeps skipping them -- moving
/// the stamp later in the run changes nothing for them. Measured on a neutral eight-language
/// fixture: re-running a reordered-but-not-unstamping alef over a pre-fix tree canonicalised 6 of
/// the 21 affected files and left the other 15 exactly as it found them.
///
/// The pre-fix state is reconstructed here rather than produced by shelling out to an older
/// binary: the target is written back to the generator's own raw bytes under a well-formed
/// `alef:hash:` line, which is byte-for-byte what a pre-fix run left behind. ~keep
#[test]
fn all_reformats_a_scaffold_file_left_stamped_and_uncanonical_by_an_earlier_run() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_all_fixture_workspace(&root);

    run_all(&root);
    let target = root.join(SCAFFOLD_TARGET);
    let canonical = std::fs::read_to_string(&target).expect("the first alef all must emit the scaffold file");

    // The hash value is a placeholder: `finalize_hashes_sweeping` recomputes it at the end of the
    // next run, and poly's skip only pattern-matches the line's shape, never its correctness.
    let stale = crate::core::hash::inject_hash_line(&raw_scaffold_content(&root, SCAFFOLD_TARGET), &"a".repeat(64));
    assert!(
        crate::core::hash::content_has_alef_marker(&stale) && stale.contains("alef:hash:"),
        "sanity: the reconstructed pre-fix file must carry both an alef marker and a hash line, \
         or poly would never have skipped it: {stale}"
    );
    assert!(
        poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &stale),
        "control failed: the reconstructed pre-fix file is already poly-canonical, so the final \
         assertion below cannot fail: {stale}"
    );
    std::fs::write(&target, &stale).expect("plant the pre-fix file state");

    run_all(&root);

    let healed = std::fs::read_to_string(&target).expect("the scaffold file must survive the second run");
    assert!(
        !poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &healed),
        "`alef all` left a stamped, non-canonical scaffold file exactly as it found it -- poly \
         skipped it for the stamp it inherited, and nothing stripped that stamp before the format \
         pass. Content:\n{healed}"
    );
    assert_eq!(
        crate::core::hash::strip_hash_line(&healed),
        crate::core::hash::strip_hash_line(&canonical),
        "the healed file's body must match what a from-scratch run produces, not merely be poly-clean"
    );
}

/// Run `alef generate --lang python` against `root`, mirroring [`run_all`] but through
/// `Commands::Generate` -- the path this module's fix (removing `Commands::Generate`'s own
/// five per-phase `finalize_hashes` checkpoints, all ahead of its single format pass) landed
/// in, and which `all_commands.rs`'s author could not touch.
fn run_generate_python(root: &Path) {
    let _skip_guard = SkipCommandsGuard::set("cargo");
    let _cwd = crate::test_support::CwdGuard::enter(root);
    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: Some(vec!["python".to_owned()]),
            clean: false,
            skip_frb: true,
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");
}

/// `Commands::Generate`'s own version of [`all_formats_scaffold_output_before_stamping_it`]:
/// before this module's fix, `Commands::Generate` stamped `current_gen_paths` immediately after
/// the scaffold writer ran (`core_commands.rs`, then line 345) -- roughly 90 lines ahead of its
/// only `format_generated_reporting` call -- and poly's hash-stamped-generated-file skip turned
/// that format pass into a no-op for the file, which shipped with whatever the scaffold template
/// emitted. Same defect shape as `alef all` had, in a command `all_commands.rs`'s fix could not
/// reach. ~keep
#[test]
fn generate_formats_scaffold_output_before_stamping_it() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_all_fixture_workspace(&root);

    let raw = raw_scaffold_content(&root, SCAFFOLD_TARGET);
    assert!(
        poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &raw),
        "control failed: the scaffold generator now emits poly-canonical bytes for \
         {SCAFFOLD_TARGET}, so this test can no longer tell a formatted pipeline from an \
         unformatted one. Point it at a scaffold path poly still has an opinion about. Raw \
         content was:\n{raw}"
    );

    run_generate_python(&root);

    let shipped = std::fs::read_to_string(root.join(SCAFFOLD_TARGET))
        .unwrap_or_else(|error| panic!("alef generate must emit {SCAFFOLD_TARGET}: {error}"));
    assert!(
        shipped.contains("alef:hash:"),
        "sanity: the shipped scaffold file must actually be stamped, or the claim below is about \
         a file alef does not own: {shipped}"
    );
    assert!(
        !poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &shipped),
        "`alef generate` shipped a scaffold-phase file poly would still rewrite -- it was stamped \
         before the format pass, so poly skipped it. Content:\n{shipped}"
    );
}

/// `Commands::Generate`'s own version of
/// [`all_reformats_a_scaffold_file_left_stamped_and_uncanonical_by_an_earlier_run`]: the heal
/// half, and the half a pure reordering does not deliver, for the `alef generate` path.
///
/// A repository generated by a pre-fix alef holds files that are stamped AND non-canonical. Their
/// generated bodies have not changed, so `write_files_report` (which compares hash-stripped
/// bodies) does not rewrite them, so they keep the stamp, so poly keeps skipping them. Nothing
/// short of stripping the stamp before the format pass (`unstamp_before_formatting`) can reach
/// them -- and, distinctly from the `alef all` case, `Commands::Generate` must also reformat
/// something even when nothing changed this run: `changed_languages` is empty on a pure heal, so
/// the gate must fall back to the run's full `languages` set rather than silently no-op via an
/// empty `Some(&changed_languages)`. ~keep
#[test]
fn generate_reformats_a_scaffold_file_left_stamped_and_uncanonical_by_an_earlier_run() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_all_fixture_workspace(&root);

    run_generate_python(&root);
    let target = root.join(SCAFFOLD_TARGET);
    let canonical = std::fs::read_to_string(&target).expect("the first alef generate must emit the scaffold file");

    // The hash value is a placeholder: the final `finalize_hashes` in `Commands::Generate`
    // recomputes it at the end of the next run, and poly's skip only pattern-matches the line's
    // shape, never its correctness.
    let stale = crate::core::hash::inject_hash_line(&raw_scaffold_content(&root, SCAFFOLD_TARGET), &"a".repeat(64));
    assert!(
        crate::core::hash::content_has_alef_marker(&stale) && stale.contains("alef:hash:"),
        "sanity: the reconstructed pre-fix file must carry both an alef marker and a hash line, \
         or poly would never have skipped it: {stale}"
    );
    assert!(
        poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &stale),
        "control failed: the reconstructed pre-fix file is already poly-canonical, so the final \
         assertion below cannot fail: {stale}"
    );
    std::fs::write(&target, &stale).expect("plant the pre-fix file state");

    run_generate_python(&root);

    let healed = std::fs::read_to_string(&target).expect("the scaffold file must survive the second run");
    assert!(
        !poly_would_reformat(SCAFFOLD_TARGET_FILE_NAME, &healed),
        "`alef generate` left a stamped, non-canonical scaffold file exactly as it found it -- \
         poly skipped it for the stamp it inherited, and nothing stripped that stamp before the \
         format pass. Content:\n{healed}"
    );
    assert_eq!(
        crate::core::hash::strip_hash_line(&healed),
        crate::core::hash::strip_hash_line(&canonical),
        "the healed file's body must match what a from-scratch run produces, not merely be poly-clean"
    );
}

/// Regression for alef-task #557's "orphan-reclaim bookkeeping gap" diagnostic
/// (`cli::pipeline::generate::orphans::sweep_manifest_orphans`): `alef all` must claim every path
/// `PostBuildStep::owned_paths` reports as written, exactly like `alef generate` already does
/// (`core_commands/generate.rs`'s own fold-in, added for the alef #B incident this fixture's
/// helpers above were written against). Before this fix, `all_commands.rs`'s `handle` never called
/// `owned_paths` at all, so `MaterializeSwiftBridge`'s real (non-placeholder) swift-bridge trio --
/// `RustBridgeC.h`'s populated form, `SwiftBridgeCore.swift`, `{binding_crate}.swift` -- was
/// written to disk by every `alef all` run but never recorded in the persisted
/// `all-bindings-swift-ownership` stage manifest `sweep_manifest_orphans` reads back as its
/// "previous run" baseline on the NEXT run. That produces exactly the diagnosed symptom: a root
/// this run recorded kept files under, with the previous-run manifest recording none under it --
/// permanently, since nothing ever closed the loop. ~keep
#[test]
fn all_records_the_materialized_swift_bridge_trio_in_the_binding_ownership_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_all_swift_fixture_workspace(&root);
    seed_fake_swift_bridge_build_output(&root);

    run_all(&root);

    let sources_rust_bridge = root.join("packages/swift/Sources/RustBridge");
    let core_swift = sources_rust_bridge.join("SwiftBridgeCore.swift");
    let crate_swift = sources_rust_bridge.join(format!("{BINDING_CRATE_NAME}.swift"));
    let header = header_path(&root);
    for materialized in [&header, &core_swift, &crate_swift] {
        assert!(
            materialized.is_file(),
            "sanity: MaterializeSwiftBridge must have written {} from the fake swift-bridge build \
             output, or the manifest assertion below is vacuous",
            materialized.display()
        );
    }

    let manifest_path = root.join(".alef/test-lib/hashes/all-bindings-swift-ownership.manifest");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        panic!(
            "the all-bindings-swift-ownership manifest must exist after `alef all`: {} ({e})",
            manifest_path.display()
        )
    });
    let manifest_paths: std::collections::HashSet<&str> = manifest.lines().collect();

    for materialized in [&header, &core_swift, &crate_swift] {
        let absolute = materialized.display().to_string();
        assert!(
            manifest_paths.contains(absolute.as_str()),
            "post-build-owned path {absolute} must be recorded in the binding ownership manifest \
             so the NEXT `alef all` run's orphan sweep has a non-empty previous-run baseline for \
             this root -- manifest contents:\n{manifest}"
        );
    }
}

/// A committed `RustBridgeC.h` that `umbrella_header::is_populated` accepts (it references a
/// `__swift_bridge__$` symbol), carrying a marker no fresh materialization can reproduce.
///
/// Both properties are load-bearing. "Populated" is what sends `emit_swift_bridge_files` down its
/// `Ok(None)` branch when no build output is found -- the exact path alef #477's reporter hit --
/// rather than down the placeholder branch a never-built project takes. The marker is what makes
/// staleness observable: the fake build output below declares `DemoOpaque`, so a header that
/// still carries `REPRO-477` after a run was not refreshed, whatever the run logged. ~keep
const STALE_COMMITTED_HEADER: &str = "#ifndef RUST_BRIDGE_C_H\n\
                                      #define RUST_BRIDGE_C_H\n\
                                      \n\
                                      // REPRO-477 stale marker\n\
                                      void __swift_bridge__$stale_marker(void);\n\
                                      \n\
                                      #endif /* RUST_BRIDGE_C_H */\n";

fn write_stale_committed_header(root: &Path) {
    let header = header_path(root);
    std::fs::create_dir_all(header.parent().expect("header has a parent")).expect("create RustBridgeC dir");
    std::fs::write(&header, STALE_COMMITTED_HEADER).expect("write stale committed header");
}

/// Point the fixture's cargo at a target directory outside the project, the way alef #477's
/// reporter did.
///
/// A `.cargo/config.toml` `build.target-dir` is used rather than exporting `CARGO_TARGET_DIR`:
/// the env var is process-global and `cargo test` runs every test in one process, so setting it
/// here would redirect the build output of every concurrent test that spawns cargo -- the leak
/// `cli::pipeline::commands::test_apps`' `resolves_binary_built_under_a_custom_cargo_target_dir`
/// documents having already been bitten by. The config file is confined to this fixture tree.
///
/// It is also the stronger probe of the two: `CARGO_TARGET_DIR` is readable without cargo, so a
/// resolver that merely consulted the environment would satisfy it. Only asking `cargo metadata`
/// finds a `build.target-dir`. Cargo discovers that file by walking up from the process cwd, not
/// from `--manifest-path`, which is why this only works under `try_generate`'s `CwdGuard`. ~keep
fn point_cargo_at_an_external_target_dir(root: &Path, external_target: &Path) {
    std::fs::create_dir_all(root.join(".cargo")).expect("create .cargo dir");
    std::fs::write(
        root.join(".cargo/config.toml"),
        crate::test_support::cargo_target_dir_config(external_target),
    )
    .expect("write .cargo/config.toml");
}

/// Regression for alef #477: `MaterializeSwiftBridge` must read swift-bridge's build output from
/// the directory cargo actually writes to, not from a hard-coded `<workspace root>/target`.
///
/// Before the fix, `find_swift_bridge_out_dir` joined `"target"` onto the ancestor that carries
/// `Cargo.lock`. With the output redirected elsewhere that scan matched nothing, the already
/// populated committed header sent `emit_swift_bridge_files` to `Ok(None)`, nothing was written,
/// and the caller logged `Re-materialized swift-bridge files ... from fresh build output` anyway
/// -- exit 0, files stale. ~keep
#[test]
fn generate_materializes_from_an_external_cargo_target_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    let external = tempfile::tempdir().expect("tempdir for the external cargo target dir");
    let external_target = external
        .path()
        .canonicalize()
        .unwrap_or_else(|_| external.path().to_path_buf());

    write_fixture_workspace(&root);
    write_stale_committed_header(&root);
    point_cargo_at_an_external_target_dir(&root, &external_target);
    seed_fake_swift_bridge_build_output_in(&external_target);
    assert!(
        !root.join("target").exists(),
        "sanity: the project must have no `target/` of its own, or the old hard-coded join would \
         find the output anyway and this test could not fail"
    );

    run_generate(&root);

    let header = std::fs::read_to_string(header_path(&root)).expect("the header must still exist after generate");
    assert!(
        !header.contains("REPRO-477"),
        "the stale committed header survived a run that reported success -- the build output \
         under {} was never read. Header:\n{header}",
        external_target.display()
    );
    assert!(
        header.contains("DemoOpaque"),
        "the header must carry the declarations from the external build output. Header:\n{header}"
    );
    let core_swift = root.join("packages/swift/Sources/RustBridge/SwiftBridgeCore.swift");
    assert!(
        core_swift.is_file(),
        "the rest of the trio must be materialized from the external target dir too; {} is missing",
        core_swift.display()
    );
}

/// The other half of alef #477: when build output that was expected is missing, the step must
/// fail instead of logging success over an untouched tree.
///
/// A populated committed header is what marks this crate as one that *has* been built here
/// before, so "no build output found" is a broken run rather than a first generation. The
/// placeholder-writing first-generation case stays a success and is pinned separately by
/// `core::backend`'s `materialize_swift_bridge_does_not_claim_trio_members_it_never_wrote`. ~keep
#[test]
fn generate_fails_when_expected_swift_bridge_build_output_is_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    write_stale_committed_header(&root);

    let error = try_generate(&root).expect_err("generate must fail when the expected build output is missing");

    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("no usable swift-bridge build output"),
        "the failure must name the missing build output rather than some unrelated error: {rendered}"
    );
    assert_eq!(
        std::fs::read_to_string(header_path(&root)).expect("the header must still exist"),
        STALE_COMMITTED_HEADER,
        "a failed materialization must leave the committed header exactly as it found it"
    );
}
