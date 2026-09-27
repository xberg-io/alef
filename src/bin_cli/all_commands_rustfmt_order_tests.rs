//! THE regression test for alef #465: `alef all` used to write generated Rust bindings before
//! the scaffold stage that emits `rustfmt.toml`, so a project's FIRST `alef all` run (no
//! committed `rustfmt.toml` yet) fed `format_rust_content` a directory with no config anywhere
//! in its upward chain -- a hard error for `rustfmt --config-path <dir>` -- and the freshly
//! generated bindings landed on disk completely unformatted, raw codegen.
//!
//! This drives the real `handle` entry point end to end against a from-scratch fixture (no
//! pre-existing `rustfmt.toml`), with `poly` made to look absent on `PATH` for the run's
//! duration. That second part is not incidental: `alef all`'s own later whole-tree formatting
//! pass (`pipeline::format_generated_reporting`) would otherwise reformat the very file this
//! test inspects using `poly fmt --fix`, correctly, regardless of whether the write-time
//! ordering fix under test did anything at all -- which would make this test pass for the wrong
//! reason (see `measurement-discipline`/`prove-the-check-fired`). `cargo fmt --all` cannot paper
//! over this either: the fixture's root `Cargo.toml` declares no `[workspace]`, so it never sees
//! the separately-generated `crates/test-lib-py` package.

use super::handle;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::test_support::{CwdGuard, PathWithoutToolGuard, RealCargoGuard};

const FIXTURE_SOURCE: &str = "pub fn compute_widget_totals(\n    \
    alpha: String,\n    bravo: String,\n    charlie: String,\n    delta: String,\n    echo: String,\n    \
    foxtrot: String,\n    golf: String,\n    hotel: String,\n    india: String,\n    juliett: String,\n\
) -> String {\n    \
    format!(\"{alpha}{bravo}{charlie}{delta}{echo}{foxtrot}{golf}{hotel}{india}{juliett}\")\n\
}\n";

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

fn write_fixture(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

fn all_command() -> Commands {
    Commands::All {
        clean: false,
        clobber_create_once_seeds: false,
        strict: false,
        skip_frb: false,
        skip_snippet_validation: false,
        skip_compile: true,
    }
}

/// THE REGRESSION TEST. Both bounds matter and neither is redundant:
///
/// - `> 100` fails on its own if the pre-pass ran but `format_rust_content` fell back to
///   rustfmt's *default* `max_width = 100` instead of finding the project's real, committed
///   `rustfmt.toml` (`max_width = 120`) -- which is exactly what happens if the pre-pass call is
///   removed from `handle` while the rest of the fix stays in place: no config exists yet, so
///   `format_rust_content` correctly falls back to defaults instead of hard-erroring, and the
///   long call-forwarding line this fixture is built to produce gets wrapped down to fit under
///   100 columns everywhere.
/// - `<= 120` fails on its own if the ordering bug is present at all: the freshly generated
///   `crates/test-lib-py/src/lib.rs` then lands as raw, unformatted codegen (measured by hand at
///   over 600 columns on one line for this exact fixture), which also happens to still exceed
///   100 -- so `> 100` alone would pass on the broken output too.
///
/// Sabotage: comment out (or reorder after the bindings stage) the
/// `pipeline::write_format_config_prepass(..)` call in `all_commands.rs::handle`.
#[test]
fn all_writes_rustfmt_toml_before_bindings_so_the_first_run_is_already_formatted() {
    if !crate::cli::pipeline::is_tool_available("rustfmt") {
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
    write_fixture(&root);
    assert!(
        !root.join("rustfmt.toml").exists(),
        "fixture setup: this run must start with no committed rustfmt.toml, or it is not \
         exercising a project's first `alef all` run at all"
    );

    // `RealCargoGuard`: the fixture's root `Cargo.toml` makes `converge_full_regen`'s
    // `run_cargo_fmt` spawn a real `cargo fmt --all` unconditionally. `PathWithoutToolGuard`:
    // see this file's module doc for why `poly` must look absent for this test to mean anything.
    let _cargo_guard = RealCargoGuard::acquire();
    let _no_poly = PathWithoutToolGuard::exclude("poly");
    let _cwd = CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    handle(all_command(), &context).expect("alef all must succeed against a plain python fixture");

    let generated_path = root.join("crates/test-lib-py/src/lib.rs");
    let generated = std::fs::read_to_string(&generated_path)
        .unwrap_or_else(|e| panic!("read generated bindings at {}: {e}", generated_path.display()));
    let longest_line = generated.lines().map(str::len).max().unwrap_or(0);

    assert!(
        longest_line > 100,
        "longest generated line was {longest_line} columns (expected > 100) -- this fixture is \
         built so a correctly max_width=120-formatted file needs more than 100 columns \
         somewhere; a shorter result means rustfmt's default max_width=100 was used instead of \
         the project's own rustfmt.toml, got:\n{generated}"
    );
    assert!(
        longest_line <= 120,
        "longest generated line was {longest_line} columns (expected <= 120) -- the project's \
         own rustfmt.toml sets max_width = 120, so nothing in a correctly formatted file may \
         exceed it; a much longer line means the bindings landed unformatted (alef #465), got:\n{generated}"
    );
}
