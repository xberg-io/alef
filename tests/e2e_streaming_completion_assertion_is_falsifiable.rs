//! The csharp and ruby streaming renderers used to emit an unconditional assignment immediately
//! before the assertion that read it (`streamComplete = true;` / `stream_complete = true`), so
//! `Assert.True(streamComplete)` and `expect(stream_complete).to be(true)` could not fail for any
//! stream whatsoever. Worse than a dropped assertion: the test is green, carries no skip marker,
//! and proves nothing. Both backends additionally aliased `no_chunks_after_done` onto the same
//! flag, collapsing two distinct assertions onto one unfalsifiable check.
//!
//! These tests are written against the *emitted method*, not the renderer's internals, and each
//! one asserts FIRST that a streaming assertion was emitted at all — a test that only looked for
//! the absence of `= true` would pass vacuously on a backend that emits nothing, which is exactly
//! the failure mode next door to the one under test.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const FIXTURE_ID: &str = "stream_terminates_cleanly";

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["csharp", "ruby"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "crawl_stream"
module = "MyLib"
result_var = "result"
args = [
  { name = "url", field = "input.url", type = "string" },
]

[crates.e2e.call.streaming]
enabled = true
item_type = "StreamChunk"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn assertion(assertion_type: &str, field: &str) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// `stream_content` is one of the chat pseudo-fields both renderers use to decide the streamed
/// chunk is chat-shaped, i.e. that it carries the terminal `finish_reason` `stream_complete` is
/// defined by. Without it the fixture is a generic event stream and completion is refused.
fn chat_stream_group() -> Vec<FixtureGroup> {
    group_with(vec![
        assertion("not_empty", "stream_content"),
        assertion("is_true", "stream_complete"),
        assertion("is_true", "no_chunks_after_done"),
    ])
}

/// The negative control: same fixture, no chat-shaped field, so neither backend can read a
/// terminal marker off the chunks.
fn generic_stream_group() -> Vec<FixtureGroup> {
    group_with(vec![assertion("is_true", "stream_complete")])
}

fn group_with(assertions: Vec<Assertion>) -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "stream".to_string(),
        fixtures: vec![Fixture {
            id: FIXTURE_ID.to_string(),
            category: Some("stream".to_string()),
            description: "streaming call terminates cleanly".to_string(),
            input: serde_json::json!({ "url": "https://example.com" }),
            assertions,
            source: "stream/stream_terminates_cleanly.json".to_string(),
            ..Fixture::default()
        }],
    }]
}

/// Slice out the single emitted test body so "assigned a literal `true`" is scoped to the same
/// method as the assertion, not to some unrelated fixture elsewhere in the generated project.
fn method_body<C: E2eCodegen>(codegen: &C, groups: &[FixtureGroup], start_marker: &str, end_marker: &str) -> String {
    let (e2e, resolved) = build_config();
    let files = codegen
        .generate(groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let paths: Vec<String> = files.iter().map(|f| f.path.display().to_string()).collect();
    let (content, start) = files
        .iter()
        .find_map(|f| f.content.find(start_marker).map(|i| (f.content.clone(), i)))
        .unwrap_or_else(|| panic!("no generated file carries `{start_marker}`. Files: {paths:?}"));
    let rest = &content[start..];
    let end = rest.find(end_marker).map_or(rest.len(), |i| i + end_marker.len());
    rest[..end].to_string()
}

/// The core check. `operand` is the identifier the emitted assertion reads; if any line in the
/// same method assigns it a literal `true`, the assertion is unfalsifiable no matter how the
/// stream behaved.
fn assert_operand_is_never_assigned_literal_true(body: &str, operand: &str) {
    for line in body.lines() {
        let trimmed = line.trim();
        let assigned_true = trimmed == format!("{operand} = true;")
            || trimmed == format!("{operand} = true")
            || trimmed == format!("var {operand} = true;")
            || trimmed == format!("{operand} = 1;");
        assert!(
            !assigned_true,
            "`{operand}` is assigned a literal `true` in the same method that asserts it, so the \
             assertion cannot fail. Emitted:\n{body}"
        );
    }
}

/// Test methods land at 8 spaces of indent inside `namespace { class { ... } }`, so the method's
/// own closing brace is the first `\n        }\n` after its signature.
fn csharp_method(groups: &[FixtureGroup]) -> String {
    method_body(
        &CSharpCodegen,
        groups,
        "public async Task Test_StreamTerminatesCleanly()",
        "\n        }\n",
    )
}

fn ruby_example(groups: &[FixtureGroup]) -> String {
    method_body(&RubyCodegen, groups, &format!("it '{FIXTURE_ID}"), "\n  end\n")
}

#[test]
fn csharp_stream_complete_assertion_reads_a_value_derived_from_the_chunks() {
    let body = csharp_method(&chat_stream_group());

    assert!(
        body.contains("Assert.True(streamComplete);"),
        "the fixture must emit a stream_complete assertion at all before its operand is judged. \
         Emitted:\n{body}"
    );
    assert_operand_is_never_assigned_literal_true(&body, "streamComplete");
    assert!(
        body.contains("var streamComplete = chunks.Count > 0"),
        "stream_complete must be derived from the collected chunks. Emitted:\n{body}"
    );
    assert!(
        body.contains("FinishReason.HasValue"),
        "stream_complete must read the terminal finish_reason, matching every other backend. \
         Emitted:\n{body}"
    );
}

#[test]
fn csharp_no_chunks_after_done_is_a_separate_probe_not_an_alias() {
    let body = csharp_method(&chat_stream_group());

    assert!(
        body.contains("Assert.True(noChunksAfterDone);"),
        "the fixture must emit a no_chunks_after_done assertion at all. Emitted:\n{body}"
    );
    assert!(
        !body.contains("Assert.True(true)"),
        "no_chunks_after_done must not render as a literal tautology. Emitted:\n{body}"
    );
    assert_operand_is_never_assigned_literal_true(&body, "noChunksAfterDone");
    assert!(
        body.contains("var noChunksAfterDone = !(await streamEnumerator.MoveNextAsync());"),
        "no_chunks_after_done must be the result of asking the enumerator for one more element \
         after completion. Emitted:\n{body}"
    );
    // The aliasing this test exists to prevent: two distinct fields resolving to one variable.
    assert_eq!(
        body.matches("Assert.True(streamComplete);").count(),
        1,
        "stream_complete and no_chunks_after_done must not both render as `streamComplete`. \
         Emitted:\n{body}"
    );
}

#[test]
fn csharp_refuses_stream_complete_visibly_when_the_chunks_carry_no_terminal_marker() {
    let body = csharp_method(&generic_stream_group());

    assert!(
        !body.contains("Assert.True(streamComplete);"),
        "a generic event stream has no terminal finish_reason, so a passing completion check must \
         not be emitted. Emitted:\n{body}"
    );
    assert!(
        body.contains("streaming assertion on unsupported field 'stream_complete'"),
        "refusing must be visible to the skip ledger, not silent. Emitted:\n{body}"
    );
    assert!(
        body.contains("no terminal finish_reason"),
        "the skip must say why the assertion could not be checked. Emitted:\n{body}"
    );
}

#[test]
fn ruby_stream_complete_assertion_reads_a_value_derived_from_the_chunks() {
    let body = ruby_example(&chat_stream_group());

    assert!(
        body.contains("expect(stream_complete).to be(true)"),
        "the fixture must emit a stream_complete assertion at all before its operand is judged. \
         Emitted:\n{body}"
    );
    assert_operand_is_never_assigned_literal_true(&body, "stream_complete");
    assert!(
        body.contains("stream_complete = !chunks.empty? && !chunks.last&.choices&.first&.finish_reason.nil?"),
        "stream_complete must bind the resolver's own ruby accessor. Emitted:\n{body}"
    );
}

#[test]
fn ruby_no_chunks_after_done_reads_the_chunks_after_the_done_chunk() {
    let body = ruby_example(&chat_stream_group());

    assert!(
        body.contains("done_at = chunks.index { |c| c.choices.any? { |ch| !ch.finish_reason.nil? } }"),
        "no_chunks_after_done must locate the done chunk. Emitted:\n{body}"
    );
    assert!(
        body.contains("expect(chunks.drop(done_at + 1).all?"),
        "no_chunks_after_done must check the chunks after the done chunk. Emitted:\n{body}"
    );
    assert_eq!(
        body.matches("expect(stream_complete).to be(true)").count(),
        1,
        "no_chunks_after_done must not render as a second `stream_complete` check. Emitted:\n{body}"
    );
}

#[test]
fn ruby_refuses_stream_complete_visibly_when_the_chunks_carry_no_terminal_marker() {
    let body = ruby_example(&generic_stream_group());

    assert!(
        !body.contains("expect(stream_complete).to be(true)"),
        "a generic event stream has no terminal finish_reason, so a passing completion check must \
         not be emitted. Emitted:\n{body}"
    );
    assert!(
        body.contains("streaming assertion on unsupported field 'stream_complete'"),
        "refusing must be visible to the skip ledger, not silent. Emitted:\n{body}"
    );
    assert!(
        body.contains("no terminal finish_reason"),
        "the skip must say why the assertion could not be checked. Emitted:\n{body}"
    );
}
