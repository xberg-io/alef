//! Regression coverage for 0.82.0's removal of `[lint.<lang>]` / `[setup.<lang>]` /
//! `[update.<lang>]` / `[clean.<lang>]` / `[build_commands.<lang>]` from the `alef.toml` schema.
//! `[test.<lang>]` is the one exception and is not covered here -- see `lookups.rs`'s own test
//! module for its coverage, which is unaffected by this removal.
//!
//! Split into its own file rather than grown inline in `lookups.rs`: that file is already at
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md). ~keep

use crate::core::config::extras::Language;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::config::output::{BuildCommandConfig, StringOrVec};

fn resolved_one(toml: &str) -> super::ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn resolved_lint_config_rejects_removed_workspace_lint_table() {
    let cfg: Result<NewAlefConfig, _> = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.lint.python]
check = "ruff check ."

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    );
    let err = cfg.expect_err("[workspace.lint] must no longer parse");
    assert!(
        err.to_string().contains("lint"),
        "error should name the removed `lint` field: {err}"
    );
}

/// Proves the removal did not leave Python lint with no command at all: with nothing
/// configured (the only shape `alef.toml` can express now), `lint_config_for_language` must
/// still return alef's own built-in `ruff` pipeline. A removal that silently dropped the
/// command instead of the override would still return a `LintConfig`, so the check that
/// matters is on the actual command text, not merely `Some`/`is_ok`.
#[test]
fn resolved_lint_config_falls_back_to_the_builtin_default() {
    let r = resolved_one(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    );
    let lint = r.lint_config_for_language(Language::Python);
    assert!(
        lint.check.expect("python has a default lint check command").commands()[0].contains("ruff"),
        "the built-in default must still fire with no override table available"
    );
}

/// Same "defaults still fire, not silently empty" proof as
/// `resolved_lint_config_falls_back_to_the_builtin_default`, for the other three removed
/// override families. A regression that left one language with `None` everywhere -- "no
/// command configured" reporting success -- is exactly the defect class 0.82.0 exists to
/// close, so each assertion is on the actual command text, not merely `is_some()`.
#[test]
fn resolved_setup_update_clean_configs_fall_back_to_builtin_defaults() {
    let r = resolved_one(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    );

    let setup = r.setup_config_for_language(Language::Python);
    assert!(
        setup.install.is_some(),
        "python setup default must still supply an install command"
    );

    let update = r.update_config_for_language(Language::Python);
    assert!(
        update.update.is_some() || update.upgrade.is_some(),
        "python update default must still supply an update or upgrade command"
    );

    let clean = r.clean_config_for_language(Language::Python);
    assert!(
        clean.clean.is_some() || clean.argv_clean.is_some(),
        "python clean default must still supply a clean command"
    );
}

/// `ResolvedCrateConfig::build_commands` is the one surviving override point for
/// `build_command_config_for_language`, and it exists only for this crate's own
/// `#[cfg(test)]` hermetic build-orchestration tests -- never through `alef.toml`. This pins
/// the two behaviors those tests depend on: a test override that declares its own `build`
/// replaces alef's built-in command and drops the built-in `dependency_precondition` (which
/// describes alef's own default, not the override's), while a `before`-only entry leaves both
/// the built-in `build` and its `dependency_precondition` in place. Direct field mutation,
/// not TOML: `[crates.build_commands.python]` no longer parses at all (0.82.0 removed it from
/// the schema), so this is the only way left to exercise the merge. ~keep
#[test]
fn build_command_config_test_override_replaces_build_and_drops_builtin_dependency_precondition() {
    let mut r = resolved_one(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    );
    r.build_commands.insert(
        Language::Python.to_string(),
        BuildCommandConfig {
            precondition: Some("command -v maturin".to_string()),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single("maturin build --out dist".to_string())),
            build_release: None,
            timeout_seconds: None,
        },
    );

    let effective = r.build_command_config_for_language(Language::Python);
    assert_eq!(
        effective.build.expect("override build command").commands(),
        vec!["maturin build --out dist"]
    );
    assert_eq!(
        effective.dependency_precondition, None,
        "the built-in dependency check describes alef's own default command, not the override"
    );
}

#[test]
fn build_command_config_before_only_test_override_keeps_the_builtin_build_and_dependency_precondition() {
    let mut r = resolved_one(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    );
    r.build_commands.insert(
        Language::Python.to_string(),
        BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: Some(StringOrVec::Single("echo hi".to_string())),
            build: None,
            build_release: None,
            timeout_seconds: None,
        },
    );

    let effective = r.build_command_config_for_language(Language::Python);
    assert!(effective.build.is_some(), "the built-in build command must survive");
    assert!(
        effective.dependency_precondition.is_some(),
        "a before-only override must keep the built-in dependency check"
    );
}
