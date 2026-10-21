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

#[test]
fn stream_error_reads_the_stream_before_the_expected_throw_check() {
    let output = render(true);
    let read = output.find("for try await _ in _stream {}").expect("stream is read");
    let fail = output.find("XCTFail(\"expected to throw\")").expect("error check");
    let call = output.find("let _stream = try await").expect("stream is bound");
    assert!(call < read && read < fail, "{output}");
}

#[test]
fn non_stream_error_keeps_the_direct_call() {
    let output = render(false);
    assert!(output.contains("XCTFail(\"expected to throw\")"), "{output}");
    assert!(!output.contains("for try await"), "{output}");
}
