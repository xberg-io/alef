//! Verifies the Kotlin e2e codegen emits client-object instantiation when
//! `CallOverride.client_factory` is set, and falls back to flat function calls
//! when it is absent.
//!
//! Mirrors the Go/Zig codegen pattern: when `client_factory` is set the
//! generated test must instantiate `DefaultClient`, call the method on it, and
//! close it. When absent, the object-level function is called directly —
//! the kreuzberg flat-function style must remain untouched.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::kotlin::KotlinE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::HashMap;

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "request": { "model": "gpt-4o", "messages": [] } }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: HashMap::new(),
        }),
        visitor: None,
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

fn make_group(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![make_fixture(id)],
    }
}

fn render_kotlin_smoke(toml: &str, fixture_id: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id)];
    let files = KotlinE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    // Find the smoke test file (not the build.gradle.kts).
    files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains("SmokeTest.kt") || p.contains("smoke_test.kt")
        })
        .expect("SmokeTest.kt is emitted")
        .content
        .clone()
}

const BASE_TOML_WITH_FLAT_OVERRIDE: &str = r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.kreuzberg.literllm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "dev.kreuzberg.literllm.LiterLlm"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "LiterLlm"
function = "chat"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// When `client_factory` is set, the generated test must:
///   1. Create a client via the facade factory `<class>.<client_factory>(apiKey, baseUrl)`
///   2. Call the method on the client instance (`client.chat(...)`)
///   3. Close the client via `client.close()`
///   4. Wire `MOCK_SERVER_URL` into the baseUrl
#[test]
fn with_client_factory_emits_client_instantiation() {
    let toml = r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.kreuzberg.literllm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "dev.kreuzberg.literllm.LiterLlm"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "LiterLlm"
function = "chat"
client_factory = "createClient"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;
    let rendered = render_kotlin_smoke(toml, "smoke_basic");

    assert!(
        rendered.contains("LiterLlm.createClient(apiKey"),
        "must instantiate client via facade factory. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("client.chat("),
        "must call chat on client instance. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("client.close()"),
        "must close the client. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("MOCK_SERVER_URL"),
        "must include MOCK_SERVER_URL in baseUrl. Rendered:\n{rendered}"
    );
}

/// When `client_factory` is absent, the generator falls back to the flat
/// object-function call pattern. This ensures kreuzberg regression-free.
#[test]
fn without_client_factory_emits_flat_function_call() {
    let rendered = render_kotlin_smoke(BASE_TOML_WITH_FLAT_OVERRIDE, "smoke_basic");

    assert!(
        rendered.contains("LiterLlm.chat("),
        "must call object-level function directly. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("DefaultClient("),
        "must NOT instantiate DefaultClient when client_factory is absent. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("client.close()"),
        "must NOT close any client when client_factory is absent. Rendered:\n{rendered}"
    );
}
