use super::snippet::{SnippetContext, render_snippet_body};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render(language: &str, streaming: bool) -> String {
    render_with_item(language, streaming, false)
}

fn render_with_item(language: &str, streaming: bool, typed: bool) -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "usage", "description": "Show usage", "input": {},
        "docs": {"topic": "streaming"},
        "assertions": [{"type": "not_empty", "field": "usage.total_tokens"}]
    }))
    .unwrap();
    let mut e2e: E2eConfig = serde_json::from_value(serde_json::json!({
        "call": {"function": "stream", "module": "sample", "result_var": "response",
                 "async": true, "streaming": streaming},
        "fields_optional": ["usage"]
    }))
    .unwrap();
    if typed {
        e2e.call.streaming = Some(crate::e2e::config::StreamingConfig::Recipe(
            crate::e2e::config::StreamingRecipe {
                item_type: Some("Chunk".into()),
                enabled: Some(true),
            },
        ));
    }
    let types = if typed {
        vec![crate::core::ir::TypeDef {
            name: "Chunk".into(),
            ..Default::default()
        }]
    } else {
        vec![]
    };
    render_snippet_body(SnippetContext {
        lang: language,
        fixture: &fixture,
        module: "sample",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &types,
        enums: &[],
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &ResolvedCrateConfig::default(),
    })
}

#[test]
fn node_stream_shows_each_chunk_with_custom_result_name() {
    let body = render("node", true);
    assert!(body.contains("for await (const responseChunk of response)"), "{body}");
    assert!(body.contains("responseChunk.usage?.totalTokens"), "{body}");
    assert!(!body.contains("console.log(response.usage"), "{body}");
}

#[test]
fn wasm_stream_uses_null_terminated_next_and_releases_iterator() {
    let body = render("wasm", true);
    assert!(body.contains("const responseChunk = await response.next();"), "{body}");
    assert!(body.contains("if (responseChunk === null) break;"), "{body}");
    assert!(body.contains("response.free();"), "{body}");
    assert!(body.contains("responseChunk.usage?.totalTokens"), "{body}");
    assert!(!body.contains("for await"), "{body}");
}

#[test]
fn ordinary_response_is_not_driven_as_a_stream() {
    for language in ["node", "wasm"] {
        let body = render(language, false);
        assert!(body.contains("console.log(response.usage?.totalTokens)"), "{body}");
        assert!(
            !body.contains("response.next()") && !body.contains("for await"),
            "{body}"
        );
    }
}

#[test]
fn wasm_stream_types_and_releases_known_wrapped_items() {
    let body = render_with_item("wasm", true, true);
    assert!(body.contains("WasmChunk"), "{body}");
    assert!(
        body.contains("const responseChunk: WasmChunk | null = await response.next();"),
        "{body}"
    );
    assert!(body.contains("responseChunk.free();"), "{body}");
    assert!(
        body.find("responseChunk.free();") < body.find("response.free();"),
        "{body}"
    );
}
