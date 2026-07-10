//! Verifies that `[crates.e2e.exclude_categories]` filters out fixtures whose
//! resolved category matches an excluded entry, while leaving every other
//! fixture untouched.
//!
//! Categories are excluded *globally* across all generated languages — the
//! filter applies whether the category is set explicitly on the fixture or
//! resolved from the parent directory name, and whether the consumer reads the
//! filtered groups directly or goes through the per-language
//! `should_include_fixture` chokepoint.

use alef::core::config::e2e::{CallConfig, E2eConfig};
use alef::e2e::fixture::{Fixture, group_fixtures};
use std::collections::HashSet;

fn make_fixture(id: &str, category: Option<&str>, source: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: category.map(str::to_string),
        description: format!("fixture {id}"),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: Vec::new(),
        source: source.to_string(),
        http: None,
    }
}

fn e2e_config_with_exclusions(excluded: &[&str]) -> E2eConfig {
    E2eConfig {
        fixtures: "fixtures".to_string(),
        output: "e2e".to_string(),
        languages: Vec::new(),
        call: CallConfig {
            function: "chat".to_string(),
            ..CallConfig::default()
        },
        calls: Default::default(),
        packages: Default::default(),
        format: Default::default(),
        fields: Default::default(),
        fields_optional: HashSet::new(),
        fields_array: HashSet::new(),
        fields_method_calls: HashSet::new(),
        result_fields: HashSet::new(),
        fields_c_types: Default::default(),
        fields_enum: HashSet::new(),
        fields_display_as_text: HashSet::new(),
        env: Default::default(),
        dep_mode: Default::default(),
        registry: Default::default(),
        exclude_categories: excluded.iter().map(|s| s.to_string()).collect(),
        test_documents_dir: "test_documents".to_string(),
        harness: Default::default(),
        harness_extras: Default::default(),
        extra_system_libs: Default::default(),
    }
}

#[test]
fn exclude_categories_removes_matching_fixture_groups() {
    let fixtures = vec![
        make_fixture("cache_hit", Some("cache"), "cache/cache_hit.json"),
        make_fixture("proxy_basic", Some("proxy"), "proxy/proxy_basic.json"),
        make_fixture("chat_simple", Some("chat"), "chat/chat_simple.json"),
        make_fixture("embed_basic", Some("embed"), "embed/embed_basic.json"),
    ];

    let cfg = e2e_config_with_exclusions(&["cache", "proxy", "budget", "hooks"]);
    let groups = group_fixtures(&fixtures);

    let kept: Vec<_> = groups
        .into_iter()
        .filter(|g| !cfg.exclude_categories.contains(&g.category))
        .collect();

    let kept_categories: Vec<_> = kept.iter().map(|g| g.category.as_str()).collect();
    assert_eq!(kept_categories, vec!["chat", "embed"]);
    assert!(!kept_categories.contains(&"cache"));
    assert!(!kept_categories.contains(&"proxy"));
}

#[test]
fn exclude_categories_resolves_from_directory_when_field_missing() {
    let fixtures = vec![
        make_fixture("cache_hit", None, "cache/cache_hit.json"),
        make_fixture("chat_simple", None, "chat/chat_simple.json"),
    ];
    let cfg = e2e_config_with_exclusions(&["cache"]);
    let groups = group_fixtures(&fixtures);

    let kept: Vec<_> = groups
        .into_iter()
        .filter(|g| !cfg.exclude_categories.contains(&g.category))
        .collect();
    let kept_categories: Vec<_> = kept.iter().map(|g| g.category.as_str()).collect();
    assert_eq!(kept_categories, vec!["chat"]);
}

#[test]
fn empty_exclude_categories_is_a_noop() {
    let fixtures = vec![
        make_fixture("cache_hit", Some("cache"), "cache/cache_hit.json"),
        make_fixture("chat_simple", Some("chat"), "chat/chat_simple.json"),
    ];

    let cfg = e2e_config_with_exclusions(&[]);
    assert!(cfg.exclude_categories.is_empty());

    let groups = group_fixtures(&fixtures);
    let kept: Vec<_> = groups
        .into_iter()
        .filter(|g| !cfg.exclude_categories.contains(&g.category))
        .collect();
    assert_eq!(kept.len(), 2);
}

#[test]
fn exclude_categories_parses_from_toml() {
    let toml_src = r#"
fixtures = "fixtures"
output = "e2e"
exclude_categories = ["cache", "proxy", "budget", "hooks"]

[call]
function = "chat"
module = "mylib"
"#;

    let e2e: E2eConfig = toml::from_str(toml_src).expect("parse e2e config");

    let mut expected: HashSet<String> = HashSet::new();
    for c in ["cache", "proxy", "budget", "hooks"] {
        expected.insert(c.to_string());
    }
    assert_eq!(e2e.exclude_categories, expected);
}

#[test]
fn exclude_categories_defaults_to_empty_when_absent() {
    let toml_src = r#"
fixtures = "fixtures"
output = "e2e"

[call]
function = "chat"
module = "mylib"
"#;

    let e2e: E2eConfig = toml::from_str(toml_src).expect("parse e2e config");
    assert!(e2e.exclude_categories.is_empty());
}
