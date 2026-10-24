//! A streaming fixture that asserts `usage.total_tokens` must read the usage off the LAST CHUNK of
//! the drained stream. `usage` is a stream-level root in the shared streaming resolver, but the
//! deep-path split and the streaming-field predicate omitted it, so most backends read
//! `result.usage` (the stream, not a response: a compile error or a runtime `undefined`) and a few
//! emitted a skip comment, which left a green test that checked nothing.
//!
//! Each test asserts FIRST that a usage assertion was emitted at all, then that it is not a skip
//! marker and does not read the stream result.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::codegen::java::JavaCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::codegen::python::PythonE2eCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::codegen::typescript::TypeScriptCodegen;
use alef::e2e::codegen::wasm::WasmCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    build_config_with(
        r#"
[crates.e2e.call.streaming]
enabled = true
item_type = "ChatCompletionChunk"
"#,
    )
}

fn build_config_with(streaming: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat_stream"
module = "MyLib"
result_var = "result"
args = [
  {{ name = "req", field = "input", type = "json_object" }},
]
{streaming}"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn usage_group(field: &str) -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "streaming".to_string(),
        fixtures: vec![Fixture {
            id: "stream_with_usage".to_string(),
            category: Some("streaming".to_string()),
            description: "stream carries usage in the final chunk".to_string(),
            input: serde_json::json!({ "model": "m", "stream": true }),
            assertions: vec![
                Assertion {
                    assertion_type: "greater_than_or_equal".to_string(),
                    field: Some("chunks.length".to_string()),
                    value: Some(serde_json::json!(1)),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some(field.to_string()),
                    value: Some(serde_json::json!(18)),
                    ..Assertion::default()
                },
            ],
            source: "streaming/stream_with_usage.json".to_string(),
            ..Fixture::default()
        }],
    }]
}

fn generated<C: E2eCodegen>(codegen: &C, field: &str) -> String {
    let (e2e, resolved) = build_config();
    generated_with(codegen, &e2e, &resolved, field)
}

fn generated_with<C: E2eCodegen>(
    codegen: &C,
    e2e: &alef::e2e::config::E2eConfig,
    resolved: &alef::core::config::ResolvedCrateConfig,
    field: &str,
) -> String {
    let files = codegen
        .generate(&usage_group(field), e2e, resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .filter(|f| f.content.contains("stream_with_usage") || f.content.contains("StreamWithUsage"))
        .map(|f| f.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines that mention the asserted counter, ignoring case and separators.
fn usage_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|l| l.to_lowercase().replace('_', "").contains("totaltokens"))
        .map(|l| l.trim().to_string())
        .collect()
}

fn assert_reads_last_chunk<C: E2eCodegen>(codegen: &C, lang: &str) {
    let content = generated(codegen, "usage.total_tokens");
    let lines = usage_lines(&content);
    assert!(
        !lines.is_empty(),
        "{lang}: no usage assertion was emitted at all. Emitted:\n{content}"
    );
    let skipped = content
        .lines()
        .any(|l| l.contains("skipped") && l.contains("usage.total_tokens"));
    assert!(
        !skipped,
        "{lang}: the usage assertion is a skip marker, so the test proves nothing. Emitted:\n{content}"
    );
    for line in &lines {
        let norm = line.to_lowercase().replace('_', "");
        assert!(
            !norm.contains("result.usage")
                && !norm.contains("result->getusage")
                && !norm.contains("chunks.usage")
                && !norm.contains("result.usage()"),
            "{lang}: the usage assertion reads the stream result instead of the last chunk: `{line}`"
        );
    }
    assert!(
        lines.iter().any(|l| l.to_lowercase().contains("chunk")),
        "{lang}: no usage line reads the collected chunks. Emitted:\n{content}"
    );
}

#[test]
fn rust_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&RustE2eCodegen, "rust");
}

#[test]
fn node_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&TypeScriptCodegen, "node");
}

#[test]
fn wasm_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&WasmCodegen, "wasm");
}

#[test]
fn go_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&GoCodegen, "go");
}

#[test]
fn java_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&JavaCodegen, "java");
}

#[test]
fn elixir_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&ElixirCodegen, "elixir");
}

#[test]
fn python_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&PythonE2eCodegen, "python");
}

#[test]
fn ruby_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&RubyCodegen, "ruby");
}

#[test]
fn php_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&PhpCodegen, "php");
}

#[test]
fn csharp_usage_assertion_reads_the_last_chunk() {
    assert_reads_last_chunk(&CSharpCodegen, "csharp");
}

/// wasm-bindgen maps the `u64` counter to a `BigInt`, which never equals a numeric literal, so
/// the generated assertion must widen it with `Number(...)` before comparing.
#[test]
fn wasm_usage_counter_is_widened_before_the_comparison() {
    let content = generated(&WasmCodegen, "usage.total_tokens");
    let lines = usage_lines(&content);
    assert!(
        lines.iter().any(|l| l.contains("Number(")),
        "wasm: the counter is compared as a BigInt. Emitted:\n{content}"
    );
    // Positive twin of the node arm: node has no BigInt, so it must stay unwrapped.
    let node = generated(&TypeScriptCodegen, "usage.total_tokens");
    assert!(
        usage_lines(&node).iter().all(|l| !l.contains("Number(")),
        "node: the counter must not be wrapped. Emitted:\n{node}"
    );
}

/// A call that does not stream returns a response whose `usage` is an ordinary field. The
/// stream-scoped root must not drag that read into the collected chunks.
#[test]
fn a_non_streaming_call_still_reads_usage_off_the_response() {
    let (e2e, resolved) = build_config_with("streaming = false\n");
    let content = generated_with(&RustE2eCodegen, &e2e, &resolved, "usage.total_tokens");
    let lines = usage_lines(&content);
    assert!(
        !lines.is_empty(),
        "no usage assertion was emitted at all. Emitted:\n{content}"
    );
    for line in &lines {
        assert!(
            line.contains("result") && !line.contains("chunks"),
            "a non-streaming usage assertion must read the response, not chunks: `{line}`"
        );
    }
}
