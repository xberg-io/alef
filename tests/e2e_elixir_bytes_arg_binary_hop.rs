//! Regression coverage for two related but distinct Elixir e2e bytes-marshalling
//! defects that share this file because both were exercised by the same
//! `try_push_bytes_value` / `render_struct_fields` code paths.
//!
//! alef#308: a `bytes` value nested in a struct field (e.g. an options struct's
//! `content` field, reached via `render_struct_fields` / `docs_file_read.jinja`)
//! later crosses a `Jason.encode!` hop, and Jason blows up on the first
//! non-UTF-8 byte with `Jason.EncodeError: invalid byte 0xC4`. The fix there
//! converts the struct-field path to an integer-list shape
//! (`:binary.bin_to_list(...)`), matching the shape the already-working inline
//! `Vec<u8>` JSON-array path emits. `docs_file_read_template_emits_integer_list_not_bare_binary`
//! below pins that fix and must keep passing.
//!
//! alef#453: an `arg_type = "bytes"` argument passed *directly* to a free
//! function (not nested in a struct) maps straight to a `rustler::Binary` NIF
//! parameter and never crosses `Jason.encode!` on any facade shape (see
//! `json_encode_param_indices` in `src/backends/rustler/gen_bindings/public_api_args.rs`,
//! which never marks a bytes param). Rustler's `Binary` decoder does not accept
//! an Elixir list, so the integer-list form that alef#308 needs for the
//! struct-field path raises `ArgumentError` here instead. The fix makes
//! `try_push_bytes_value` emit a raw binary (`File.read!`/`Base.decode64!`)
//! for this direct-argument path. `bytes_arg_file_path_emits_raw_binary_not_integer_list`
//! and `bytes_arg_base64_emits_raw_binary_not_integer_list` below pin that fix.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{
    Assertion, Fixture, FixtureDocs, FixtureDocsFileInput, FixtureDocsPresentation, FixtureGroup,
};

fn build_config_with_args(args_toml: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
module = "MyLib"
result_var = "result"
returns_result = true
args = [
  {args_toml}
]
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture_with_input(input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "test_fixture".to_string(),
            category: Some("test".to_string()),
            description: "test fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input,
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_empty".to_string(),
                field: Some("output".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/test_fixture.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }
}

/// Builds a fixture whose `input.config` object has a docs-presentation file attached
/// to one of its keys, driving the `render_struct_fields` -> `docs_file_read.jinja`
/// path (args.rs `render_struct_fields`, the template render call) rather than the
/// `arg_type = "bytes"` file-path/base64 branches exercised above.
fn fixture_with_docs_file(input: serde_json::Value, doc_field: &str, doc_path: &str) -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            docs: Some(FixtureDocs {
                sample_url: None,
                topic: "test".to_string(),
                stem: None,
                paths: Default::default(),
                title: None,
                description: None,
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: Some(FixtureDocsPresentation {
                    call: None,
                    input: None,
                    args: None,
                    files: vec![FixtureDocsFileInput {
                        field: doc_field.to_string(),
                        path: doc_path.to_string(),
                    }],
                    operations: Vec::new(),
                }),
                client: None,
                side_effects: Default::default(),
                coverage_exceptions: Default::default(),
                sample_url_vars: Default::default(),
                body_file: None,
            }),
            requirements: Vec::new(),
            id: "test_fixture".to_string(),
            category: Some("test".to_string()),
            description: "test fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input,
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_empty".to_string(),
                field: Some("output".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/test_fixture.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }
}

fn generate_test_body(args_toml: &str, input: serde_json::Value) -> String {
    generate_test_body_with_docs(args_toml, vec![fixture_with_input(input)])
}

fn generate_test_body_with_docs(args_toml: &str, groups: Vec<FixtureGroup>) -> String {
    let (e2e, resolved) = build_config_with_args(args_toml);
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_test.exs"))
        .expect("Elixir test file is emitted");

    test_file.content.clone()
}

/// Drives the `docs_file_read.jinja` template path directly (args.rs
/// `render_struct_fields`, the call site rendering that template): a `json_object`
/// arg whose value object has a field carrying a fixture docs-presentation file.
/// This is the path that actually produces `File.read!` calls in xberg's generated
/// Elixir e2e suite today (a bare relative path, not `test_documents`-prefixed) -
/// unlike the two tests above, whose branches currently emit no real call sites.
#[test]
fn docs_file_read_template_emits_integer_list_not_bare_binary() {
    let args_toml = r#"
  { name = "config", field = "input.config", type = "json_object", element_type = "MyConfigType" }
"#;
    let input = serde_json::json!({
        "config": { "content": "placeholder" }
    });
    let groups = vec![fixture_with_docs_file(input, "/config/content", "text/plain.txt")];
    let body = generate_test_body_with_docs(args_toml, groups);

    assert!(
        body.contains(r#"content: :binary.bin_to_list(File.read!("text/plain.txt"))"#),
        "expected exact integer-list docs-file read, got:\n{body}"
    );
    assert!(
        !body.contains(r#"content: File.read!("text/plain.txt")"#),
        "found a bare File.read! from the docs_file_read.jinja path, unwrapped before the Jason.encode! hop:\n{body}"
    );
}

/// Covers the `test_documents`-prefixed branch in args.rs (~line 500): a direct
/// `arg_type = "bytes"` argument, as used by a free function taking a bytes
/// parameter (e.g. `fn sample(data: &[u8])`), maps straight to a
/// `rustler::Binary` NIF parameter and must receive a raw binary. alef#453:
/// this branch previously emitted the byte-integer-list shape meant for the
/// struct-field/`Jason.encode!` path (alef#308), and Rustler's `Binary`
/// decoder rejects a list, raising `ArgumentError`.
#[test]
fn bytes_arg_file_path_emits_raw_binary_not_integer_list() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" }
"#;
    let input = serde_json::json!({
        "data": "docs/sample.bin"
    });
    let body = generate_test_body(args_toml, input);

    assert!(
        body.contains(r#"data = File.read!("../../test_documents/docs/sample.bin")"#),
        "expected exact raw-binary file read, got:\n{body}"
    );
    assert!(
        !body.contains(":binary.bin_to_list"),
        "found a byte-integer-list conversion on a direct bytes arg - Rustler's Binary decoder \
         rejects a list and raises ArgumentError (alef#453):\n{body}"
    );
}

/// Covers the base64-decode branch in args.rs (~line 518). Same reasoning as
/// the file-path case above: a direct `bytes` argument is passed to the NIF
/// as a raw binary, never JSON-encoded, so decoding straight to a binary
/// (not an integer list) is required (alef#453).
#[test]
fn bytes_arg_base64_emits_raw_binary_not_integer_list() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" }
"#;
    // "xyz+AMQ" is not shaped like a file path (no '/'), so it takes the base64 branch.
    let input = serde_json::json!({
        "data": "xyzAMQ"
    });
    let body = generate_test_body(args_toml, input);

    assert!(
        body.contains(r#"data = Base.decode64!("xyzAMQ", padding: false)"#),
        "expected exact raw-binary base64 decode, got:\n{body}"
    );
    assert!(
        !body.contains(":binary.bin_to_list"),
        "found a byte-integer-list conversion on a direct bytes arg - Rustler's Binary decoder \
         rejects a list and raises ArgumentError (alef#453):\n{body}"
    );
}
