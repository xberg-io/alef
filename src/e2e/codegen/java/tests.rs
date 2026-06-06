use super::args::{JavaArgsContext, build_args_and_setup};
use super::visitor::{apply_java_visitor_arg, java_visitor_binding};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig, SelectWhen};
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

fn make_fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Test that resolve_call_for_fixture correctly routes to batchScrape
/// when input has batch_urls and select_when condition matches.
#[test]
fn test_java_select_when_routes_to_batch_scrape() {
    let mut calls = HashMap::new();
    calls.insert(
        "batch_scrape".to_string(),
        CallConfig {
            function: "batchScrape".to_string(),
            module: "com.example.sample_stream".to_string(),
            select_when: Some(SelectWhen {
                input_has: Some("batch_urls".to_string()),
                ..Default::default()
            }),
            ..CallConfig::default()
        },
    );

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "scrape".to_string(),
            module: "com.example.sample_stream".to_string(),
            ..CallConfig::default()
        },
        calls,
        ..E2eConfig::default()
    };

    // Fixture with batch_urls but no explicit call field should route to batch_scrape
    let fixture = make_fixture_with_input("batch_empty_urls", serde_json::json!({ "batch_urls": [] }));

    let resolved_call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    assert_eq!(resolved_call.function, "batchScrape");

    // Fixture without batch_urls should fall back to default scrape
    let fixture_no_batch =
        make_fixture_with_input("simple_scrape", serde_json::json!({ "url": "https://example.com" }));
    let resolved_default = e2e_config.resolve_call_for_fixture(
        fixture_no_batch.call.as_deref(),
        &fixture_no_batch.id,
        &fixture_no_batch.resolved_category(),
        &fixture_no_batch.tags,
        &fixture_no_batch.input,
    );
    assert_eq!(resolved_default.function, "scrape");
}

#[test]
fn handle_config_deserialization_uses_resolved_options_type() {
    let args = vec![ArgMapping {
        name: "session".to_string(),
        field: "input.config".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = make_fixture_with_input("session_fixture", serde_json::json!({ "config": { "limit": 3 } }));
    let mut teardown = String::new();
    let (setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        JavaArgsContext {
            class_name: "Sample",
            options_type: Some("SessionConfig"),
            fixture: &fixture,
            adapter_request_type: None,
            owner_handle_is_receiver: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            teardown_block: &mut teardown,
        },
    );

    let rendered = setup.join("\n");
    assert_eq!(args_str, "session");
    assert!(rendered.contains("MAPPER.readValue(\"{\\\"limit\\\":3}\", SessionConfig.class)"));
    assert!(rendered.contains("Sample.createSession(sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

#[test]
fn java_visitor_arg_uses_trait_bridge_options_metadata() {
    use crate::core::config::{BridgeBinding, TraitBridgeConfig};

    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            type_alias: Some("RenderHandle".to_string()),
            param_name: Some("renderer".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("RenderOptions".to_string()),
            options_field: Some("callback".to_string()),
            context_type: Some("RenderContext".to_string()),
            result_type: Some("RenderDecision".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let binding = java_visitor_binding(&config, &[], None, None).expect("visitor binding");
    assert_eq!(binding.options_type, "RenderOptions");
    assert_eq!(binding.options_field, "callback");
    assert_eq!(binding.trait_type, "Renderer");
    assert_eq!(binding.context_type, "RenderContext");
    assert_eq!(binding.result_type, "RenderDecision");

    let args = apply_java_visitor_arg(&mut Vec::new(), "html, null", &[], "visitor", &binding);
    assert_eq!(args, "html, new RenderOptions().withCallback(visitor)");
    assert!(!args.contains("DefaultOptions"));
}
