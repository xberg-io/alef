//! Docs-snippet rendering for streaming adapters declared with an `owner_type`.

use super::test_support::line_containing;
use super::*;
use crate::e2e::config::CallConfig;

/// Synthetic streaming `owner_type` adapter whose facade signature takes exactly
/// the declared `request` param, matching the shape declared by
/// `[[crates.adapters]]` in `alef.toml`. Mirrors
/// `test_method::tests::streaming_owner_adapter`.
fn streaming_owner_adapter(function_name: &str, owner_type: &str) -> crate::core::config::extras::AdapterConfig {
    crate::core::config::extras::AdapterConfig {
        name: function_name.to_string(),
        pattern: crate::core::config::extras::AdapterPattern::Streaming,
        core_path: format!("test_core::{function_name}"),
        params: vec![crate::core::config::extras::AdapterParam {
            name: "request".to_string(),
            ty: "sample::StreamRequest".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: None,
        owner_type: Some(owner_type.to_string()),
        item_type: Some("Item".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: Vec::new(),
    }
}

fn streaming_owner_call() -> CallConfig {
    CallConfig {
        function: "stream_items".into(),
        result_var: "result".into(),
        args: vec![
            crate::e2e::config::ArgMapping {
                name: "handle".into(),
                field: "input.handle".into(),
                arg_type: "handle".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "url".into(),
                field: "input.url".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ],
        ..CallConfig::default()
    }
}

fn streaming_owner_fixture() -> Fixture {
    Fixture {
        id: "stream_basic".into(),
        description: "Stream items".into(),
        input: serde_json::json!({ "handle": {}, "url": "https://example.com" }),
        ..Fixture::default()
    }
}

/// Regression for #149: an owner_type streaming adapter's declared request
/// param (`[[crates.adapters.params]]`) must be built from the fixture input
/// and bound to a local `val request = ...` before the call references it —
/// mirroring `test_method.rs`'s identical construction for the generated
/// JUnit test. Before this fix `render_snippet_body` never resolved the
/// adapter's `streaming_request` at all: the handle-typed arg was still
/// treated as the call receiver (so it never reached `args`), but nothing
/// rebuilt or bound the request in its place, leaving the call either
/// missing its required argument or passing an un-rebuilt raw value instead
/// of the declared `request` identifier this test pins.
#[test]
fn kotlin_snippet_binds_the_declared_request_before_the_call() {
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_owner_adapter("stream_items", "Engine")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &streaming_owner_fixture(),
        &E2eConfig {
            call: streaming_owner_call(),
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        false,
    )
    .expect("snippet renders");

    assert_eq!(
        line_containing(&body, "val request ="),
        r#"val request = mapper.readValue("{\"url\":\"https://example.com\"}", StreamRequest::class.java)"#
    );
    assert_eq!(
        line_containing(&body, "val result ="),
        "val result = handle.streamItems(request)"
    );
    assert!(
        !body.contains("\\\"handle\\\""),
        "owner handle config must not leak into the request JSON:\n{body}"
    );
    assert!(
        !body.contains("streamItems(\"https://example.com\")"),
        "the raw url must not be passed positionally in place of the declared request:\n{body}"
    );
}

#[test]
fn kotlin_android_snippet_binds_the_declared_request_before_the_call() {
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_owner_adapter("stream_items", "Engine")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &streaming_owner_fixture(),
        &E2eConfig {
            call: streaming_owner_call(),
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        true,
    )
    .expect("snippet renders");

    assert_eq!(
        line_containing(&body, "val request ="),
        r#"val request = mapper.readValue("{\"url\":\"https://example.com\"}", StreamRequest::class.java)"#
    );
    assert_eq!(
        line_containing(&body, "val result ="),
        "val result = handle.streamItems(request)"
    );
    assert!(
        !body.contains("\\\"handle\\\""),
        "owner handle config must not leak into the request JSON:\n{body}"
    );
}

fn cold_flow_snippet(presentation: bool, expects_error: bool) -> String {
    let mut fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "cold_flow", "description": "Cold flow", "input": null,
        "docs": {"topic": "streaming", "presentation": {"operations": [
            {"op": "show", "path": "usage.total_tokens", "display": true}
        ]}}
    }))
    .expect("fixture");
    if !presentation {
        fixture.docs = None;
    }
    if expects_error {
        fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
    }
    let config: E2eConfig = serde_json::from_value(serde_json::json!({
        "call": {"function": "chat_stream", "streaming": true, "overrides": {
            "java": {"client_factory": "create_client"}
        }},
        "result_fields": ["usage"], "fields_optional": ["usage"]
    }))
    .expect("config");
    render_snippet_body(
        &fixture,
        &config,
        &ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        },
        &[],
        &[],
        true,
    )
    .expect("snippet")
}

#[test]
fn android_cold_flow_collects_once_inside_client_lifetime() {
    for presentation in [false, true] {
        let body = cold_flow_snippet(presentation, false);
        assert!(body.contains(".use { client ->\n"), "{body}");
        assert!(body.contains("client.chatStream().collect { resultChunk ->"), "{body}");
        assert_eq!(body.matches(".collect {").count(), 1, "{body}");
        assert!(!body.contains("toList("), "{body}");
        assert!(!body.contains("result.usage"), "{body}");
        assert!(
            body.contains(if presentation {
                "println(resultChunk.usage?.totalTokens)"
            } else {
                "println(resultChunk)"
            }),
            "{body}"
        );
    }
}

#[test]
fn android_deferred_flow_error_is_collected_inside_the_catch_scope() {
    let body = cold_flow_snippet(false, true);
    let attempt = body.find("try {").expect("error handler");
    let lifetime = body.find(".use { client ->").expect("client lifetime");
    let collect = body.find(".collect {").expect("cold flow collected");
    let catch = body.find("catch (error: Exception)").expect("error catch");
    assert!(attempt < lifetime && lifetime < collect && collect < catch, "{body}");
    assert_eq!(body.matches(".collect {").count(), 1, "{body}");
}
