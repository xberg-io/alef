//! Regression test for PHP e2e codegen streaming-vs-non-streaming field disambiguation.
//!
//! When a fixture has a non-streaming call, the field "chunks" should NOT be treated
//! as a streaming virtual field. Instead, it should be handled as a regular result field
//! accessible via $result->chunks.
//!
//! The bug: PHP codegen was checking `is_streaming_virtual_field("chunks")` without
//! verifying that the call was actually streaming, causing "Undefined variable $chunks"
//! errors at test runtime when fixtures like config_chunking_prepend_heading_context
//! tried to reference an undeclared $chunks variable.
//!
//! The fix (in src/e2e/codegen/php/assertions.rs): add `is_streaming &&` before the
//! streaming field check in the assertion renderer, so "chunks" only resolves against the
//! collected-chunk list when the call actually streams.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// Two calls over the same crate: one streaming, one not. Both carry a `chunks` field, so the
/// only thing that can distinguish them in the emitted assertions is the `streaming` flag.
const TOML_TWO_CALLS: &str = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chunk_text"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "text", field = "input.text", type = "string" },
]

[crates.e2e.calls.chunk_text]
function = "chunk_text"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "text", field = "input.text", type = "string" },
]

[crates.e2e.calls.stream_text]
function = "stream_text"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
streaming = true
args = [
  { name = "text", field = "input.text", type = "string" },
]
"#;

fn render(call: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML_TWO_CALLS).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![chunks_count_min_fixture(call)],
    }];
    let files = PhpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("PHP codegen succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Test.php"))
        .expect("a *Test.php file is emitted")
        .content
        .clone()
}

fn chunks_count_min_fixture(call: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: format!("{call}_case"),
        category: Some("smoke".to_string()),
        description: "chunks count_min".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some(call.to_string()),
        input: serde_json::json!({ "text": "hello world" }),
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "count_min".to_string(),
            field: Some("chunks".to_string()),
            value: Some(serde_json::json!(2)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke/chunks.json".to_string(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

/// The non-streaming call must read `chunks` off the result object. `$chunks` is only ever
/// declared by the streaming preamble, so emitting it here is an "Undefined variable $chunks"
/// fatal at PHPUnit runtime — which no Rust-side check other than this one would notice.
#[test]
fn php_chunks_count_min_non_streaming_uses_result_field() {
    let content = render("chunk_text");

    assert!(
        content.contains("$result->chunks"),
        "a non-streaming `chunks` assertion must read the result field; got:\n{content}"
    );
    assert!(
        !content.contains("count($chunks)"),
        "a non-streaming call must not resolve `chunks` against the streaming `$chunks` list, \
         which it never declares; got:\n{content}"
    );
}

/// The other half of the guard: the streaming call must still take the streaming path, so the
/// fix cannot be "satisfied" by disabling streaming field resolution outright.
#[test]
fn php_chunks_count_min_streaming_uses_collected_chunk_list() {
    let content = render("stream_text");

    assert!(
        content.contains("count($chunks)"),
        "a streaming `chunks` assertion must resolve against the collected `$chunks` list; got:\n{content}"
    );
}
