//! Regression test for alef#308: the Elixir e2e generator must never hand a raw
//! binary read from a fixture file (or decoded from base64) directly to a NIF
//! argument, because that value later crosses a `Jason.encode!` hop (e.g. the
//! `ExtractInput` struct's `bytes` field) and Jason blows up on the first
//! non-UTF-8 byte with `Jason.EncodeError: invalid byte 0xC4`.
//!
//! The fix converts both the file-read and base64-decode paths to an
//! integer-list shape (`:binary.bin_to_list(...)`), matching the shape the
//! already-working inline `Vec<u8>` JSON-array path emits.

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

/// Covers the `test_documents`-prefixed branch in args.rs (~line 300), a
/// forward-looking guard should a fixture ever route a `bytes` arg through a file
/// path here - this branch produces zero call sites in xberg's generated e2e today;
/// `docs_file_read_template_emits_integer_list_not_bare_binary` above covers the
/// path that actually produces the reported `Jason.EncodeError`. ~keep
///
/// A `bytes` arg whose fixture value looks like a file path must be read as raw
/// bytes and immediately converted to a byte-integer list, not left as a bare
/// binary that later crashes `Jason.encode!` on non-UTF-8 content.
#[test]
fn bytes_arg_file_path_emits_integer_list_not_bare_binary() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" }
"#;
    let input = serde_json::json!({
        "data": "docs/sample.bin"
    });
    let body = generate_test_body(args_toml, input);

    assert!(
        body.contains(r#"data = :binary.bin_to_list(File.read!("../../test_documents/docs/sample.bin"))"#),
        "expected exact integer-list file read, got:\n{body}"
    );
    // No bare `File.read!(...)` assigned straight to a variable used as the NIF arg.
    assert!(
        !body.contains("data = File.read!("),
        "found a bare File.read! assignment that skips the byte-integer conversion:\n{body}"
    );
}

/// Covers the base64-decode branch in args.rs (~line 315) - speculative hardening,
/// not a fix for an observed failure: this branch also produces zero call sites in
/// xberg's generated e2e today. ~keep
///
/// A `bytes` arg whose fixture value is an inline base64 literal must also be
/// converted to a byte-integer list after decoding - the decoded binary is just
/// as capable of containing non-UTF-8 bytes as a file read is.
#[test]
fn bytes_arg_base64_emits_integer_list_not_bare_binary() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" }
"#;
    // "xyz+AMQ" is not shaped like a file path (no '/'), so it takes the base64 branch.
    let input = serde_json::json!({
        "data": "xyzAMQ"
    });
    let body = generate_test_body(args_toml, input);

    assert!(
        body.contains(r#"data = :binary.bin_to_list(Base.decode64!("xyzAMQ", padding: false))"#),
        "expected exact integer-list base64 decode, got:\n{body}"
    );
    assert!(
        !body.contains("data = Base.decode64!("),
        "found a bare Base.decode64! assignment that skips the byte-integer conversion:\n{body}"
    );
}
