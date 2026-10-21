//! Verifies zig streaming fixtures are emitted when they can be, and that when they
//! cannot be, the omission leaves an artefact instead of vanishing.
//!
//! The defect: every fixture whose resolved call was `streaming = true` was dropped by
//! a hardcoded zig-only filter, and a category left empty by it hit a bare `continue`.
//! No file, no log, no gate failure — `alef verify`, the empty-category check in
//! `e2e/validate.rs` and `fixture_inclusion` all still reported zig as included, so a
//! consumer whose config explicitly routed `crawl_stream` to zig got nothing and was
//! never told.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn streaming_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: Some("stream".to_string()),
        description: "streaming fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "request": { "model": "gpt-4o", "messages": [] } }),
        mock_response: Some(alef::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "stream.json".to_string(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

fn render_stream_category(toml: &str) -> Vec<alef::core::backend::GeneratedFile> {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "stream".to_string(),
        fixtures: vec![streaming_fixture("crawl_stream_events")],
    }];
    ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds")
}

fn stream_test_file(files: &[alef::core::backend::GeneratedFile]) -> Option<String> {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("stream_test.zig"))
        .map(|f| f.content.clone())
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "samplellm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat_stream"
module = "demo_client"
result_var = "result"

[crates.e2e.call.streaming]
enabled = true

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// A streaming fixture routed to zig with a `client_factory` configured must actually
/// produce a test. This is the half that was unreachable: the filter dropped it before
/// the emitter — which is fully written — ever saw it.
#[test]
fn streaming_fixture_is_emitted_when_a_client_factory_is_configured() {
    let toml = format!(
        r#"{BASE_TOML}
[crates.e2e.call.overrides.zig]
client_factory = "create_client"
result_is_json_struct = true
"#
    );
    let files = render_stream_category(&toml);

    let rendered = stream_test_file(&files).expect("stream_test.zig must be emitted for a streaming fixture");
    assert!(
        rendered.contains("crawl_stream_events"),
        "the streaming fixture must appear in the emitted suite. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("SkipZigTest"),
        "an emittable streaming fixture must produce a real test, not the skip placeholder. Rendered:\n{rendered}"
    );
}

/// Without a `client_factory` the streaming fixture genuinely cannot be emitted — the
/// call would be rendered as a free function and zig exposes streaming only as a method
/// on the handle, so the suite would not compile. It must still leave an artefact naming
/// what was dropped. This is the control: it proves the fix above is not simply
/// "emit everything regardless".
#[test]
fn an_unemittable_streaming_category_leaves_a_placeholder_not_silence() {
    let files = render_stream_category(BASE_TOML);

    let rendered = stream_test_file(&files).expect("an excluded category must still emit a placeholder file");
    assert!(
        rendered.contains("SkipZigTest"),
        "placeholder must be a skipping zig test. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("crawl_stream_events"),
        "placeholder must name the fixture that was dropped. Rendered:\n{rendered}"
    );
}
