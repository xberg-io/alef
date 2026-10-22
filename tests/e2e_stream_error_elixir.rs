//! An Elixir stream reports a failure while it is read: the call returns `{:ok, stream}` and the
//! enumeration raises. The generated error test must read the stream, so a fixture with a
//! mid-stream error passes against a stream that raises and fails against one that ends clean.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, elixir::ElixirCodegen};
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use std::io::ErrorKind;

fn render(streaming: bool, declared: bool) -> String {
    let source = format!(
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
function = "stream_text"
module = "MyLib"
result_var = "events"
streaming = {streaming}
args = []
"#
    );
    let config: NewAlefConfig = toml::from_str(&source).expect("configuration parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e configuration");
    let resolved = config.resolve().expect("configuration resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "streaming".into(),
        fixtures: vec![Fixture {
            id: "mid_stream_error".into(),
            category: Some("streaming".into()),
            assertions: vec![Assertion {
                assertion_type: "error".into(),
                value: declared.then(|| serde_json::json!("Rate limit reached")),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }],
    }];
    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation");
    assert!(!files.is_empty(), "generation must examine a real fixture");
    files
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The statements of the generated test body, without the `test` and `describe` wrappers.
fn test_body(output: &str) -> String {
    let start = output.find("      __reason =").expect("stream error assertion");
    let body = &output[start..];
    let end = body.find("\n    end\n").expect("test ends");
    body[..end].to_string()
}

#[test]
fn stream_error_reads_the_stream_inside_the_error_assertion() {
    for declared in [false, true] {
        let body = test_body(&render(true, declared));
        assert!(body.contains("{:ok, __stream} ->"), "{body}");
        assert!(
            body.contains("Enum.to_list(__stream)"),
            "the stream must be read:\n{body}"
        );
        assert!(body.contains("rescue"), "{body}");
        assert_eq!(body.contains("Rate limit reached"), declared, "{body}");
    }
}

#[test]
fn non_stream_error_keeps_the_direct_call() {
    let output = render(false, false);
    assert!(output.contains("assert {:error, _} = MyLib.stream_text()"), "{output}");
    assert!(!output.contains("Enum.to_list(__stream)"), "{output}");
}

fn run(body: &str, stream: &str) -> Option<std::process::Output> {
    let script = format!(
        "import ExUnit.Assertions\n\ndefmodule MyLib do\n  def stream_text, do: {{:ok, {stream}}}\nend\n\n{body}\n"
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("stream_error.exs");
    std::fs::write(&path, script).expect("write script");
    match alef::core::tool_command("elixir").arg(&path).output() {
        Ok(output) => Some(output),
        Err(error) if error.kind() == ErrorKind::NotFound && std::env::var_os("ALEF_REQUIRE_ELIXIR").is_some() => {
            panic!("ALEF_REQUIRE_ELIXIR is set but Elixir is unavailable: {error}")
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => panic!("failed to execute Elixir: {error}"),
    }
}

#[test]
fn generated_error_test_passes_only_when_the_stream_raises() {
    let body = test_body(&render(true, true));
    let raising = "Stream.unfold(0, fn 0 -> {%{}, 1}; _ -> raise \"Rate limit reached\" end)";
    let clean = "Stream.unfold(0, fn 0 -> {%{}, 1}; _ -> nil end)";

    let Some(passes) = run(&body, raising) else { return };
    assert!(
        passes.status.success(),
        "a stream that raises satisfies the error assertion:\n{}",
        String::from_utf8_lossy(&passes.stderr)
    );
    let fails = run(&body, clean).expect("Elixir is available");
    assert!(
        !fails.status.success(),
        "a stream that ends clean must fail the error assertion"
    );
}
