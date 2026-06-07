use super::*;
use std::collections::{HashMap, HashSet};

fn empty_e2e_with_test_documents(dir: &str) -> E2eConfig {
    E2eConfig {
        test_documents_dir: dir.to_string(),
        ..Default::default()
    }
}

#[test]
fn test_documents_dir_default_is_test_documents() {
    let cfg: E2eConfig = toml::from_str("[call]\nfunction = \"f\"\n").expect("minimal TOML must deserialize");
    assert_eq!(cfg.test_documents_dir, "test_documents");
}

#[test]
fn test_documents_dir_explicit_override_wins() {
    let cfg: E2eConfig = toml::from_str("test_documents_dir = \"fixture_files\"\n[call]\nfunction = \"f\"\n")
        .expect("explicit override must deserialize");
    assert_eq!(cfg.test_documents_dir, "fixture_files");
}

#[test]
fn test_documents_relative_from_at_lang_root_returns_two_dots_up() {
    let cfg = empty_e2e_with_test_documents("test_documents");
    assert_eq!(cfg.test_documents_relative_from(0), "../../test_documents");
}

#[test]
fn test_documents_relative_from_at_spec_depth_returns_three_dots_up() {
    let cfg = empty_e2e_with_test_documents("test_documents");
    assert_eq!(cfg.test_documents_relative_from(1), "../../../test_documents");
}

#[test]
fn test_documents_relative_from_at_two_subdirs_deep_returns_four_dots_up() {
    let cfg = empty_e2e_with_test_documents("test_documents");
    assert_eq!(cfg.test_documents_relative_from(2), "../../../../test_documents");
}

#[test]
fn test_documents_relative_uses_configured_dir_name() {
    let cfg = empty_e2e_with_test_documents("fixture_files");
    assert_eq!(cfg.test_documents_relative_from(0), "../../fixture_files");
    assert_eq!(cfg.test_documents_relative_from(1), "../../../fixture_files");
}

#[test]
fn select_when_with_no_discriminators_never_matches() {
    let sel = SelectWhen::default();
    assert!(!sel.matches("any_id", "any_category", &[], &serde_json::Value::Null));
}

#[test]
fn select_when_input_has_matches_non_null_key() {
    let sel = SelectWhen {
        input_has: Some("batch_urls".to_string()),
        ..Default::default()
    };
    let input = serde_json::json!({ "batch_urls": [] });
    assert!(sel.matches("fid", "cat", &[], &input));
    let empty_input = serde_json::json!({ "url": "x" });
    assert!(!sel.matches("fid", "cat", &[], &empty_input));
}

#[test]
fn select_when_category_matches_exactly() {
    let sel = SelectWhen {
        category: Some("crawl".to_string()),
        ..Default::default()
    };
    assert!(sel.matches("any_id", "crawl", &[], &serde_json::Value::Null));
    assert!(!sel.matches("any_id", "scrape", &[], &serde_json::Value::Null));
}

#[test]
fn select_when_id_prefix_matches() {
    let sel = SelectWhen {
        id_prefix: Some("batch_crawl_".to_string()),
        ..Default::default()
    };
    assert!(sel.matches("batch_crawl_events", "any", &[], &serde_json::Value::Null));
    assert!(!sel.matches("batch_scrape_basic", "any", &[], &serde_json::Value::Null));
}

#[test]
fn select_when_id_glob_handles_star() {
    let sel = SelectWhen {
        id_glob: Some("crawl_stream*".to_string()),
        ..Default::default()
    };
    assert!(sel.matches("crawl_stream_basic", "any", &[], &serde_json::Value::Null));
    assert!(!sel.matches("batch_crawl_stream", "any", &[], &serde_json::Value::Null));
}

#[test]
fn select_when_tag_matches_any_tag_in_list() {
    let sel = SelectWhen {
        tag: Some("streaming".to_string()),
        ..Default::default()
    };
    let tags = vec!["smoke".to_string(), "streaming".to_string()];
    assert!(sel.matches("fid", "cat", &tags, &serde_json::Value::Null));
    assert!(!sel.matches("fid", "cat", &["smoke".to_string()], &serde_json::Value::Null));
}

#[test]
fn select_when_multiple_discriminators_anded() {
    let sel = SelectWhen {
        category: Some("stream".to_string()),
        id_prefix: Some("batch_crawl_stream".to_string()),
        ..Default::default()
    };
    assert!(sel.matches("batch_crawl_stream_events", "stream", &[], &serde_json::Value::Null));
    // Wrong category fails even though prefix matches
    assert!(!sel.matches("batch_crawl_stream_events", "crawl", &[], &serde_json::Value::Null));
    // Wrong prefix fails even though category matches
    assert!(!sel.matches("crawl_stream_basic", "stream", &[], &serde_json::Value::Null));
}

#[test]
fn select_when_deserializes_legacy_input_has_only() {
    let toml_src = r#"
            [call]
            function = "scrape"

            [calls.batch_scrape]
            function = "batch_scrape"
            select_when = { input_has = "batch_urls" }
        "#;
    let cfg: E2eConfig = toml::from_str(toml_src).expect("legacy input_has must deserialize");
    let sel = cfg.calls["batch_scrape"].select_when.as_ref().unwrap();
    assert_eq!(sel.input_has.as_deref(), Some("batch_urls"));
    assert!(sel.category.is_none());
    assert!(sel.id_prefix.is_none());
}

#[test]
fn select_when_deserializes_compound_discriminators() {
    let toml_src = r#"
            [call]
            function = "scrape"

            [calls.batch_crawl_stream]
            function = "batch_crawl_stream"
            select_when = { category = "stream", id_prefix = "batch_crawl_stream" }
        "#;
    let cfg: E2eConfig = toml::from_str(toml_src).expect("compound select_when must deserialize");
    let sel = cfg.calls["batch_crawl_stream"].select_when.as_ref().unwrap();
    assert_eq!(sel.category.as_deref(), Some("stream"));
    assert_eq!(sel.id_prefix.as_deref(), Some("batch_crawl_stream"));
}

#[test]
fn resolve_call_for_fixture_routes_by_category_then_falls_back() {
    let mut calls = HashMap::new();
    calls.insert(
        "crawl".to_string(),
        CallConfig {
            function: "crawl".to_string(),
            select_when: Some(SelectWhen {
                category: Some("crawl".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let cfg = E2eConfig {
        call: CallConfig {
            function: "scrape".to_string(),
            ..Default::default()
        },
        calls,
        ..Default::default()
    };
    let input = serde_json::json!({ "url": "https://example.com" });
    let resolved = cfg.resolve_call_for_fixture(None, "crawl_basic", "crawl", &[], &input);
    assert_eq!(resolved.function, "crawl");
    let resolved = cfg.resolve_call_for_fixture(None, "scrape_basic", "scrape", &[], &input);
    assert_eq!(resolved.function, "scrape");
}

// --- effective_* resolver helpers ---

#[test]
fn effective_result_fields_returns_global_when_call_is_empty() {
    let mut global = HashSet::new();
    global.insert("url".to_string());
    let cfg = E2eConfig {
        result_fields: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_result_fields(&call), &global);
}

#[test]
fn effective_result_fields_call_override_wins_over_global() {
    let mut global = HashSet::new();
    global.insert("url".to_string());
    let mut per_call = HashSet::new();
    per_call.insert("pages".to_string());
    per_call.insert("final_url".to_string());
    let cfg = E2eConfig {
        result_fields: global,
        ..Default::default()
    };
    let call = CallConfig {
        result_fields: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_result_fields(&call), &per_call);
}

#[test]
fn effective_fields_returns_global_when_call_is_empty() {
    let mut global = HashMap::new();
    global.insert("metadata.title".to_string(), "metadata.document.title".to_string());
    let cfg = E2eConfig {
        fields: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields(&call), &global);
}

#[test]
fn effective_fields_call_override_wins_over_global() {
    let mut global = HashMap::new();
    global.insert("a".to_string(), "b".to_string());
    let mut per_call = HashMap::new();
    per_call.insert("x".to_string(), "y".to_string());
    let cfg = E2eConfig {
        fields: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields(&call), &per_call);
}

#[test]
fn effective_fields_optional_returns_global_when_call_is_empty() {
    let mut global = HashSet::new();
    global.insert("segments".to_string());
    let cfg = E2eConfig {
        fields_optional: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields_optional(&call), &global);
}

#[test]
fn effective_fields_optional_call_override_wins_over_global() {
    let mut global = HashSet::new();
    global.insert("segments".to_string());
    let mut per_call = HashSet::new();
    per_call.insert("pages".to_string());
    let cfg = E2eConfig {
        fields_optional: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields_optional: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields_optional(&call), &per_call);
}

#[test]
fn effective_fields_array_returns_global_when_call_is_empty() {
    let mut global = HashSet::new();
    global.insert("choices".to_string());
    let cfg = E2eConfig {
        fields_array: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields_array(&call), &global);
}

#[test]
fn effective_fields_array_call_override_wins_over_global() {
    let mut global = HashSet::new();
    global.insert("choices".to_string());
    let mut per_call = HashSet::new();
    per_call.insert("pages".to_string());
    let cfg = E2eConfig {
        fields_array: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields_array: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields_array(&call), &per_call);
}

#[test]
fn effective_fields_method_calls_returns_global_when_call_is_empty() {
    let mut global = HashSet::new();
    global.insert("metadata.format".to_string());
    let cfg = E2eConfig {
        fields_method_calls: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields_method_calls(&call), &global);
}

#[test]
fn effective_fields_method_calls_call_override_wins_over_global() {
    let mut global = HashSet::new();
    global.insert("metadata.format".to_string());
    let mut per_call = HashSet::new();
    per_call.insert("pages.status".to_string());
    let cfg = E2eConfig {
        fields_method_calls: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields_method_calls: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields_method_calls(&call), &per_call);
}

#[test]
fn effective_fields_enum_returns_global_when_call_is_empty() {
    let mut global = HashSet::new();
    global.insert("choices.finish_reason".to_string());
    let cfg = E2eConfig {
        fields_enum: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields_enum(&call), &global);
}

#[test]
fn effective_fields_enum_call_override_wins_over_global() {
    let mut global = HashSet::new();
    global.insert("choices.finish_reason".to_string());
    let mut per_call = HashSet::new();
    per_call.insert("assets.category".to_string());
    let cfg = E2eConfig {
        fields_enum: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields_enum: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields_enum(&call), &per_call);
}

#[test]
fn effective_fields_c_types_returns_global_when_call_is_empty() {
    let mut global = HashMap::new();
    global.insert("conversion_result.metadata".to_string(), "HtmlMetadata".to_string());
    let cfg = E2eConfig {
        fields_c_types: global.clone(),
        ..Default::default()
    };
    let call = CallConfig::default();
    assert_eq!(cfg.effective_fields_c_types(&call), &global);
}

#[test]
fn effective_fields_c_types_call_override_wins_over_global() {
    let mut global = HashMap::new();
    global.insert("conversion_result.metadata".to_string(), "HtmlMetadata".to_string());
    let mut per_call = HashMap::new();
    per_call.insert("crawl_result.pages".to_string(), "PageResult".to_string());
    let cfg = E2eConfig {
        fields_c_types: global,
        ..Default::default()
    };
    let call = CallConfig {
        fields_c_types: per_call.clone(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_fields_c_types(&call), &per_call);
}

// --- HomebrewCliTest / PackageRef.cli_tests ---

#[test]
fn package_ref_cli_tests_default_is_empty() {
    let pkg = PackageRef::default();
    assert!(pkg.cli_tests.is_empty());
}

#[test]
fn package_ref_cli_tests_deserializes_from_toml() {
    let toml_src = r#"
[call]
function = "f"

[[registry.packages.homebrew.cli_tests]]
name = "version"
command = "$CLI_FORMULA --version"
expect_contains = "$VERSION"

[[registry.packages.homebrew.cli_tests]]
name = "help"
command = "$CLI_FORMULA --help"
"#;
    let cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    let tests = &cfg.registry.packages["homebrew"].cli_tests;
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].name, "version");
    assert_eq!(tests[0].command, "$CLI_FORMULA --version");
    assert_eq!(tests[0].expect_contains.as_deref(), Some("$VERSION"));
    assert_eq!(tests[1].name, "help");
    assert_eq!(tests[1].command, "$CLI_FORMULA --help");
    assert!(tests[1].expect_contains.is_none());
}

#[test]
fn resolve_package_cli_tests_registry_wins_over_base() {
    let toml_src = r#"
[call]
function = "f"

[packages.homebrew]
cli_formula = "mytool"

[[packages.homebrew.cli_tests]]
name = "base-check"
command = "$CLI_FORMULA base"

[[registry.packages.homebrew.cli_tests]]
name = "registry-check"
command = "$CLI_FORMULA registry"
expect_contains = "ok"
"#;
    let mut cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    cfg.dep_mode = DependencyMode::Registry;
    let resolved = cfg.resolve_package("homebrew").expect("must resolve");
    assert_eq!(resolved.cli_tests.len(), 1);
    assert_eq!(resolved.cli_tests[0].name, "registry-check");
}

#[test]
fn resolve_package_platform_hashes_registry_wins_over_base() {
    let toml_src = r#"
[call]
function = "f"

[packages.zig.platform_hashes]
linux-x86_64 = "base-linux"

[registry.packages.zig.platform_hashes]
linux-x86_64 = "registry-linux"
macos-arm64 = "registry-macos"
"#;
    let mut cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    cfg.dep_mode = DependencyMode::Registry;

    let resolved = cfg.resolve_package("zig").expect("must resolve");

    assert_eq!(
        resolved.platform_hashes.get("linux-x86_64").map(String::as_str),
        Some("registry-linux")
    );
    assert_eq!(
        resolved.platform_hashes.get("macos-arm64").map(String::as_str),
        Some("registry-macos")
    );
}

#[test]
fn resolve_package_cli_tests_falls_back_to_base_when_registry_empty() {
    let toml_src = r#"
[call]
function = "f"

[[packages.homebrew.cli_tests]]
name = "base-check"
command = "$CLI_FORMULA base"

[registry.packages.homebrew]
cli_formula = "mytool"
"#;
    let mut cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    cfg.dep_mode = DependencyMode::Registry;
    let resolved = cfg.resolve_package("homebrew").expect("must resolve");
    assert_eq!(resolved.cli_tests.len(), 1);
    assert_eq!(resolved.cli_tests[0].name, "base-check");
}

#[test]
fn package_ref_ffi_formula_is_optional_with_no_default() {
    // ffi_formula must NOT default to anything when absent.
    let toml_src = r#"
[call]
function = "f"

[registry.packages.homebrew]
cli_formula = "mytool"
tap = "myorg/tap"
"#;
    let cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    let pkg = &cfg.registry.packages["homebrew"];
    assert!(
        pkg.ffi_formula.is_none(),
        "ffi_formula must be None when not configured"
    );
}

#[test]
fn effective_resolver_helpers_deserialize_from_toml() {
    let toml = r#"
[call]
function = "scrape"
result_fields = ["url", "markdown"]
fields_enum = ["status"]

[call.fields]
"meta.title" = "meta.document.title"

[call.fields_c_types]
"scrape_result.meta" = "MetaResult"
"#;
    let cfg: E2eConfig = toml::from_str(toml).expect("must deserialize");
    let call = &cfg.call;
    assert!(cfg.effective_result_fields(call).contains("url"));
    assert!(cfg.effective_result_fields(call).contains("markdown"));
    assert!(cfg.effective_fields_enum(call).contains("status"));
    assert_eq!(
        cfg.effective_fields(call).get("meta.title").map(String::as_str),
        Some("meta.document.title")
    );
    assert_eq!(
        cfg.effective_fields_c_types(call)
            .get("scrape_result.meta")
            .map(String::as_str),
        Some("MetaResult")
    );
}

#[test]
fn call_streaming_recipe_deserializes_item_type() {
    let toml = r#"
[call]
function = "stream_events"
streaming = { item_type = "Event" }
"#;
    let cfg: E2eConfig = toml::from_str(toml).expect("must deserialize inline streaming recipe");
    assert_eq!(cfg.call.streaming_enabled(), None);
    assert_eq!(cfg.call.streaming_item_type(), Some("Event"));

    let toml = r#"
[call]
function = "stream_events"

[call.streaming]
enabled = true
item_type = "Event"
"#;
    let cfg: E2eConfig = toml::from_str(toml).expect("must deserialize streaming table recipe");
    assert_eq!(cfg.call.streaming_enabled(), Some(true));
    assert_eq!(cfg.call.streaming_item_type(), Some("Event"));
}
