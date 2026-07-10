//! Verifies the Kotlin e2e codegen emits a direct `String.contains(...)`
//! check for `contains` / `contains_all` / `not_contains` assertions on
//! string-typed fields, instead of casting to `List<String>`.
//!
//! Background: older Kotlin codegen unconditionally emitted
//! `(field as List<String>).contains(value)` for collection-style assertions.
//! That crashed for plain `String` fields and always failed for complex list
//! item types. The current renderer uses direct substring checks for strings
//! and stringifies collection fields before the case-insensitive contains check.
//!
//! Regression originally reported via demo-client v1.4 CI run:
//!   `TranscribeTest.test_transcribe_basic_audio`
//! which asserted `result.text` contains a phrase and crashed at runtime.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin::KotlinE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn make_fixture_with_contains_on_string() -> FixtureGroup {
    FixtureGroup {
        category: "transcribe".to_string(),
        fixtures: vec![Fixture {
            id: "basic_audio".to_string(),
            category: Some("transcribe".to_string()),
            description: "transcribe an audio clip and assert phrase".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "request": { "model": "whisper-1" } }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![
                Assertion {
                    assertion_type: "contains".to_string(),
                    field: Some("text".to_string()),
                    value: Some(serde_json::Value::String("hello world".to_string())),
                    values: None,
                    method: None,
                    check: None,
                    args: None,
                    return_type: None,
                },
                Assertion {
                    assertion_type: "contains_all".to_string(),
                    field: Some("text".to_string()),
                    value: None,
                    values: Some(vec![
                        serde_json::Value::String("hello".to_string()),
                        serde_json::Value::String("world".to_string()),
                    ]),
                    method: None,
                    check: None,
                    args: None,
                    return_type: None,
                },
                Assertion {
                    assertion_type: "not_contains".to_string(),
                    field: Some("text".to_string()),
                    value: Some(serde_json::Value::String("goodbye".to_string())),
                    values: None,
                    method: None,
                    check: None,
                    args: None,
                    return_type: None,
                },
            ],
            source: "transcribe.json".to_string(),
            http: None,
        }],
    }
}

fn base_toml() -> &'static str {
    r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample_crate.samplellm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields = { text = "text" }
result_fields = ["text"]

[crates.e2e.call]
function = "transcribe"
module = "dev.sample_crate.samplellm.SampleLlm"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "SampleLlm"
function = "transcribe"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#
}

#[test]
fn contains_on_string_field_does_not_cast_to_list() {
    let cfg: NewAlefConfig = toml::from_str(base_toml()).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_fixture_with_contains_on_string()];
    let files = KotlinE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.contains("TranscribeTest.kt") || p.contains("transcribe") && p.ends_with(".kt")
        })
        .expect("transcribe test file should be emitted");
    let content = &test_file.content;

    assert!(
        !content.contains("as List<String>"),
        "must NOT cast a String field to List<String>. Rendered:\n{content}"
    );

    assert!(
        content.contains(".contains(\"hello world\")"),
        "must emit a substring check on the text field. Rendered:\n{content}"
    );
    assert!(
        content.contains(".contains(\"hello\")"),
        "must emit substring check for contains_all entry 'hello'. Rendered:\n{content}"
    );
    assert!(
        content.contains(".contains(\"world\")"),
        "must emit substring check for contains_all entry 'world'. Rendered:\n{content}"
    );
    assert!(
        content.contains(".contains(\"goodbye\")"),
        "must emit substring check for not_contains entry 'goodbye'. Rendered:\n{content}"
    );
}

#[test]
fn contains_on_list_field_stringifies_collection() {
    let toml_src = r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample_crate.samplellm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields = { tags = "tags" }
fields_array = ["tags"]
result_fields = ["tags"]

[crates.e2e.call]
function = "list_tags"
module = "dev.sample_crate.samplellm.SampleLlm"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "SampleLlm"
function = "listTags"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "tags".to_string(),
        fixtures: vec![Fixture {
            id: "basic".to_string(),
            category: Some("tags".to_string()),
            description: "tags".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "request": {} }),
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
                assertion_type: "contains".to_string(),
                field: Some("tags".to_string()),
                value: Some(serde_json::Value::String("python".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "tags.json".to_string(),
            http: None,
        }],
    }];

    let files = KotlinE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    let combined = files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(".kt"))
        .map(|f| format!("// === {} ===\n{}", f.path.display(), f.content))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !combined.contains("as List<String>"),
        "must not use an unchecked List<String> cast for collection contains assertions. Rendered:\n{combined}"
    );
    assert!(
        combined.contains(r#"result.tags().toString().lowercase().contains("python".toString().lowercase())"#),
        "must stringify collection fields for contains assertions (assertion source: `contains` on field `tags` \
         declared in `fields_array`). Rendered:\n{combined}"
    );
}
