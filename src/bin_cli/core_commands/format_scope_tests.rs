//! What a partial regen (`alef generate`) actually hands to the formatter, measured through
//! the real command rather than through `poly_paths`' arithmetic.
//!
//! `format_generated(..., Some(changed_languages))` used to format `package_dir(lang)` alone,
//! while `finalize_hashes` stamped -- and `generate_sweep_roots` swept -- both that and
//! `output_for(lang)`. For most languages the two coincide or nest, so nothing showed. For
//! Python they do not: the wheel is `packages/python`, the PyO3 glue crate is generated into
//! `crates/<name>-py/src`, and only the first was ever formatted. `alef generate --lang python`
//! therefore stamped Rust no formatter had canonicalised, and the next whole-tree pass made
//! that stamp stale.
//!
//! Elixir deliberately does NOT gain this property: `POLY_ELIXIR_EXCLUDE_GLOBS` keeps poly away
//! from `.ex`/`.exs` entirely, because poly's Elixir engine misindents mix-canonical output and
//! then reports its own output `--check`-clean. That exclusion is correct, and this fixture
//! stays on Python so it never encodes an argument against it. ~keep

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use std::path::Path;

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
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

/// Deliberately unformatted Rust, planted in the glue crate's source directory before the run.
///
/// The scope question is "was this directory handed to poly at all", and the emitter's own
/// output cannot answer it: for a fixture this small the PyO3 glue crate happens to come out
/// already rustfmt-canonical, so asserting on `lib.rs` passes with or without the fix --
/// measured, not assumed. poly formats the directories it is given, not a per-file allowlist,
/// so a file that only a widened scope can reach is what actually discriminates. ~keep
const UNFORMATTED_PROBE: &str = "pub fn probe()->i32{let x=1;x}\n";

fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// `alef generate` must format the generated PyO3 glue crate, not only the wheel package.
#[test]
fn generate_formats_the_rust_glue_crate_it_stamps() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let glue_src = root.join("crates/test-lib-py/src");
    std::fs::create_dir_all(&glue_src).expect("glue crate src directory");
    let probe = glue_src.join("scope_probe.rs");
    std::fs::write(&probe, UNFORMATTED_PROBE).expect("plant the scope probe");

    // Holds `test_support::CWD_LOCK` for the whole test: `sources_hash` and
    // `read_alef_toml_bytes` resolve against the process cwd, so a test that enters a fixture
    // directory without it passes alone and fails in the full suite. ~keep
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");

    assert!(
        root.join("crates/test-lib-py/src/lib.rs").exists(),
        "sanity: the run must actually have generated the PyO3 glue crate, or the scope \
         assertion below proves nothing"
    );
    let formatted = std::fs::read_to_string(root.join("packages/python/test_lib/__init__.py"))
        .expect("sanity: the wheel package must also have been generated");
    assert!(
        !formatted.is_empty(),
        "sanity: the wheel package directory -- the one scope that always worked -- must be populated"
    );

    let probe_after = std::fs::read_to_string(&probe).expect("read the scope probe back");
    assert_ne!(
        probe_after, UNFORMATTED_PROBE,
        "the generated PyO3 glue crate's source directory was never handed to the formatter. \
         `finalize_hashes` stamps that directory and `generate_sweep_roots` sweeps it, so a \
         narrower formatting scope means `alef generate` emits non-canonical Rust and stamps a \
         hash over it -- and the next whole-tree format pass makes every one of those stamps stale"
    );
}

/// A crate targeting Java plus Node, with the `package_metadata` a Java scaffold requires
/// (`repository`, `authors`, `license`) so `scaffold_java` succeeds without an FFI target.
///
/// Node is included deliberately, not incidentally: napi's `PostBuildStep::PatchFile` runs
/// unconditionally on every `alef generate` (see `napi::gen_bindings::build_config`), so
/// `languages_with_post_build_steps` always folds `node` into `changed_languages`, making
/// `any_output_changed` true even on a run where nothing in `files`/`stub_files` changed. That
/// is what actually exercises the defect: with `any_output_changed` true, `format_scope` is
/// `changed_languages` verbatim (not the "format every requested language" fallback a
/// single-language fixture would hit instead), so `java`'s absence from that set -- because a
/// scaffold-only pom.xml write never added it -- narrows `poly_paths` to exclude
/// `packages/java` entirely. A java-only fixture does not reproduce this: with one requested
/// language, the fallback scope and the correct scope are the same set by coincidence. ~keep
const JAVA_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["java", "node"]

[workspace.package_metadata]
repository = "https://github.com/example/test-lib"
authors = ["Test Author <test@example.com>"]
license = "MIT"

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.java]
package = "com.example.testlib"

[crates.node]
package_name = "test-lib"
"#;

/// A crate configuring `[crates.scaffold.cargo]`, so `alef generate` writes the workspace-root
/// `.cargo/config.toml` -- a scaffold file `package_dir`/`output_for`/crate-root ties to no
/// single language at all, unlike `pom.xml` above (which at least belongs to `packages/java`).
const CARGO_CONFIG_FIXTURE_ALEF_TOML: &str = r#"
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

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/test-lib"
authors = ["Test Author"]
keywords = ["test"]

[crates.scaffold.cargo]
"#;

/// `alef generate` must leave `.cargo/config.toml` poly-canonical, the same as every other
/// scaffold file it writes -- not just the ones that happen to live under a language's own
/// `package_dir`/`output_for`/crate root.
///
/// Before the fix, `poly_paths`' partial-regen scope is built entirely from
/// `package_dir(lang)`/`output_for(lang)`/crate-root directories (see its own doc comment), so a
/// workspace-root scaffold file with no single-language owner -- `.cargo/config.toml`, rendered
/// by `scaffold::render_cargo_config` -- is never handed to poly on an `alef generate` run,
/// regardless of which languages `format_scope` names. The raw template emits the
/// `wasm32-unknown-unknown` target's `rustflags` as one unwrapped line; only poly's own TOML
/// formatter reflows a long array like that one across lines, so a file that never reaches poly
/// ships exactly as the raw template wrote it. `alef all`'s whole-tree convergence pass
/// (`only_languages = None`) always covers it, which is why the same fixture through `alef all`
/// never reproduces this. ~keep
#[test]
fn generate_formats_a_workspace_root_scaffold_file_no_language_owns() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    std::fs::write(root.join("alef.toml"), CARGO_CONFIG_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");

    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");

    let cargo_config_path = root.join(".cargo/config.toml");
    assert!(
        cargo_config_path.exists(),
        "sanity: `alef generate` must have written .cargo/config.toml, or the canonical-form \
         assertion below proves nothing"
    );

    let check = std::process::Command::new("poly")
        .args(["fmt", "--check", "--fix-generated", ".cargo/config.toml"])
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
        "`poly fmt --check --fix-generated .cargo/config.toml` must report the file clean after \
         `alef generate` -- a workspace-root scaffold file with no owning language must still \
         reach the formatter. Output:\n{check_output}"
    );
}

fn write_java_fixture_workspace(root: &Path, alef_toml: &str) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), alef_toml).expect("write fixture alef.toml");
}

fn run_java_generate(root: &Path) {
    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");
}

/// A regen whose ONLY change is a scaffold-managed manifest -- `package_metadata.license`
/// rewrites `packages/java/pom.xml`'s `<licenses>` block but touches no `.java` binding
/// source at all -- must still leave `pom.xml` poly-canonical.
///
/// Before the fix, `reconcile_managed_scaffold_manifests`'s write report fed `any_written`
/// but never `changed_languages` (unlike the bindings/service-api/public-api/stubs phases,
/// which all insert their own language on a real change). On a regen where Java's bindings
/// were cache-hit (the license edit does not touch generated `.java` source), `java` never
/// entered `format_scope`, so `poly_paths` never named `packages/java` and the freshly
/// rewritten `pom.xml` shipped exactly as `reconcile_managed_scaffold_manifests` wrote it --
/// unformatted. `alef all`'s whole-tree convergence pass (`only_languages = None`) never
/// showed this, because it reformats every byte under the repo root regardless of which
/// phase wrote it. ~keep
#[test]
fn generate_formats_a_scaffold_manifest_changed_by_a_config_only_edit() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_java_fixture_workspace(&root, JAVA_FIXTURE_ALEF_TOML);

    let _cwd = crate::test_support::CwdGuard::enter(&root);

    run_java_generate(&root);
    let pom_path = root.join("packages/java/pom.xml");
    let baseline = std::fs::read_to_string(&pom_path).expect("sanity: pom.xml must exist after the first run");
    assert!(
        baseline.contains("<name>MIT</name>"),
        "sanity: the baseline pom.xml must carry the configured license, or the edit below \
         proves nothing: {baseline}"
    );

    // The only change between the two runs: `license` in `alef.toml`. No Rust source, no
    // binding content, changed at all.
    let updated_alef_toml = JAVA_FIXTURE_ALEF_TOML.replace("license = \"MIT\"", "license = \"Apache-2.0\"");
    std::fs::write(root.join("alef.toml"), &updated_alef_toml).expect("rewrite alef.toml with the new license");

    run_java_generate(&root);

    let updated = std::fs::read_to_string(&pom_path).expect("pom.xml must still exist after the second run");
    assert!(
        updated.contains("<name>Apache-2.0</name>"),
        "sanity: the second run must actually have rewritten pom.xml's license, or the \
         canonical-form assertion below proves nothing: {updated}"
    );

    let check = std::process::Command::new("poly")
        .args(["fmt", "--check", "--fix-generated", "packages/java/pom.xml"])
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
        "`poly fmt --check --fix-generated packages/java/pom.xml` must report the file clean \
         after a regen whose only change was a scaffold-managed manifest -- `java` must be \
         folded into `changed_languages` even when no binding-phase write touched it. \
         Output:\n{check_output}"
    );
}
