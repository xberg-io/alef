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

    let poly = poly_toml(&files);
    assert!(
        poly.generated_header,
        "poly.toml must be alef-managed (generated_header)"
    );

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
fn emits_a_canonical_rustfmt_toml_at_width_120() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();

    let rustfmt = files
        .iter()
        .find(|f| f.path.to_string_lossy() == "rustfmt.toml")
        .expect("scaffold should emit a repo-root rustfmt.toml");
    assert!(rustfmt.generated_header, "rustfmt.toml must be alef-managed");
    assert!(
        rustfmt.content.contains("max_width = 120"),
        "rustfmt.toml must pin width 120 (poly defers to rustfmt discovery); got {:?}",
        rustfmt.content
    );
}

#[test]
fn poly_toml_drives_hooks_builtins_and_excludes() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(c.contains("[hooks]") && c.contains("stages = [ \"pre-commit\" ]"));
    assert!(c.contains("[hooks.builtin]"));
    assert!(c.contains("cargo = true"), "cargo builtin must be enabled");
    assert!(
        c.contains("commit = { stages = [ \"commit-msg\" ] }"),
        "commit builtin must run on commit-msg"
    );
    assert!(c.contains("[discovery]") && c.contains("\"target/**\""));
    assert!(c.contains("lint = { exclude = ["));
    assert!(c.contains("fmt = { exclude = ["));
    assert!(c.contains("file_safety = { exclude = ["));
}

#[test]
fn poly_toml_python_ruff_pyrefly_and_per_file_ignores() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(c.contains("[lint.python.ruff]") && c.contains("select = [ \"ALL\" ]"));
    assert!(c.contains("\"ANN401\","), "ruff ignore list must be ported");
    assert!(c.contains("pydocstyle_convention = \"google\""));
    assert!(c.contains("pylint_max_args = 10"));
    assert!(c.contains("[per-file-ignores]") && c.contains("\"**/api.py\""));
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
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(!c.contains("[lint.python.ruff]"), "no python table without python");
    assert!(!c.contains("[lint.php.mago]"), "no php table without php");
    assert!(!c.contains("pyrefly"), "no pyrefly hook without python");
    assert!(
        !c.contains("\"**/api.py\""),
        "no python wrapper per-file-ignores without python"
    );
    assert!(c.contains("\"**/e2e/**\""), "test/e2e per-file-ignores always emitted");
}

#[test]
fn poly_toml_never_enables_system_native_formatters() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(
        &api,
        &config,
        &[
            Language::Zig,
            Language::Java,
            Language::Kotlin,
            Language::KotlinAndroid,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::R,
        ],
    )
    .unwrap();
    let c = &poly_toml(&files).content;
    for header in [
        "[fmt.shell.shfmt]",
        "[fmt.zig.zigfmt]",
        "[fmt.java.google-java-format]",
        "[fmt.kotlin.ktfmt]",
        "[fmt.swift.swift-format]",
        "[fmt.dart.dartfmt]",
        "[fmt.gleam.gleamfmt]",
        "[fmt.r.styler]",
    ] {
        assert!(
            !c.contains(header),
            "{header} must NOT be enabled (pure-Rust policy); got:\n{c}"
        );
    }
}

#[test]
fn poly_toml_excludes_only_cargo_toml_from_formatting() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;
    assert!(
        c.contains("\"**/Cargo.toml\","),
        "Cargo.toml must be excluded from poly; got:\n{c}"
    );
    assert!(
        !c.contains("packages/elixir/**/*.ex"),
        "Elixir must NOT be excluded (poly tier-2 formats it now); got:\n{c}"
    );
    let fmt_pos = c.find("fmt = { exclude =").expect("fmt builtin present");
    assert!(
        c[fmt_pos..].contains("\"**/Cargo.toml\","),
        "Cargo.toml exclude must be mirrored into the fmt builtin"
    );
}

#[test]
fn poly_toml_extra_excludes_appear_in_discovery_and_hooks() {
    let config = test_config_with_poly(r#"exclude = ["vendor/generated/**", "third-party/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"vendor/generated/**\","),
        "[discovery] exclude must contain repo-extra glob"
    );
    assert!(
        c.contains("\"third-party/**\","),
        "[discovery] exclude must contain second repo-extra glob"
    );

    let lint_pos = c.find("lint = { exclude =").expect("lint builtin present");
    let fmt_pos = c.find("fmt = { exclude =").expect("fmt builtin present");
    let filesafety_pos = c
        .find("file_safety = { exclude =")
        .expect("file_safety builtin present");

    for builtin_pos in [lint_pos, fmt_pos, filesafety_pos] {
        let after = &c[builtin_pos..];
        assert!(
            after.contains("\"vendor/generated/**\","),
            "builtin at pos {builtin_pos} must include repo-extra exclude"
        );
    }

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

    let e2e_pos = c.find("\"**/e2e/**\"").expect("e2e glob present");
    let legacy_pos = c.find("\"**/legacy_api.py\"").expect("legacy_api.py glob present");
    assert!(
        legacy_pos > e2e_pos,
        "repo per-file-ignores must be appended after the generated test globs"
    );
}

#[test]
fn poly_toml_empty_poly_config_leaves_output_unchanged() {
    let config_default = test_config();
    let config_explicit_empty = test_config_with_poly("");

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

#[test]
fn poly_toml_typos_extend_words_emitted_before_per_file_ignores() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.typos.extend-words]
flate = "flate"
arange = "arange"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.typos.extend_words]\n"),
        "[lint.typos.extend_words] section must be emitted; got:\n{c}"
    );
    assert!(c.contains("arange = \"arange\""), "arange entry must appear");
    assert!(c.contains("flate = \"flate\""), "flate entry must appear");

    let typos_pos = c.find("[lint.typos.extend_words]").expect("extend_words present");
    let per_file_pos = c.find("[per-file-ignores]").expect("per-file-ignores present");
    assert!(
        typos_pos < per_file_pos,
        "[lint.typos.*] must appear before [per-file-ignores]"
    );
}

#[test]
fn poly_toml_typos_extend_identifiers_emitted() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.typos.extend-identifiers]
PyMuPDF = "PyMuPDF"
PDFium = "PDFium"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.typos.extend_identifiers]\n"),
        "[lint.typos.extend_identifiers] section must be emitted; got:\n{c}"
    );
    assert!(c.contains("PDFium = \"PDFium\""), "PDFium identifier must appear");
    assert!(c.contains("PyMuPDF = \"PyMuPDF\""), "PyMuPDF identifier must appear");

    assert!(
        !c.contains("[lint.typos.extend_words]"),
        "extend_words must not be emitted when empty"
    );
}

#[test]
fn poly_toml_typos_both_tables_emitted_with_correct_ordering() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.typos.extend-words]
flate = "flate"

[workspace.poly.typos.extend-identifiers]
PyMuPDF = "PyMuPDF"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.typos.extend_words]\n"),
        "extend_words must be emitted"
    );
    assert!(
        c.contains("[lint.typos.extend_identifiers]\n"),
        "extend_identifiers must be emitted"
    );

    let words_pos = c.find("[lint.typos.extend_words]").expect("extend_words present");
    let idents_pos = c
        .find("[lint.typos.extend_identifiers]")
        .expect("extend_identifiers present");
    let per_file_pos = c.find("[per-file-ignores]").expect("per-file-ignores present");
    assert!(words_pos < idents_pos, "extend_words must precede extend_identifiers");
    assert!(idents_pos < per_file_pos, "typos tables must precede per-file-ignores");
}

#[test]
fn poly_toml_empty_typos_config_emits_no_typos_tables() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        !c.contains("[lint.typos."),
        "no [lint.typos.*] tables must be emitted when TyposConfig is empty; got:\n{c}"
    );
}

#[test]
fn poly_toml_omits_uncomment_table_by_default() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        !c.contains("[lint.uncomment]"),
        "no [lint.uncomment] table when [workspace.poly.uncomment] is absent; got:\n{c}"
    );
}

#[test]
fn poly_toml_empty_poly_config_still_omits_uncomment() {
    let config = test_config_with_poly("");
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;
    assert!(!c.contains("[lint.uncomment]"), "empty poly config must omit uncomment");
}

#[test]
fn poly_toml_uncomment_table_emitted_with_snake_case_keys() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.uncomment]
enabled = true
remove-todos = true
remove-fixme = false
remove-docs = true
use-default-ignores = false
preserve-patterns = ["SAFETY:", "allow("]
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(c.contains("[lint.uncomment]\n"), "uncomment table must be emitted");
    assert!(c.contains("enabled = true\n"), "enabled key must render");
    assert!(c.contains("remove_todos = true\n"), "remove_todos snake_case key");
    assert!(c.contains("remove_fixme = false\n"), "remove_fixme snake_case key");
    assert!(c.contains("remove_docs = true\n"), "remove_docs snake_case key");
    assert!(
        c.contains("use_default_ignores = false\n"),
        "use_default_ignores snake_case key"
    );
    assert!(c.contains("\"SAFETY:\","), "preserve_patterns entries must render");
    assert!(c.contains("\"allow(\","), "second preserve_patterns entry must render");

    assert!(!c.contains("remove-todos"), "must not leak kebab-case keys");
    assert!(!c.contains("use-default-ignores"), "must not leak kebab-case keys");

    let uncomment_pos = c.find("[lint.uncomment]").expect("uncomment present");
    let per_file_pos = c.find("[per-file-ignores]").expect("per-file-ignores present");
    assert!(
        uncomment_pos < per_file_pos,
        "[lint.uncomment] must precede [per-file-ignores]"
    );
}

#[test]
fn poly_toml_uncomment_empty_table_fills_poly_defaults() {
    let config = test_config_with_workspace_toml("[workspace.poly.uncomment]\n");
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.uncomment]\n"),
        "empty table still emits [lint.uncomment]"
    );
    assert!(c.contains("enabled = true\n"), "enabled defaults true");
    assert!(
        c.contains("use_default_ignores = true\n"),
        "use_default_ignores defaults true"
    );
    assert!(c.contains("remove_todos = false\n"), "remove_todos defaults false");
    assert!(c.contains("preserve_patterns = []\n"), "empty preserve_patterns -> []");
}

#[test]
fn poly_toml_file_safety_exclude_defaults_to_shared_excludes() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    let array_body = |key: &str| -> String {
        let start = c.find(key).unwrap_or_else(|| panic!("{key} present"));
        let rel_end = c[start..].find("] }").expect("builtin array close");
        c[start + key.len()..start + rel_end + 3].to_string()
    };
    assert_eq!(
        array_body("lint = { exclude ="),
        array_body("file_safety = { exclude ="),
        "file_safety exclude must match lint's when no file-safety-exclude is set"
    );
}

#[test]
fn poly_toml_file_safety_exclude_only_appends_to_file_safety() {
    let config = test_config_with_poly(r#"file-safety-exclude = ["crates/*/src/lib.rs", "**/inner.rs"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    let builtin_region = |key: &str| -> &str {
        let start = c.find(key).unwrap_or_else(|| panic!("{key} present"));
        let rel_end = c[start..].find("] }").expect("builtin array close");
        &c[start..start + rel_end + 3]
    };

    let fs_region = builtin_region("file_safety = { exclude =");
    assert!(
        fs_region.contains("\"crates/*/src/lib.rs\","),
        "file-safety-exclude glob must appear in file_safety builtin"
    );
    assert!(
        fs_region.contains("\"**/inner.rs\","),
        "second file-safety-exclude glob must appear in file_safety builtin"
    );
    assert!(
        fs_region.contains("\"target/**\","),
        "default file_safety excludes preserved"
    );

    let discovery_pos = c.find("[discovery]").expect("discovery present");
    let hooks_pos = c.find("[hooks]").expect("hooks present");
    let discovery_region = &c[discovery_pos..hooks_pos];
    assert!(
        !discovery_region.contains("crates/*/src/lib.rs"),
        "file-safety-exclude glob must NOT appear in [discovery]"
    );

    let lint_region = builtin_region("lint = { exclude =");
    assert!(
        !lint_region.contains("crates/*/src/lib.rs"),
        "file-safety-exclude glob must NOT be in lint builtin"
    );
    let fmt_region = builtin_region("fmt = { exclude =");
    assert!(
        !fmt_region.contains("crates/*/src/lib.rs"),
        "file-safety-exclude glob must NOT be in fmt builtin"
    );
}

#[test]
fn poly_toml_typos_entries_are_alphabetically_ordered() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.poly.typos.extend-words]
zensical = "zensical"
arange = "arange"
flate = "flate"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    let arange_pos = c.find("arange =").expect("arange entry present");
    let flate_pos = c.find("flate =").expect("flate entry present");
    let zensical_pos = c.find("zensical =").expect("zensical entry present");
    assert!(arange_pos < flate_pos, "arange must come before flate (alphabetical)");
    assert!(
        flate_pos < zensical_pos,
        "flate must come before zensical (alphabetical)"
    );
}
