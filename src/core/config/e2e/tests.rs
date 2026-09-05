use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

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
fn error_field_aliases_deserialize_without_weakening_strict_config() {
    let cfg: E2eConfig = toml::from_str(
        r#"
[call]
function = "run"

[error_field_aliases]
status = "status_code"
"#,
    )
    .expect("declared error field aliases must deserialize");

    assert_eq!(
        cfg.error_field_aliases.get("status").map(String::as_str),
        Some("status_code")
    );
    assert!(toml::from_str::<E2eConfig>("unknown = true\n[call]\nfunction = \"run\"\n").is_err());
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
    let mut calls = BTreeMap::new();
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
fn effective_function_prefers_the_per_language_override_over_the_base() {
    let call: CallConfig = toml::from_str(
        r#"
function = "clear_reranker_backends"

[overrides.wasm]
function = "clearRerankerBackends"
"#,
    )
    .expect("a call with a per-language function override must deserialize");

    assert_eq!(call.effective_function("wasm"), Some("clearRerankerBackends"));
    assert_eq!(call.effective_function("python"), Some("clear_reranker_backends"));
}

#[test]
fn effective_function_reads_an_override_when_the_base_names_nothing() {
    let call: CallConfig = toml::from_str(
        r#"
function = ""

[overrides.wasm]
function = "clearRerankerBackends"
"#,
    )
    .expect("a call that names itself only per language must deserialize");

    assert_eq!(call.effective_function("wasm"), Some("clearRerankerBackends"));
    assert_eq!(
        call.effective_function("python"),
        None,
        "an override for one language says nothing about another"
    );
}

#[test]
fn effective_function_treats_a_blank_name_as_no_name() {
    let call: CallConfig = toml::from_str(
        r#"
function = "   "

[overrides.wasm]
function = ""
"#,
    )
    .expect("blank function names must deserialize");

    assert_eq!(
        call.effective_function("wasm"),
        None,
        "a blank override must fall through, and a blank base must not become the empty symbol"
    );
}

#[test]
fn core_lookup_name_keeps_the_base_so_existing_consumers_do_not_move() {
    let call: CallConfig = toml::from_str(
        r#"
function = "stream_items"

[overrides.csharp]
function = "StreamItemsAsync"
"#,
    )
    .expect("a call with a per-language function override must deserialize");

    assert_eq!(
        call.core_lookup_name("csharp").as_deref(),
        Some("stream_items"),
        "adapters and the IR are keyed by the Rust name, so a populated base must win over the \
         binding's own spelling"
    );
    assert_eq!(call.core_lookup_name("python").as_deref(), Some("stream_items"));
}

#[test]
fn core_lookup_name_falls_back_to_the_override_when_the_base_names_nothing() {
    let call: CallConfig = toml::from_str(
        r#"
function = ""

[overrides.csharp]
function = "ClearRerankerBackends"

[overrides.ruby]
function = "clear_reranker_backends"
"#,
    )
    .expect("a call that names itself only per language must deserialize");

    assert_eq!(
        call.core_lookup_name("csharp").as_deref(),
        Some("clear_reranker_backends"),
        "the C# override is the only name there is, and it must be snake-cased back into the \
         spelling the adapter and IR tables use"
    );
    assert_eq!(
        call.core_lookup_name("ruby").as_deref(),
        Some("clear_reranker_backends")
    );
}

#[test]
fn core_lookup_name_reports_no_name_rather_than_looking_up_the_empty_string() {
    let call: CallConfig = toml::from_str(
        r#"
function = "  "

[overrides.csharp]
function = "ClearRerankerBackends"
"#,
    )
    .expect("blank function names must deserialize");

    assert_eq!(
        call.core_lookup_name("python"),
        None,
        "a call that names nothing for this language must return None; looking up `\"\"` matches \
         no adapter and silently derives names from the empty string"
    );
}

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
expect_contains = "1.2.3"

[[registry.packages.homebrew.cli_tests]]
name = "help"
command = "$CLI_FORMULA --help"
"#;
    let cfg: E2eConfig = toml::from_str(toml_src).expect("must deserialize");
    let tests = &cfg.registry.packages["homebrew"].cli_tests;
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].name, "version");
    assert_eq!(tests[0].command, "$CLI_FORMULA --version");
    assert_eq!(tests[0].expect_contains.as_deref(), Some("1.2.3"));
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

// --- deny_unknown_fields regression coverage ---

/// Regression test for the root-cause defect: field-classification keys
/// (`fields_optional`, `fields_array`, `fields_enum`, `result_fields`,
/// `fields_method_calls`) belong directly on `[e2e]`. A consumer that
/// misnests them one level deeper — under `[e2e.snippets]` — used to have
/// them silently discarded twice over: once by `SnippetConfig` lacking
/// `deny_unknown_fields`, and (had they instead been misnested under some
/// other already-known `[e2e]` sub-table, or mistyped at the top level)
/// again by `E2eConfig` itself lacking it. This test pins the top-level case:
/// an unrecognised key directly under `[e2e]` must be a hard error.
///
/// Without `#[serde(deny_unknown_fields)]` on `E2eConfig` this test fails:
/// `toml::from_str` returns `Ok(..)` and `results_fields` (the typo) is
/// silently dropped instead of surfacing as a config error.
#[test]
fn unknown_top_level_e2e_key_is_rejected_not_silently_dropped() {
    let toml_src = r#"
        [call]
        function = "f"

        results_fields = ["pages"]
    "#;
    let err = toml::from_str::<E2eConfig>(toml_src)
        .expect_err("an unrecognised top-level [e2e] key must be rejected, not silently dropped");
    let message = err.to_string();
    assert!(
        message.contains("results_fields"),
        "error must name the offending key: {message}"
    );
    assert!(
        message.contains("unknown field"),
        "error must be serde's unknown-field diagnostic: {message}"
    );
}

/// `[e2e.call]` is a required table, so a real `alef.toml` always runs `result_var` through
/// serde and the field arrives populated. This pins that entry point: it is the reason an
/// unset `result_var` is not reachable by *omitting* the key.
#[test]
fn an_omitted_result_var_deserialises_to_the_documented_default() {
    let call: CallConfig = toml::from_str("function = \"process\"\n").expect("a call without result_var must load");
    assert_eq!(call.result_var, "result");
    assert_eq!(call.effective_result_var(), "result");
}

/// The one origin serde does not cover. `#[serde(default)]` fires on an absent key, never on a
/// present-but-blank one, and nothing validates the value, so `result_var = ""` reaches the
/// emitters as an empty identifier. Resolving it here is what keeps that out of generated code.
#[test]
fn an_explicitly_blank_result_var_resolves_to_the_documented_default() {
    for blank in ["\"\"", "\"   \""] {
        let call: CallConfig =
            toml::from_str(&format!("result_var = {blank}\n")).expect("a blank result_var must load");
        assert_eq!(
            call.effective_result_var(),
            "result",
            "a blank result_var ({blank}) must resolve to the documented default"
        );
    }
}

/// The control for both tests above: resolution must not overwrite a name someone chose.
#[test]
fn an_explicit_result_var_is_returned_verbatim() {
    let call: CallConfig = toml::from_str("result_var = \"captured\"\n").expect("an explicit result_var must load");
    assert_eq!(call.result_var, "captured");
    assert_eq!(call.effective_result_var(), "captured");
    assert_eq!(
        CallConfig {
            result_var: "captured".into(),
            ..CallConfig::default()
        }
        .effective_result_var(),
        "captured"
    );
}

/// A programmatically built `CallConfig` must carry exactly what a TOML-loaded one carries.
///
/// `result_var` is the field where the two disagreed: the serde default said "result" and the
/// derived `Default` said `""`, and every consumer that read the field rather than resolving it
/// picked up whichever derivation its caller happened to use. Comparing the two whole structs
/// rather than that one field is deliberate — the next field with a non-trivial serde default
/// would otherwise reintroduce the same split with no signal. ~keep
#[test]
fn default_matches_the_serde_defaults() {
    let from_serde: CallConfig = toml::from_str("").expect("every CallConfig field carries a serde default");
    assert_eq!(
        serde_json::to_value(CallConfig::default()).expect("CallConfig serialises"),
        serde_json::to_value(&from_serde).expect("CallConfig serialises"),
        "`Default for CallConfig` has drifted from the serde defaults; a field's default is now \
         two different values depending on how the call was built"
    );
    assert_eq!(
        from_serde.result_var, "result",
        "the comparison above is only meaningful while the serde default is a real name"
    );
}
