//! Tests for the repo-root `poly.toml` scaffolding.

use super::*;
use crate::core::config::{Language, NewAlefConfig};

/// Build a `ResolvedCrateConfig` from a full `alef.toml`-shaped TOML string.
///
/// `extra_workspace_toml` is inserted inside the `[workspace]` section (scalar
/// values and sub-tables whose headers use the `[workspace.*]` prefix).
/// For poly settings that are purely scalar values (like `exclude`), callers
/// can embed them directly:
///   `test_config_with_workspace_toml("[workspace.poly]\nexclude = [...]")`
fn test_config_with_workspace_toml(extra_workspace_toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node"]

{extra_workspace_toml}

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// Build a `ResolvedCrateConfig` with an explicit `[workspace.poly]` table
/// that contains only scalar-level poly entries (no sub-tables).
///
/// `poly_scalars` is inserted directly under `[workspace.poly]` — suitable for
/// `exclude = [...]` but NOT for `[per-file-ignores]` (those need their own
/// `[workspace.poly.per-file-ignores]` table header).
fn test_config_with_poly(poly_scalars: &str) -> ResolvedCrateConfig {
    test_config_with_workspace_toml(&format!("[workspace.poly]\n{poly_scalars}"))
}

/// Locate the generated `poly.toml` in a scaffold result.
fn poly_toml(files: &[GeneratedFile]) -> &GeneratedFile {
    files
        .iter()
        .find(|f| f.path.to_string_lossy() == "poly.toml")
        .expect("scaffold should emit a repo-root poly.toml")
}

#[test]
fn emits_a_generated_poly_toml_replacing_precommit() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();

    // poly.toml is emitted, alef-managed (hash-tracked, overwritten on regen).
    let poly = poly_toml(&files);
    assert!(
        poly.generated_header,
        "poly.toml must be alef-managed (generated_header)"
    );

    // The former per-tool / pre-commit configs are gone.
    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    assert!(
        !paths.iter().any(|p| p.ends_with(".pre-commit-config.yaml")),
        "must not emit .pre-commit-config.yaml; got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with(".typos.toml")),
        "must not emit .typos.toml; got {paths:?}"
    );
}

#[test]
fn poly_toml_drives_hooks_builtins_and_excludes() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    // Single-config hook orchestration: builtins + commit-msg stage.
    assert!(c.contains("[hooks]") && c.contains("stages = [ \"pre-commit\" ]"));
    assert!(c.contains("[hooks.builtin]"));
    assert!(c.contains("cargo = true"), "cargo builtin must be enabled");
    assert!(
        c.contains("commit = { stages = [ \"commit-msg\" ] }"),
        "commit builtin must run on commit-msg"
    );
    // Excludes appear in discovery (direct CLI) and the builtin hook path.
    assert!(c.contains("[discovery]") && c.contains("\"target/**\""));
    assert!(c.contains("polylint = { exclude = ["));
    assert!(c.contains("polyfmt = { exclude = ["));
    assert!(c.contains("file_safety = { exclude = ["));
}

#[test]
fn poly_toml_python_ruff_pyrefly_and_per_file_ignores() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    // ruff rule selection (ported from the dropped [tool.ruff]).
    assert!(c.contains("[lint.python.ruff]") && c.contains("select = [ \"ALL\" ]"));
    assert!(c.contains("\"ANN401\","), "ruff ignore list must be ported");
    // Forward-compat ruff params poly will honor once landed.
    assert!(c.contains("pydocstyle_convention = \"google\""));
    assert!(c.contains("pylint_max_args = 10"));
    // Cross-engine per-file ignores for the alef wrappers.
    assert!(c.contains("[per-file-ignores]") && c.contains("\"**/api.py\""));
    // pyrefly type-check hook in project mode (replaces mypy).
    assert!(c.contains("[hooks.pre-commit.commands.pyrefly]") && c.contains("pyrefly check packages/python"));
}

#[test]
fn poly_toml_php_uses_mago_correctness_security() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python, Language::Php]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.php.mago]") && c.contains("select = [ \"correctness\", \"security\" ]"),
        "PHP must use mago correctness/security ruleset (replacing phpstan/php-cs-fixer)"
    );
}

#[test]
fn poly_toml_omits_language_tables_when_language_absent() {
    let config = test_config();
    let api = test_api();
    // No Python, no PHP.
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(!c.contains("[lint.python.ruff]"), "no python table without python");
    assert!(!c.contains("[lint.php.mago]"), "no php table without php");
    assert!(!c.contains("pyrefly"), "no pyrefly hook without python");
    // per-file-ignores is always emitted (generated test/e2e suites exist in
    // every repo), but the python-wrapper entries must be absent without python.
    assert!(
        !c.contains("\"**/api.py\""),
        "no python wrapper per-file-ignores without python"
    );
    assert!(c.contains("\"**/e2e/**\""), "test/e2e per-file-ignores always emitted");
}

// ── [workspace.poly] merge tests ────────────────────────────────────────────

#[test]
fn poly_toml_extra_excludes_appear_in_discovery_and_hooks() {
    let config = test_config_with_poly(r#"exclude = ["vendor/generated/**", "third-party/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    // Extra globs must appear in [discovery] exclude.
    assert!(
        c.contains("\"vendor/generated/**\","),
        "[discovery] exclude must contain repo-extra glob"
    );
    assert!(
        c.contains("\"third-party/**\","),
        "[discovery] exclude must contain second repo-extra glob"
    );

    // The same extra globs must be mirrored into all three builtin excludes.
    let polylint_pos = c.find("polylint = { exclude =").expect("polylint builtin present");
    let polyfmt_pos = c.find("polyfmt = { exclude =").expect("polyfmt builtin present");
    let filesafety_pos = c.find("file_safety = { exclude =").expect("file_safety builtin present");

    // A simple content check: the string appears after each builtin key.
    // Because the same merged `excludes` variable is used for all three, a
    // substring check across the whole document is sufficient.
    for builtin_pos in [polylint_pos, polyfmt_pos, filesafety_pos] {
        let after = &c[builtin_pos..];
        assert!(
            after.contains("\"vendor/generated/**\","),
            "builtin at pos {builtin_pos} must include repo-extra exclude"
        );
    }

    // Default globs must still be present (defaults come first).
    assert!(c.contains("\"target/**\","), "built-in excludes must be preserved");
}

#[test]
fn poly_toml_extra_per_file_ignores_appended() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.per-file-ignores]
"**/legacy_api.py" = ["ANN", "D103"]
"**/compat.py" = ["UP035", "F401"]"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    // Both repo-specific per-file-ignore globs and their codes must appear.
    assert!(
        c.contains("\"**/legacy_api.py\""),
        "repo per-file-ignore glob must be emitted"
    );
    assert!(c.contains("\"ANN\","), "rule code ANN must appear for legacy_api.py");
    assert!(c.contains("\"D103\","), "rule code D103 must appear for legacy_api.py");
    assert!(
        c.contains("\"**/compat.py\""),
        "second repo per-file-ignore glob must be emitted"
    );
    assert!(c.contains("\"UP035\","), "rule code UP035 must appear for compat.py");

    // Repo globs must come AFTER the generated test/e2e globs.
    let e2e_pos = c.find("\"**/e2e/**\"").expect("e2e glob present");
    let legacy_pos = c.find("\"**/legacy_api.py\"").expect("legacy_api.py glob present");
    assert!(
        legacy_pos > e2e_pos,
        "repo per-file-ignores must be appended after the generated test globs"
    );
}

#[test]
fn poly_toml_empty_poly_config_leaves_output_unchanged() {
    // A config with an explicit empty [workspace.poly] section must produce
    // output byte-identical to one with no [workspace.poly] at all.
    let config_default = test_config();
    let config_explicit_empty = test_config_with_poly(""); // empty poly section

    let api = test_api();
    let files_default = scaffold(&api, &config_default, &[Language::Python, Language::Node]).unwrap();
    let files_empty = scaffold(&api, &config_explicit_empty, &[Language::Python, Language::Node]).unwrap();

    let default_content = &poly_toml(&files_default).content;
    let empty_content = &poly_toml(&files_empty).content;

    assert_eq!(
        default_content, empty_content,
        "empty [workspace.poly] must produce byte-identical poly.toml"
    );
}
