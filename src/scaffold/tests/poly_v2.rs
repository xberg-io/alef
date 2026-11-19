//! The generated `poly.toml` follows the v2 schema poly 0.36 reads: `[engines.*]` tables, no v1
//! `[lint.*]`/`[fmt.*]`/`[per-file-ignores]` tables, typos kept on, and the opt-in engines a
//! consumer enables under `[workspace.poly]`.

use super::*;
use crate::core::config::NewAlefConfig;

fn config_with_poly(poly_toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python"]

{poly_toml}

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid alef.toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

fn poly_toml_for(config: &ResolvedCrateConfig, languages: &[Language]) -> (String, toml::Value) {
    let content = scaffold_poly_config(config, languages)
        .into_iter()
        .find(|file| file.path == std::path::Path::new("poly.toml"))
        .expect("scaffold emits poly.toml")
        .content;
    let parsed = toml::from_str(&content).expect("generated poly.toml parses");
    (content, parsed)
}

#[test]
fn emits_only_v2_tables_for_every_language() {
    let languages = [
        Language::Python,
        Language::Php,
        Language::Ffi,
        Language::Elixir,
        Language::Go,
        Language::Java,
        Language::Node,
    ];
    let config = config_with_poly(
        "[workspace.poly]\nlint-workspace = false\n\n[workspace.poly.typos.extend-words]\nflate = \"flate\"\n\n\
         [workspace.poly.uncomment]\n\n[workspace.poly.shell-formatter]\nindent-width = 2\n",
    );
    let (content, parsed) = poly_toml_for(&config, &languages);

    for v1_key in ["lint", "fmt", "per-file-ignores", "rules"] {
        assert!(parsed.get(v1_key).is_none(), "v1 `{v1_key}` emitted:\n{content}");
    }
    assert_eq!(parsed["required_version"].as_str(), Some(">=0.36.0"));
    assert_eq!(parsed["workspace"]["run"].as_str(), Some("never"));
    let engines = &parsed["engines"];
    assert!(engines["ruff"]["lint"]["select"].is_array());
    assert!(engines["mago"]["lint"]["select"].is_array());
    assert!(engines["rumdl"]["lang"]["markdown"]["lint"]["disable"].is_array());
    assert!(engines["rumdl"]["lang"]["markdown"]["format"]["disable"].is_array());
    assert_eq!(engines["typos"]["enabled"].as_bool(), Some(true));
    assert_eq!(engines["typos"]["extend_words"]["flate"].as_str(), Some("flate"));
    assert_eq!(engines["uncomment"]["enabled"].as_bool(), Some(true));
    assert_eq!(engines["shfmt"]["format"]["indent_width"].as_integer(), Some(2));
    assert!(parsed["per_file_ignores"]["**/tests/**"].is_array());
}

/// typos is opt-in under v2 but ran by default under v1; dropping the table would silently turn
/// spell checking off in every consumer.
#[test]
fn keeps_typos_on_without_any_typos_config() {
    let (content, parsed) = poly_toml_for(&ResolvedCrateConfig::default(), &[Language::Rust]);
    assert_eq!(
        parsed["engines"]["typos"]["enabled"].as_bool(),
        Some(true),
        "typos must stay on:\n{content}"
    );
}

#[test]
fn pins_the_clang_format_release_poly_downloads_for_ffi_headers() {
    let (content, parsed) = poly_toml_for(&ResolvedCrateConfig::default(), &[Language::Ffi]);
    let clang_format = &parsed["tools"]["clang-format"];
    assert_eq!(clang_format["enabled"].as_bool(), Some(true), "{content}");
    assert_eq!(clang_format["version"].as_str(), Some("21.1.8"), "{content}");
    assert_eq!(clang_format["install"].as_str(), Some("auto"), "{content}");
}

#[test]
fn omits_opt_in_engines_unless_configured() {
    let (content, parsed) = poly_toml_for(&ResolvedCrateConfig::default(), &[Language::Rust]);
    for engine in ["skif", "tsrs", "harper"] {
        assert!(parsed["engines"].get(engine).is_none(), "{engine} emitted:\n{content}");
    }
}

#[test]
fn emits_skif_tsrs_and_harper_when_configured() {
    let config = config_with_poly(
        "[workspace.poly.skif]\ndir = \"docs-site\"\n\n[workspace.poly.tsrs]\ndir = \"docs-site\"\n\
         project = \"docs-site/tsconfig.app.json\"\n\n[workspace.poly.harper]\n",
    );
    let (content, parsed) = poly_toml_for(&config, &[Language::Python]);
    let engines = &parsed["engines"];

    assert_eq!(engines["skif"]["enabled"].as_bool(), Some(true), "{content}");
    assert_eq!(engines["skif"]["dir"].as_str(), Some("docs-site"));
    assert_eq!(engines["tsrs"]["dir"].as_str(), Some("docs-site"));
    assert_eq!(engines["tsrs"]["project"].as_str(), Some("docs-site/tsconfig.app.json"));
    assert_eq!(engines["harper"]["enabled"].as_bool(), Some(true));
    let ignore: Vec<&str> = engines["harper"]["ignore"]
        .as_array()
        .expect("harper ignore list")
        .iter()
        .map(|rule| rule.as_str().expect("rule name"))
        .collect();
    assert_eq!(
        ignore,
        [
            "UseTitleCase",
            "OxfordComma",
            "SplitWords",
            "OrthographicConsistency",
            "ExpandMemoryShorthands",
            "ExpandStandardInputAndOutput",
            "ExpandConfiguration",
            "ExpandMinimum",
        ]
    );
    assert!(
        content.contains("no-Oxford-comma style"),
        "the default list keeps its reason:\n{content}"
    );
}

#[test]
fn harper_ignore_list_replaces_the_default() {
    let config = config_with_poly("[workspace.poly.harper]\nignore = [\"LongSentences\"]\n");
    let (_, parsed) = poly_toml_for(&config, &[Language::Python]);
    let ignore = parsed["engines"]["harper"]["ignore"].as_array().expect("ignore list");
    assert_eq!(ignore.len(), 1);
    assert_eq!(ignore[0].as_str(), Some("LongSentences"));
}
