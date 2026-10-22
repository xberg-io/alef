//! A Swift stream reports a failure while it is read: the call returns the stream and the
//! `for try await` loop throws. The generated error test must read the stream inside its `do`
//! block, or the test reaches `XCTFail("expected to throw")` for a stream that fails.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn render(streaming: bool) -> String {
    let source = format!(
        r#"
[workspace]
languages = ["swift"]
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
async = true
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
                ..Assertion::default()
            }],
            ..Fixture::default()
        }],
    }];
    let files = SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation");
    assert!(!files.is_empty(), "generation must examine a real fixture");
    files
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The statements of the `do` block of the generated test, and the text that follows it.
fn split_do_block(output: &str) -> (&str, &str) {
    let (before_catch, handler) = output
        .split_once("        } catch {")
        .expect("the error test has a catch block");
    let (_, do_block) = before_catch
        .rsplit_once("        do {\n")
        .expect("the catch block closes a do block");
    (do_block, handler)
}

#[test]
fn stream_error_reads_the_stream_inside_the_do_block_before_the_expected_throw_check() {
    let output = render(true);
    let (do_block, handler) = split_do_block(&output);
    let bind = do_block
        .find("let _stream = try await")
        .expect("the stream is bound inside the do block");
    let read = do_block
        .find("for try await _ in _stream {}")
        .expect("the stream is read inside the do block");
    let fail = do_block
        .find("XCTFail(\"expected to throw\")")
        .expect("the throw check is inside the do block");
    assert!(bind < read && read < fail, "{output}");
    assert!(
        !handler.contains("for try await") && !handler.contains("let _stream"),
        "the stream is not read after the catch block starts:\n{output}"
    );
}

#[test]
fn non_stream_error_keeps_the_direct_call() {
    let output = render(false);
    assert!(output.contains("XCTFail(\"expected to throw\")"), "{output}");
    assert!(!output.contains("for try await"), "{output}");
}
