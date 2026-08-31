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

    assert!(c.contains("[hooks]") && c.contains("stages = [\"pre-commit\"]"));
    assert!(c.contains("[hooks.builtin]"));
    assert!(c.contains("cargo = true"), "cargo builtin must be enabled");
    assert!(
        c.contains("commit = { stages = [\"commit-msg\"] }"),
        "commit builtin must run on commit-msg"
    );
    assert!(c.contains("[discovery]") && c.contains("\"**/target/**\""));
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

    assert!(c.contains("[lint.python.ruff]"));
    assert!(
        c.contains("select = [\n") && c.contains("\"ANN\",") && !c.contains("\"ALL\""),
        "ruff select must be an explicit allowlist, not the ALL catch-all"
    );
    assert!(
        !c.contains("\"CPY\",") && !c.contains("\"CPY001\","),
        "the copyright-header family must not be selected or ignored"
    );
    assert!(c.contains("\"ANN401\","), "in-family ruff ignores must be preserved");
    assert!(c.contains("pydocstyle_convention = \"google\""));
    assert!(c.contains("pylint_max_args = 10"));
    assert!(c.contains("[per-file-ignores]") && c.contains("\"**/api.py\""));
    assert!(c.contains("[hooks.pre-commit.commands.pyrefly]") && c.contains("pyrefly check packages/python"));
    assert!(
        c.contains("workspace = true"),
        "pyrefly must be a workspace hook so `poly lint .` runs it once over the whole project"
    );
}

#[test]
fn poly_toml_emits_workspace_lint_hooks_for_non_bundled_linters() {
    let config = test_config();
    let files = scaffold_poly_config(
        &config,
        &[
            Language::Python,
            Language::Ruby,
            Language::Go,
            Language::Java,
            Language::Dart,
            Language::Elixir,
            Language::KotlinAndroid,
        ],
    );
    let c = &files
        .iter()
        .find(|f| f.path.to_str() == Some("poly.toml"))
        .expect("poly.toml emitted")
        .content;

    // Each external linter poly does not bundle is delegated once through a workspace hook.
    // Exact equality, never `contains`: `"mix credo --strict"` is a SUBSTRING of the primed
    // credo command, so a containment check would still pass if the priming were dropped. ~keep
    let document: toml::Value = toml::from_str(c).expect("generated poly.toml must parse");
    let commands = &document["hooks"]["pre-commit"]["commands"];
    for (hook, cmd) in [
        ("golangci-lint", "golangci-lint run ./..."),
        ("checkstyle", "mvn -q checkstyle:check"),
        ("credo", "mix deps.get && mix credo --strict"),
    ] {
        assert_eq!(commands[hook]["run"].as_str(), Some(cmd), "wrong `{hook}` command");
        // The flag that makes `poly lint .` run these once (not per file).
        assert_eq!(
            commands[hook]["workspace"].as_bool(),
            Some(true),
            "`{hook}` must be workspace-scoped"
        );
    }

    // rubocop, steep and the two dart analyzers are NOT emitted: they need a gitignored,
    // project-local dependency directory that poly's staged snapshot never carries, so they
    // could not pass as pre-commit hooks. See `unrunnable_snapshot_hooks_are_not_emitted`. ~keep
    for hook in ["rubocop", "steep", "dart-analyze", "dart-e2e-analyze"] {
        assert!(commands.get(hook).is_none(), "`{hook}` must not be emitted");
    }
}

/// Snippet validation is a regeneration-time concern, not a pre-commit one: it compiles every
/// snippet against built language artifacts, which costs minutes and requires a toolchain per
/// target language that a committing developer is not guaranteed to have. `alef all` already
/// runs it in its docs stage against the tree it just generated. A `poly` hook re-ran that whole
/// pass on every commit touching any snippet root, so a one-line docs edit paid for a full
/// multi-language compile. No `poly.toml` alef generates may schedule it. ~keep
#[test]
fn poly_toml_never_schedules_snippet_validation_as_a_hook() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.docs.snippets]
dirs = ["docs/snippets"]
inline_dirs = ["docs/guides"]"#,
    );
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let content = &poly_toml(&files).content;

    assert!(!content.contains("alef-snippets"), "no snippet hook table: {content}");
    assert!(
        !content.contains("alef snippets check"),
        "no snippet check command anywhere: {content}"
    );
    toml::from_str::<toml::Value>(content).expect("generated poly.toml must parse");
}

/// Regression: a consumer with no `docs/` tree at all (e.g. docs migrated into
/// an Astro `docs-site/`) must not have a hardcoded `docs/assets/**` or
/// `docs/snippets/**` phantom path reinstated into their discovery excludes.
#[test]
fn poly_toml_omits_hardcoded_docs_excludes_without_snippets_config() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        !c.contains("docs/assets"),
        "must not hardcode a docs/assets exclude; got:\n{c}"
    );
    assert!(
        !c.contains("docs/snippets"),
        "must not hardcode a docs/snippets exclude when no snippet dirs are configured; got:\n{c}"
    );
}

/// The snippet-root exclude must track the CONFIGURED snippet directory, not a
/// hardcoded `docs/snippets`, so a repo whose docs live under `docs-site/` still
/// gets its actual snippet root excluded from lint/fmt.
#[test]
fn poly_toml_discovery_excludes_configured_snippet_dirs_not_hardcoded_docs_path() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.docs.snippets]
dirs = ["docs-site/src/snippets"]"#,
    );
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"docs-site/src/snippets/**\""),
        "discovery excludes must include the configured snippet root; got:\n{c}"
    );
    assert!(
        !c.contains("\"docs/snippets/**\"") && !c.contains("/docs/snippets/**"),
        "must not fall back to the hardcoded docs/snippets path; got:\n{c}"
    );
}

#[test]
fn poly_toml_discovery_excludes_every_configured_snippet_dir() {
    let config = test_config_with_workspace_toml(
        r#"[workspace.docs.snippets]
dirs = ["docs/snippets", "docs/more-snippets"]"#,
    );
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let c = &poly_toml(&files).content;

    assert!(c.contains("\"docs/snippets/**\""), "got:\n{c}");
    assert!(c.contains("\"docs/more-snippets/**\""), "got:\n{c}");
}

/// Build a `ResolvedCrateConfig` with `[crates.e2e.snippets] output` set, and
/// optionally an extra `[workspace.*]` table, so `docs_snippets_excludes` can be
/// exercised against the tree `e2e::snippets` actually writes into, independent of
/// the unrelated `[workspace.docs.snippets] dirs` table.
fn test_config_with_e2e_snippet_output(output: &str, extra_workspace_toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python"]

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

[crates.e2e]
fixtures = "fixtures"

[crates.e2e.call]

[crates.e2e.snippets]
output = "{output}"
"#,
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// Regression for the gap that left `[e2e.snippets] output` unprotected: a repo that
/// configures e2e-generated fixture snippets but never separately configures the
/// unrelated `[workspace.docs.snippets] dirs` table (a different feature entirely --
/// MkDocs `--8<--` include discovery) must still get its snippet output tree excluded
/// from poly's discovery. Without this, poly's own `[lint.uncomment]` pass strips the
/// un-`~keep`-marked `<!-- alef:hash: -->` marker that is the ONLY ownership proof a
/// generated `.md` snippet carries, and the write guard then refuses every one of
/// those files forever. ~keep
#[test]
fn poly_toml_discovery_excludes_e2e_snippet_output_without_docs_snippets_config() {
    let config = test_config_with_e2e_snippet_output("docs-site/src/snippets/generated", "");
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"docs-site/src/snippets/generated/**\""),
        "e2e.snippets.output must be excluded from discovery even with no docs.snippets config; got:\n{c}"
    );
}

/// The negative control for the regression above: no `[e2e.snippets]` and no
/// `[workspace.docs.snippets]` configured at all must not hallucinate an exclude.
#[test]
fn poly_toml_omits_e2e_snippet_exclude_when_e2e_has_no_snippets_table() {
    let config = test_config();
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let c = &poly_toml(&files).content;

    assert!(
        !c.contains("docs-site/src/snippets"),
        "must not invent a snippet exclude with no e2e.snippets or docs.snippets config; got:\n{c}"
    );
}

/// A consumer who points both `[e2e.snippets] output` and `[workspace.docs.snippets]
/// dirs` at the identical directory must get exactly one exclude entry for it, not a
/// duplicate — the two config tables serve different features but can legitimately
/// name the same tree.
#[test]
fn poly_toml_dedupes_snippet_exclude_when_e2e_output_matches_docs_snippets_dir() {
    let config = test_config_with_e2e_snippet_output(
        "docs/snippets",
        r#"[workspace.docs.snippets]
dirs = ["docs/snippets"]"#,
    );
    let files = scaffold_poly_config(&config, &[Language::Python]);
    let c = &poly_toml(&files).content;

    // Root-scoped excludes are emitted leading-slash-anchored (`/docs/snippets/**`), same as the
    // sibling `/.alef/**` and `/fixtures/**` entries. Matching on the unanchored spelling counts
    // zero and would fail a working dedupe, so the anchored form is the one to assert. ~keep
    assert_eq!(
        c.matches("\"/docs/snippets/**\"").count(),
        1,
        "identical e2e.snippets.output and docs.snippets.dirs entries must not duplicate; got:\n{c}"
    );
}

#[test]
fn poly_toml_php_uses_mago_correctness_security() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python, Language::Php]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[lint.php.mago]") && c.contains("select = [\"correctness\", \"security\"]"),
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

/// Arrays must use taplo's 2-space indent. A 4-space indent means the freshly
/// written `poly.toml` fails the consumer's own `poly fmt --check`, and — since
/// the byte-equality skip then never fires — alef rewrites the file on every run.
#[test]
fn poly_toml_arrays_use_taplos_two_space_indent() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\n  \"**/target/**\","),
        "multi-line array entries must be indented 2 spaces; got:\n{c}"
    );
    assert!(
        !c.contains("\n    \"**/target/**\","),
        "4-space indent is not taplo-canonical and never survives `poly fmt`; got:\n{c}"
    );
}

/// poly has no native Elixir formatter, so without this opt-in it reindents
/// `.ex`/`.exs` with its own tree-sitter query and fights `mix format` forever.
#[test]
fn poly_toml_hands_elixir_to_mix_format() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("[tools.mix]") && c.contains("enabled = true"),
        "Elixir repos must declare the mix catalog formatter; got:\n{c}"
    );
    assert!(
        c.contains("root = \"packages/elixir\""),
        "mix must run from the mix project root so it reads .formatter.exs; got:\n{c}"
    );
}

#[test]
fn poly_toml_omits_mix_tool_without_elixir() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(!c.contains("[tools.mix]"), "no mix tool without Elixir; got:\n{c}");
}

/// Regression test for the discovery/hooks anchoring split: `[discovery] exclude`
/// (gitignore semantics via `ignore::overrides`) and the `[hooks.builtin]`
/// lint/fmt/file_safety excludes (whole-path semantics via `globset`) must NO
/// LONGER carry the identical literal for a bare author-supplied glob — that was
/// exactly the bug (a bare pattern unanchored in gitignore semantics, matching
/// any depth, silently became root-anchored-only under whole-path globset
/// matching, and vice versa). An explicit `**/` prefix remains the one case where
/// both matchers legitimately read the SAME string.
#[test]
fn poly_toml_extra_excludes_split_scope_between_discovery_and_hooks() {
    let config = test_config_with_poly(r#"exclude = ["vendor/generated/**", "**/third-party/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    // Bare pattern (no anchor marker) defaults to root-anchored: `[discovery]`
    // gets an explicit leading `/`.
    assert!(
        c.contains("\"/vendor/generated/**\","),
        "[discovery] exclude must root-anchor a bare author-supplied glob; got:\n{c}"
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
            "builtin at pos {builtin_pos} must carry the bare (unanchored) form"
        );
        assert!(
            !after.contains("\"/vendor/generated/**\","),
            "builtin at pos {builtin_pos} must NOT carry the [discovery] `/` anchor"
        );
    }

    // An explicit `**/` prefix is the any-depth escape hatch: both matchers read
    // the identical literal string.
    assert!(
        c.contains("\"**/third-party/**\","),
        "[discovery] exclude must preserve an explicit **/ any-depth glob verbatim; got:\n{c}"
    );
    let after_lint = &c[lint_pos..];
    assert!(
        after_lint.contains("\"**/third-party/**\","),
        "hooks builtins must carry the SAME **/ any-depth glob as [discovery]"
    );

    // Second defect fixed by the same change: alef's own `target/**` is now
    // any-depth everywhere, so it actually excludes nested `crates/*/target/**`
    // in the hooks builtins (previously bare `target/**` under globset whole-path
    // matching only matched a root-level `target/`).
    assert!(
        c.contains("\"**/target/**\","),
        "built-in target/** must be any-depth in [discovery]; got:\n{c}"
    );
    assert!(
        after_lint.contains("\"**/target/**\","),
        "built-in target/** must be any-depth in the hooks builtins too; got:\n{c}"
    );
}

/// A bare author-supplied glob with no anchor marker defaults to root-anchored:
/// `/` prefix under gitignore semantics in `[discovery]`, and bare (no prefix
/// needed — whole-path globset matching already only matches from the start of
/// the path) in the hooks builtins.
#[test]
fn poly_toml_bare_extra_exclude_is_root_anchored_in_discovery_but_bare_in_hooks() {
    let config = test_config_with_poly(r#"exclude = ["build-cache/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"/build-cache/**\","),
        "[discovery] exclude must root-anchor the bare glob with a leading /; got:\n{c}"
    );
    let lint_pos = c.find("lint = { exclude =").expect("lint builtin present");
    let after = &c[lint_pos..];
    assert!(
        after.contains("\"build-cache/**\","),
        "hooks builtin must carry the bare glob with no anchor prefix; got:\n{c}"
    );
    assert!(
        !after.contains("\"/build-cache/**\","),
        "hooks builtin must NOT carry the [discovery] `/` anchor; got:\n{c}"
    );
}

/// An explicit `**/` prefix on an author-supplied glob is the any-depth escape
/// hatch: both matchers read the identical literal string.
#[test]
fn poly_toml_any_depth_extra_exclude_is_identical_in_discovery_and_hooks() {
    let config = test_config_with_poly(r#"exclude = ["**/generated-cache/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"**/generated-cache/**\","),
        "[discovery] exclude must preserve the explicit **/ glob verbatim; got:\n{c}"
    );
    let lint_pos = c.find("lint = { exclude =").expect("lint builtin present");
    let after = &c[lint_pos..];
    assert!(
        after.contains("\"**/generated-cache/**\","),
        "hooks builtin must carry the IDENTICAL **/ glob; got:\n{c}"
    );
}

/// A leading-slash input must be re-anchored, not doubled: `/foo/**` strips its
/// own `/` before [`classify_extra`] re-adds one for `[discovery]`.
#[test]
fn poly_toml_leading_slash_extra_exclude_is_root_anchored_without_double_slash() {
    let config = test_config_with_poly(r#"exclude = ["/foo/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"/foo/**\","),
        "leading-slash input must round-trip to a single leading /; got:\n{c}"
    );
    assert!(
        !c.contains("\"//foo/**\","),
        "leading slash must be stripped before re-anchoring, not doubled; got:\n{c}"
    );
    let lint_pos = c.find("lint = { exclude =").expect("lint builtin present");
    let after = &c[lint_pos..];
    assert!(
        after.contains("\"foo/**\","),
        "hooks builtin must carry the bare (unslashed) form; got:\n{c}"
    );
}

/// A multi-segment bare pattern must default to root-anchored exactly like a
/// single-segment one — scope is tagged by [`classify_extra`], never inferred
/// from how many `/` characters the pattern contains.
#[test]
fn poly_toml_multi_segment_bare_extra_exclude_defaults_root_anchored() {
    let config = test_config_with_poly(r#"exclude = ["a/b/**"]"#);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let c = &poly_toml(&files).content;

    assert!(
        c.contains("\"/a/b/**\","),
        "a multi-segment bare glob must default root-anchored, same as a single-segment one; got:\n{c}"
    );
    let lint_pos = c.find("lint = { exclude =").expect("lint builtin present");
    let after = &c[lint_pos..];
    assert!(
        after.contains("\"a/b/**\","),
        "hooks builtin must carry the bare form; got:\n{c}"
    );
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
        fs_region.contains("\"**/target/**\","),
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

#[test]
fn poly_toml_enables_clang_format_and_ships_config_for_ffi() {
    let config = test_config();
    let files = scaffold_poly_config(&config, &[Language::Ffi]);
    let poly = files
        .iter()
        .find(|f| f.path.to_str() == Some("poly.toml"))
        .expect("poly.toml emitted");
    assert!(
        poly.content.contains("[tools.clang-format]") && poly.content.contains("enabled = true"),
        "an FFI target must enable poly's clang-format catalog tool so cbindgen headers format; got:\n{}",
        poly.content
    );
    // The whole document, with the tools table, must still be valid TOML.
    toml::from_str::<toml::Value>(&poly.content).expect("poly.toml with [tools.clang-format] must parse");

    let clang = files
        .iter()
        .find(|f| f.path.to_str() == Some(".clang-format"))
        .expect("an FFI target must ship a canonical .clang-format");
    assert!(
        clang.generated_header,
        ".clang-format must be alef-managed (overwritten every run to keep the C style uniform)"
    );
    assert!(
        clang.content.contains("BasedOnStyle: LLVM") && clang.content.contains("IndentWidth: 4"),
        "shipped .clang-format must carry the canonical LLVM/4-space style; got:\n{}",
        clang.content
    );
}

#[test]
fn poly_toml_omits_clang_format_without_ffi() {
    let config = test_config();
    let files = scaffold_poly_config(&config, &[Language::Python, Language::Node]);
    let poly = files
        .iter()
        .find(|f| f.path.to_str() == Some("poly.toml"))
        .expect("poly.toml emitted");
    assert!(
        !poly.content.contains("[tools.clang-format]"),
        "no clang-format tool table without an FFI/C-header target; got:\n{}",
        poly.content
    );
    assert!(
        !files.iter().any(|f| f.path.to_str() == Some(".clang-format")),
        "no .clang-format must be shipped without an FFI target"
    );
}

/// Repos whose CI installs only a subset of toolchains need `poly lint` to skip its
/// whole-project phase. Without a knob in `alef.toml`, that setting can only be hand-written into
/// the generated `poly.toml`, where the next scaffold run silently drops it again.
#[test]
fn emits_lint_workspace_false_when_configured() {
    let config = test_config_with_poly("lint-workspace = false");
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();

    let content = &poly_toml(&files).content;
    assert!(
        content.contains("[lint]\nworkspace = false\n"),
        "poly.toml must carry the configured [lint] workspace setting, got:\n{content}"
    );
    let lint_table = content.find("[lint]").expect("[lint] table must be emitted");
    let first_sub_table = content.find("[lint.").expect("[lint.*] sub-tables are always emitted");
    assert!(
        lint_table < first_sub_table,
        "[lint] must precede its sub-tables so it reads as a definition, not a redefinition, got:\n{content}"
    );
    let parsed: toml::Value = toml::from_str(content).expect("emitted poly.toml must be valid TOML");
    assert_eq!(parsed["lint"]["workspace"].as_bool(), Some(false));
}

/// The default must stay byte-identical to the pre-feature output: no `[lint]` table at all,
/// which lets poly apply its own default rather than alef pinning one.
#[test]
fn omits_lint_table_when_not_configured() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();

    let content = &poly_toml(&files).content;
    assert!(
        !content.contains("[lint]\nworkspace"),
        "poly.toml must not emit a [lint] workspace setting by default, got:\n{content}"
    );
}
