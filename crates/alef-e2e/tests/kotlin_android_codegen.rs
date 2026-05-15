//! Regression tests for the four kotlin_android e2e codegen bugs:
//!
//! - Bug 2: construct DefaultClient via client_factory before calling methods.
//! - Bug 3: emit Kotlin property access (no parens) for data class fields.
//! - Bug 4: serialize enum values via `.name.lowercase()` instead of `.getValue()`.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::HashMap;

fn make_chat_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("chat".to_string()),
        description: "chat test".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("chat".to_string()),
        input: serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}]
        }),
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
        source: "chat.json".to_string(),
        http: None,
    }
}

fn make_chat_fixture_with_field_assertion(id: &str, field: &str, expected: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("chat".to_string()),
        description: "chat assertion test".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("chat".to_string()),
        input: serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}]
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: HashMap::new(),
        }),
        visitor: None,
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(expected.to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "chat.json".to_string(),
        http: None,
    }
}

/// Minimal alef.toml for kotlin_android e2e with java client_factory and chat call.
const TOML_WITH_JAVA_CLIENT_FACTORY: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.kreuzberg.literllm.android"
namespace = "dev.kreuzberg.literllm.android"
artifact_id = "liter-llm-android"
group_id = "dev.kreuzberg"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
result_var = "result"

[crates.e2e.calls.chat]
function = "chat"
result_var = "result"

[[crates.e2e.calls.chat.args]]
name = "request"
field = "input"
type = "json_object"
owned = true

[crates.e2e.calls.chat.overrides.java]
client_factory = "createClient"
options_type = "ChatCompletionRequest"
options_via = "from_json"

[crates.e2e.packages.kotlin_android]
name = "liter-llm"
"#;

/// alef.toml with explicit enum_fields declaring finish_reason as enum-typed.
/// Uses `fields_array` so `choices` is treated as a list (produces `.first()` access).
const TOML_WITH_ENUM_FIELDS: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.kreuzberg.literllm.android"
namespace = "dev.kreuzberg.literllm.android"
artifact_id = "liter-llm-android"
group_id = "dev.kreuzberg"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_optional = ["choices.finish_reason"]
fields_array = ["choices"]

[crates.e2e.call]
function = "chat"
result_var = "result"

[crates.e2e.calls.chat]
function = "chat"
result_var = "result"

[[crates.e2e.calls.chat.args]]
name = "request"
field = "input"
type = "json_object"
owned = true

[crates.e2e.calls.chat.overrides.java]
client_factory = "createClient"
options_type = "ChatCompletionRequest"
options_via = "from_json"
enum_fields = { "choices.finish_reason" = "FinishReason" }

[crates.e2e.packages.kotlin_android]
name = "liter-llm"
"#;

fn render_kotlin_android_chat(toml: &str, fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "chat".to_string(),
        fixtures: vec![fixture],
    }];
    let files = KotlinAndroidE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains("ChatTest.kt")
        })
        .expect("ChatTest.kt is emitted")
        .content
        .clone()
}

// ---------------------------------------------------------------------------
// Bug 2: construct DefaultClient via client_factory
// ---------------------------------------------------------------------------

/// Regression for Bug 2: when `[crates.e2e.calls.chat.overrides.java]` has
/// `client_factory = "createClient"` the kotlin_android codegen must also pick
/// that up and emit:
///   val client = LiterLlm.createClient(...)
///   val result = client.chat(...)
///   client.close()
/// rather than a flat `LiterLlm.chat(...)` call.
#[test]
fn kotlin_android_uses_java_client_factory() {
    let fixture = make_chat_fixture("chat_basic");
    let rendered = render_kotlin_android_chat(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

    assert!(
        rendered.contains("val client = LiterLlm.createClient("),
        "must construct client via factory; got:\n{rendered}"
    );
    assert!(
        rendered.contains("client.chat("),
        "must call chat on client instance; got:\n{rendered}"
    );
    assert!(
        rendered.contains("client.close()"),
        "must close the client; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("LiterLlm.chat("),
        "must NOT call chat as flat function; got:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug 3: emit Kotlin property access (no parens) for data class fields
// ---------------------------------------------------------------------------

/// Regression for Bug 3: field accessors in kotlin_android tests must use
/// Kotlin property syntax (`result.choices.first().message.content`) rather
/// than Java getter calls (`result.choices().first().message().content()`).
#[test]
fn kotlin_android_field_access_uses_property_syntax_not_getters() {
    let fixture = make_chat_fixture_with_field_assertion("chat_content", "choices.message.content", "hello");
    let rendered = render_kotlin_android_chat(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

    // Property access: no parentheses after field names.
    assert!(
        !rendered.contains(".choices()"),
        "must NOT emit .choices() getter call; got:\n{rendered}"
    );
    assert!(
        !rendered.contains(".message()"),
        "must NOT emit .message() getter call; got:\n{rendered}"
    );
    assert!(
        !rendered.contains(".content()"),
        "must NOT emit .content() getter call; got:\n{rendered}"
    );
    // Must use dot-property access.
    assert!(
        rendered.contains(".choices"),
        "must emit .choices property access; got:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug 4: serialize enum via .name.lowercase() not .getValue()
// ---------------------------------------------------------------------------

/// Regression for Bug 4: enum-typed fields in kotlin_android tests must be
/// serialized via `.name.lowercase()` (which maps `FinishReason.STOP` to the
/// wire value `"stop"`) rather than `.getValue()` (which does not exist on
/// plain Kotlin `enum class` values).
#[test]
fn kotlin_android_enum_field_uses_name_lowercase_not_get_value() {
    let fixture = make_chat_fixture_with_field_assertion("chat_finish", "choices.finish_reason", "stop");
    let rendered = render_kotlin_android_chat(TOML_WITH_ENUM_FIELDS, fixture);

    assert!(
        !rendered.contains(".getValue()"),
        "must NOT emit .getValue() on kotlin_android enum; got:\n{rendered}"
    );
    assert!(
        rendered.contains(".name") && rendered.contains(".lowercase()"),
        "must emit .name.lowercase() for enum serialization; got:\n{rendered}"
    );
}
