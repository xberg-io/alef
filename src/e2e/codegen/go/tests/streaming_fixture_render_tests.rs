//! Coverage for streaming-fixture test-function rendering: the collect-loop snippet a
//! `chunks`/streaming fixture must emit, across both the plain streaming-config path and
//! the client-factory + `json_object`-arg override path.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

use super::test_function::{GoTestFunctionContext, render_test_function};

#[test]
fn test_streaming_fixture_emits_collect_snippet() {
    // A streaming fixture should emit `stream, err :=` and the collect loop.
    let streaming_fixture_json = r#"{
            "id": "basic_stream",
            "description": "basic streaming test",
            "call": "chat_stream",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}]
            },
            "assertions": [
                {"type": "count_min", "field": "chunks", "value": 1}
            ]
        }"#;
    let fixture: Fixture = serde_json::from_str(streaming_fixture_json).unwrap();
    assert!(fixture.is_streaming_mock(), "fixture should be detected as streaming");

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat_stream".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            r#async: true,
            streaming: Some(crate::core::config::e2e::StreamingConfig::Recipe(
                crate::core::config::e2e::StreamingRecipe {
                    item_type: Some("StreamChunk".to_string()),
                    ..Default::default()
                },
            )),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(out.contains("stream, err :="), "should use stream binding, got:\n{out}");
    assert!(
        out.contains("for chunk := range stream.Chan() {"),
        "should range over the iterator's Chan(), got:\n{out}"
    );
    assert!(
        out.contains("if streamErr := stream.Err(); streamErr != nil {")
            && out.contains("t.Fatalf(\"stream failed: %v\", streamErr)"),
        "a mid-stream failure must fail the test via Err(), not close silently, got:\n{out}"
    );
    // #448: the generated streaming start method now requires a leading `ctx context.Context`.
    assert!(
        out.contains("context.Background()"),
        "expected the call to pass context.Background() as its first argument, got:\n{out}"
    );
    assert!(
        out.contains(".ChatStreamWithContext("),
        "expected the cancellable API, got:\n{out}"
    );
}
#[test]
fn test_streaming_with_client_factory_and_json_arg() {
    // Covers no returns_result on the call, json_object args
    // (binding_returns_error=true), and client_factory from the Go call override.
    use crate::core::config::e2e::{ArgMapping, CallOverride};
    let streaming_fixture_json = r#"{
            "id": "basic_stream_client",
            "description": "basic streaming test with client",
            "call": "chat_stream",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}]
            },
            "assertions": [
                {"type": "count_min", "field": "chunks", "value": 1}
            ]
        }"#;
    let fixture: Fixture = serde_json::from_str(streaming_fixture_json).unwrap();
    assert!(fixture.is_streaming_mock(), "fixture should be detected as streaming");

    let go_override = CallOverride {
        client_factory: Some("CreateClient".to_string()),
        ..Default::default()
    };

    let mut call_overrides = std::collections::HashMap::new();
    call_overrides.insert("go".to_string(), go_override);

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat_stream".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: false, // NOT true — like real demo-client
            r#async: true,
            streaming: Some(crate::core::config::e2e::StreamingConfig::Recipe(
                crate::core::config::e2e::StreamingRecipe {
                    item_type: Some("StreamChunk".to_string()),
                    ..Default::default()
                },
            )),
            args: vec![ArgMapping {
                name: "request".to_string(),
                field: "input".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            overrides: call_overrides,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let mut out = String::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &crate::core::config::ResolvedCrateConfig::default(),
            type_defs: &[],
            enums: &[],
            errors: &[],
            functions: &[],
        },
    );

    assert!(out.contains("stream, err :="), "should use stream binding, got:\n{out}");
    assert!(
        out.contains("for chunk := range stream.Chan() {"),
        "should range over the iterator's Chan(), got:\n{out}"
    );
    assert!(
        out.contains("if streamErr := stream.Err(); streamErr != nil {")
            && out.contains("t.Fatalf(\"stream failed: %v\", streamErr)"),
        "a mid-stream failure must fail the test via Err(), not close silently, got:\n{out}"
    );
    assert!(
        out.contains("context.Background()"),
        "expected the call to pass context.Background() as its first argument, got:\n{out}"
    );
    assert!(
        out.contains(".ChatStreamWithContext("),
        "expected the cancellable API, got:\n{out}"
    );
}
