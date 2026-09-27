//! Regression test for alef#453: a free function taking a bytes argument and a
//! single trailing optional argument (e.g. `fn sample(data: &[u8], password:
//! Option<&str>)`) generated Elixir e2e calls that the generated binding rejected
//! with `ArgumentError`, before either function argument reached the Rust core.
//!
//! Two independent defects, isolated into separate tests so each can be reverted
//! without masking the other:
//!
//! - Defect 1 (`bytes_arg_to_free_function_is_a_raw_binary`): a direct
//!   `arg_type = "bytes"` argument maps to a `rustler::Binary` NIF parameter and
//!   must be passed as a raw binary, not the byte-integer-list shape that the
//!   nested-struct-field path (alef#308) needs.
//! - Defect 2 (`single_trailing_optional_scalar_arg_emits_positional`): with
//!   fewer than two trailing optional arguments, the generated Rustler facade
//!   declares fixed positional arity clauses (see `use_keyword_opts` in
//!   `src/backends/rustler/gen_bindings/public_api.rs`), not a keyword-opts
//!   parameter. A keyword-form call (`password: "..."`) against that positional
//!   facade hands a keyword list to the NIF, which fails to decode as a string.
//!
//! `combined_bytes_and_single_optional_arg_call_matches_generated_facade` mirrors
//! the exact shape reported in the issue: both defects present in one call.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

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
function = "sample_process"
module = "Sample"
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

fn generate_test_body(args_toml: &str, input: serde_json::Value) -> String {
    let (e2e, resolved) = build_config_with_args(args_toml);
    let groups = vec![fixture_with_input(input)];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_test.exs"))
        .expect("Elixir test file is emitted");

    test_file.content.clone()
}

/// Defect 1: a bytes argument passed directly to a free function (no optional
/// argument in this fixture, isolating the defect from Defect 2's fix) must be a
/// raw binary, matching the `rustler::Binary` NIF parameter it decodes into.
#[test]
fn bytes_arg_to_free_function_is_a_raw_binary() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" }
"#;
    let input = serde_json::json!({
        "data": "docs/sample.bin"
    });
    let body = generate_test_body(args_toml, input);

    assert!(
        body.contains(r#"data = File.read!("../../test_documents/docs/sample.bin")"#),
        "expected a raw-binary File.read!, got:\n{body}"
    );
    assert!(
        !body.contains(":binary.bin_to_list"),
        "a direct bytes arg must not be converted to an integer list - Rustler's Binary decoder \
         rejects it, raising ArgumentError:\n{body}"
    );
}

/// Defect 2: a single trailing optional scalar argument (no bytes argument in
/// this fixture, isolating the defect from Defect 1's fix) must be emitted
/// positionally, matching the fixed-arity facade the Rustler backend generates
/// below its 2-trailing-optional keyword-opts threshold.
#[test]
fn single_trailing_optional_scalar_arg_emits_positional() {
    let args_toml = r#"
  { name = "path", field = "input.path", type = "file_path" },
  { name = "password", field = "input.password", type = "string", optional = true }
"#;
    let input = serde_json::json!({
        "path": "test.txt",
        "password": "secret"
    });
    let body = generate_test_body(args_toml, input);

    let call_line = body
        .lines()
        .find(|line| line.contains("Sample.sample_process"))
        .expect("sample_process call found");

    assert!(
        call_line.contains(r#"Sample.sample_process("../../test_documents/test.txt", "secret")"#),
        "expected a positional call matching the fixed-arity facade, got:\n{call_line}"
    );
    assert!(
        !call_line.contains("password:"),
        "a single trailing optional arg must not be emitted as a keyword - the generated facade \
         declares it positionally below the 2-trailing-optional threshold:\n{call_line}"
    );
}

/// The exact shape reported in alef#453: a free function taking a bytes argument
/// followed by a single optional argument. Both defects must be fixed together
/// for this call to match the generated binding.
#[test]
fn combined_bytes_and_single_optional_arg_call_matches_generated_facade() {
    let args_toml = r#"
  { name = "data", field = "input.data", type = "bytes" },
  { name = "password", field = "input.password", type = "string", optional = true }
"#;
    let input = serde_json::json!({
        "data": "docs/sample.bin",
        "password": "secret"
    });
    let body = generate_test_body(args_toml, input);

    let call_line = body
        .lines()
        .find(|line| line.contains("Sample.sample_process"))
        .expect("sample_process call found");

    assert!(
        body.contains(r#"data = File.read!("../../test_documents/docs/sample.bin")"#),
        "expected a raw-binary File.read! for the bytes arg, got:\n{body}"
    );
    assert!(
        call_line.contains("Sample.sample_process(data, \"secret\")"),
        "expected a positional call passing the raw binary and the optional string \
         positionally, got:\n{call_line}"
    );
}
