//! Verifies that the Java e2e codegen emits `JsonUtil.fromJson(json, Type.class)`
//! (and imports `JsonUtil`) rather than the removed `Type.fromJson(json)` per-DTO
//! factory method when `options_via` resolves to `"from_json"`.
//!
//! Real failure: `OcrTest.java`, `SmokeTest.java`, etc. called
//! `ChatCompletionRequest.fromJson(...)` — symbol not found after the Java backend
//! centralised JSON deserialization in `JsonUtil`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn make_smoke_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "basic chat smoke test".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({
            "request": {
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "hi"}]
            }
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::json!({"id": "chatcmpl-1", "object": "chat.completion", "choices": []})),
            stream_chunks: None,
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: None,
    }
}

/// Config with a `json_object` arg whose options_type is `ChatCompletionRequest`.
/// No explicit `options_via` is set so the auto-from_json default fires.
const TOML: &str = r#"
[workspace]
languages = ["java"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate.samplellm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
java_group_id = "dev.sample_crate"

[crates.e2e.call]
function = "chat"
module = "SampleLlm"
result_var = "result"
returns_result = true

[crates.e2e.call.overrides.java]
class = "SampleLlm"
function = "chat"
options_type = "ChatCompletionRequest"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

fn render_java_smoke(fixture_id: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_smoke_fixture(fixture_id)],
    }];
    let files = JavaCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains("SmokeTest.java")
        })
        .expect("SmokeTest.java is emitted")
        .content
        .clone()
}

/// The generated test file must use `JsonUtil.fromJson(...)` — NOT `Type.fromJson(...)`.
#[test]
fn from_json_arg_emits_json_util_from_json() {
    let rendered = render_java_smoke("smoke_basic");

    assert!(
        rendered.contains("JsonUtil.fromJson("),
        "must use JsonUtil.fromJson(...) for deserialization. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("ChatCompletionRequest.fromJson("),
        "must NOT emit per-DTO ChatCompletionRequest.fromJson(...) calls. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("ChatCompletionRequest.class"),
        "must pass the target class as the second argument. Rendered:\n{rendered}"
    );
}

/// JsonUtil must be imported alongside the DTO types when from_json is used.
#[test]
fn from_json_arg_imports_json_util() {
    let rendered = render_java_smoke("smoke_basic");

    assert!(
        rendered.contains("import dev.sample_crate.samplellm.JsonUtil;"),
        "must import JsonUtil from the binding package. Rendered:\n{rendered}"
    );
}
