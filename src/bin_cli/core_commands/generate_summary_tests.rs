//! Coverage for `handle_generate`'s always-printed, per-category summary.
//!
//! Before this fix, `alef generate` printed exactly one flat line -- `"Generated {N} files"` --
//! with no breakdown and no indication of which categories this command structurally cannot
//! touch. A 98-file per-language run and a ~278-file full regen (which additionally covers
//! `docs-site/src` and `e2e/`, both entirely out of `alef generate`'s scope) printed
//! indistinguishably shaped output, and a consumer misread the former as the latter because
//! nothing in the log contradicted that reading. This is the honest fallback the task allows
//! when "silently bailed" and "legitimately nothing to do" cannot be told apart reliably: every
//! category's count, including zero, printed every run, plus an explicit statement of what this
//! command never generates.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use std::path::Path;

const FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"summary-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "summary-fixture"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "summary_fixture"

[crates.python.stubs]
output = "packages/python/summary_fixture"
"#;

fn write_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

fn run_generate(root: &Path) {
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
            skip_compile: true,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");
}

/// A fresh run must report a nonzero binding count AND explicitly say this command never covers
/// docs/e2e, naming how many of the processed crates even have `[e2e]` configured (zero, for
/// this fixture, which declares none).
#[test]
#[tracing_test::traced_test]
fn fresh_generate_reports_binding_count_and_names_the_out_of_scope_categories() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    run_generate(&root);

    assert!(
        logs_contain("alef generate` never generates docs or e2e/test-app output"),
        "the summary must explicitly name the categories this command cannot produce, so its \
         output is never mistaken for a full `alef all` regen"
    );
    assert!(
        logs_contain("0 of 1 processed crate(s) here have an [e2e] block configured"),
        "the summary must say how many processed crates even have e2e configured, not just \
         that e2e itself was zero"
    );
    assert!(
        !logs_contain("Generate summary: 0 binding"),
        "sanity: a fresh run against a real python fixture must produce at least one binding \
         file, or this test is not exercising a real generation at all"
    );
}

/// The robustness requirement: an immediate second run against an unchanged tree is a legitimate
/// no-op (everything already current) and must print zero in every category without that being
/// treated as, or read as, a failure -- `alef generate` must still exit 0 and the summary must
/// still show the honest zero, not omit the line or fail the run.
#[test]
#[tracing_test::traced_test]
fn a_legitimate_up_to_date_rerun_reports_zero_without_failing() {
    if !crate::cli::pipeline::is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    run_generate(&root);
    run_generate(&root);

    assert!(
        logs_contain("Generate summary: 0 binding, 0 service-api, 0 public-api"),
        "an up-to-date rerun must still print the full zero-safe summary rather than nothing, \
         and must not fail merely because every category was zero"
    );
}
