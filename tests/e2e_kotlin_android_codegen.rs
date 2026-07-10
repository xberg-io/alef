//! Regression tests for the kotlin_android e2e codegen bugs:
//!
//! - Bug 2: construct DefaultClient via client_factory before calling methods.
//! - Bug 3: emit Kotlin property access (no parens) for data class fields.
//! - Bug 4: serialize enum values via `.name.lowercase()` instead of `.getValue()`.
//! - Bug 5: streaming collect uses `Flow.toList()` not `asSequence().toList()`,
//!   and chunk assertions use Kotlin property access throughout.

use alef::core::config::NewAlefConfig;
use alef::core::template_versions::maven;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn make_chat_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("chat".to_string()),
        description: "chat test".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("chat".to_string()),
        input: serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}]
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
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
        setup: Vec::new(),
        call: Some("chat".to_string()),
        input: serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}]
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
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
name = "demo-client"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.democlient.android"
namespace = "dev.sample_crate.democlient.android"
artifact_id = "demo-client-android"
group_id = "dev.sample_crate"

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
name = "demo-client"
"#;

/// alef.toml with explicit enum_fields declaring finish_reason as enum-typed.
/// Uses `fields_array` so `choices` is treated as a list (produces `.first()` access).
const TOML_WITH_ENUM_FIELDS: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.democlient.android"
namespace = "dev.sample_crate.democlient.android"
artifact_id = "demo-client-android"
group_id = "dev.sample_crate"

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
name = "demo-client"
"#;

/// alef.toml for kotlin_android streaming e2e via chatStream call.
const TOML_WITH_STREAMING: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.democlient.android"
namespace = "dev.sample_crate.democlient.android"
artifact_id = "demo-client-android"
group_id = "dev.sample_crate"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat_stream"
result_var = "result"

[crates.e2e.calls.chat_stream]
function = "chat_stream"
result_var = "result"

[[crates.e2e.calls.chat_stream.args]]
name = "request"
field = "input"
type = "json_object"
owned = true

[crates.e2e.calls.chat_stream.overrides.java]
client_factory = "createClient"
options_type = "ChatCompletionRequest"
options_via = "from_json"

[crates.e2e.packages.kotlin_android]
name = "demo-client"
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
        .generate(&groups, &e2e, &resolved, &[], &[])
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

fn make_streaming_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("chat".to_string()),
        description: "chat stream test".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("chat_stream".to_string()),
        input: serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }),
        mock_response: Some(MockResponse {
            status: 200,
            body: None,
            stream_chunks: Some(vec![
                serde_json::json!({"choices": [{"delta": {"content": "hello"}, "finish_reason": null}]}),
                serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
            ]),
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("stream_content".to_string()),
                value: Some(serde_json::Value::String("hello".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("stream_complete".to_string()),
                value: Some(serde_json::Value::Bool(true)),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
        ],
        source: "chat.json".to_string(),
        http: None,
    }
}

fn render_kotlin_android_streaming(fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML_WITH_STREAMING).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "chat".to_string(),
        fixtures: vec![fixture],
    }];
    let files = KotlinAndroidE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
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

/// Regression for Bug 2: when `[crates.e2e.calls.chat.overrides.java]` has
/// `client_factory = "createClient"` the kotlin_android codegen must also pick
/// that up and emit:
///   val client = DemoClient.createClient(...)
///   val result = client.chat(...)
///   client.close()
/// rather than a flat `DemoClient.chat(...)` call.
#[test]
fn kotlin_android_uses_java_client_factory() {
    let fixture = make_chat_fixture("chat_basic");
    let rendered = render_kotlin_android_chat(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

    assert!(
        rendered.contains("val client = DemoClient.createClient("),
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
        !rendered.contains("DemoClient.chat("),
        "must NOT call chat as flat function; got:\n{rendered}"
    );
}

/// Regression for Bug 3: field accessors in kotlin_android tests must use
/// Kotlin property syntax (`result.choices.first().message.content`) rather
/// than Java getter calls (`result.choices().first().message().content()`).
#[test]
fn kotlin_android_field_access_uses_property_syntax_not_getters() {
    let fixture = make_chat_fixture_with_field_assertion("chat_content", "choices.message.content", "hello");
    let rendered = render_kotlin_android_chat(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

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
    assert!(
        rendered.contains(".choices"),
        "must emit .choices property access; got:\n{rendered}"
    );
}

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

/// Regression for Bug 5: kotlin_android streaming tests must collect a
/// `Flow<T>` with `result.toList()` (kotlinx.coroutines suspend extension),
/// not with `result.asSequence().toList()` (which only applies to Java
/// `Iterator<T>`).  Chunk field assertions must also use Kotlin property access
/// (`it.choices?.firstOrNull()?.delta?.content`) rather than Java getter calls
/// (`it.choices()?.firstOrNull()?.delta()?.content()`).
#[test]
fn kotlin_android_streaming_collect_uses_flow_to_list_not_as_sequence() {
    let fixture = make_streaming_fixture("chat_stream_basic");
    let rendered = render_kotlin_android_streaming(fixture);

    assert!(
        rendered.contains(".toList()"),
        "must emit .toList() to collect the Flow; got:\n{rendered}"
    );
    assert!(
        !rendered.contains(".asSequence()"),
        "must NOT emit .asSequence() (Java Iterator pattern); got:\n{rendered}"
    );

    assert!(
        !rendered.contains(".choices()"),
        "must NOT emit .choices() getter call in chunk assertions; got:\n{rendered}"
    );
    assert!(
        !rendered.contains(".delta()"),
        "must NOT emit .delta() getter call in chunk assertions; got:\n{rendered}"
    );
    assert!(
        !rendered.contains(".finishReason()"),
        "must NOT emit .finishReason() getter call in chunk assertions; got:\n{rendered}"
    );

    assert!(
        rendered.contains(".choices"),
        "must emit .choices property access in chunk assertions; got:\n{rendered}"
    );

    assert!(
        rendered.contains("import kotlinx.coroutines.flow.toList"),
        "must import kotlinx.coroutines.flow.toList; got:\n{rendered}"
    );
}

fn generate_kotlin_android_files(toml: &str, fixture: Fixture) -> Vec<alef::core::backend::GeneratedFile> {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "chat".to_string(),
        fixtures: vec![fixture],
    }];
    KotlinAndroidE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds")
}

fn kotlin_android_chat_test_path() -> PathBuf {
    Path::new("src")
        .join("test")
        .join("kotlin")
        .join("dev")
        .join("sample_crate")
        .join("democlient")
        .join("android")
        .join("e2e")
        .join("ChatTest.kt")
}

/// Regression for D: `kotlin_android` codegen must emit a host-JVM
/// `src/test/` source set so tests can run without an Android emulator.
#[test]
fn kotlin_android_emits_host_jvm_test_source_set() {
    let fixture = make_chat_fixture("chat_basic");
    let files = generate_kotlin_android_files(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

    let all_paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    let chat_test_path = kotlin_android_chat_test_path();
    let test_file = files.iter().find(|f| f.path.ends_with(&chat_test_path));
    let file = test_file.unwrap_or_else(|| {
        panic!(
            "src/test/.../ChatTest.kt must be emitted; got files:\n{}",
            all_paths.join("\n")
        )
    });
    let content = &file.content;

    assert!(
        content.contains("class ChatTest"),
        "host-JVM test file must declare ChatTest; got:\n{content}"
    );
    assert!(
        content.contains("org.junit.jupiter.api.Test"),
        "host-JVM test file must import JUnit 5 Test; got:\n{content}"
    );
    assert!(
        !content.contains("AndroidJUnit4"),
        "host-JVM test file must not depend on AndroidJUnit4; got:\n{content}"
    );
}

/// Regression for D: the emitted `build.gradle.kts` must apply the Android Gradle Plugin
/// so that the `android { }` DSL resolves at Kotlin script compile time. AGP 9+ supplies
/// built-in Kotlin support, so the explicit `kotlin("android")` plugin line is omitted
/// for current pins. The e2e app runs host-JVM tests, so it should not require Managed
/// Devices or an Android emulator.
#[test]
fn kotlin_android_build_gradle_applies_android_gradle_plugin() {
    let fixture = make_chat_fixture("chat_basic");
    let files = generate_kotlin_android_files(TOML_WITH_JAVA_CLIENT_FACTORY, fixture);

    let build_gradle = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("build.gradle.kts"))
        .expect("build.gradle.kts must be emitted");
    let content = &build_gradle.content;

    assert!(
        content.contains("com.android.library"),
        "build.gradle.kts must apply id(\"com.android.library\"); got:\n{content}"
    );
    let agp_major = maven::ANDROID_GRADLE_PLUGIN
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .expect("ANDROID_GRADLE_PLUGIN must start with a major version");
    if agp_major >= 9 {
        assert!(
            !content.contains("kotlin(\"android\")"),
            "AGP 9+ must not re-apply kotlin(\"android\"); got:\n{content}"
        );
    } else {
        assert!(
            content.contains("kotlin(\"android\")"),
            "AGP 8.x must apply kotlin(\"android\"); got:\n{content}"
        );
    }
    assert!(
        content.contains("unitTests"),
        "build.gradle.kts must configure host-JVM unit tests; got:\n{content}"
    );
    assert!(
        !content.contains("ManagedVirtualDevice") && !content.contains("managedDevices"),
        "build.gradle.kts must not require managed Android devices; got:\n{content}"
    );
}
