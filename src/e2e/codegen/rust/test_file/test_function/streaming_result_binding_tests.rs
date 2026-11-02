use super::*;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::e2e::fixture::Fixture;

#[path = "streaming_result_binding_tests/support.rs"]
mod support;

fn configuration() -> (E2eConfig, ResolvedCrateConfig) {
    let parsed: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "read_events"
module = "sample_core"
async = true
returns_result = true
[crates.e2e.call.streaming]
enabled = true
item_type = "String"
"#,
    )
    .expect("streaming configuration");
    let e2e = parsed.crates[0].e2e.clone().expect("e2e configuration");
    (e2e, parsed.resolve().expect("resolved configuration").remove(0))
}

fn fixture(assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "stream_multiple_choices",
        "description": "drain the complete stream",
        "input": null,
        "assertions": assertions,
        "docs": {"topic": "streaming", "stem": "multiple-choices"}
    }))
    .expect("streaming fixture")
}

fn render(fixture: &Fixture) -> String {
    let (e2e, config) = configuration();
    let mut output = String::new();
    render_test_function(
        &mut output,
        fixture,
        &e2e,
        &config,
        &[],
        &[],
        &[],
        "sample_core",
        None,
        None,
        false,
    );
    output
}

#[test]
fn not_error_only_stream_compiles_with_denied_warnings_and_drains_all_items() {
    let emitted = render(&fixture(serde_json::json!([{"type": "not_error"}])));
    support::compile_and_run("not_error_only", &emitted, support::FailurePoint::None, None);
}

#[test]
fn streams_without_assertions_still_drain_and_compile() {
    let emitted = render(&fixture(serde_json::json!([])));
    support::compile_and_run("no_assertions", &emitted, support::FailurePoint::None, None);
}

#[test]
fn asserted_streams_keep_the_collection_and_check_its_actual_length() {
    let emitted = render(&fixture(
        serde_json::json!([{"type":"count_equals", "field":"chunks", "value":2}]),
    ));
    assert!(emitted.contains("let chunks: Vec<_> ="), "{emitted}");
    support::compile_and_run("asserted_stream", &emitted, support::FailurePoint::None, None);
}

#[test]
fn structural_assertions_do_not_create_an_unused_collection_binding() {
    let emitted = render(&fixture(
        serde_json::json!([{"type":"is_true", "field":"no_chunks_after_done"}]),
    ));
    assert!(emitted.contains("let _: Vec<_> ="), "{emitted}");
    support::compile_and_run("structural_assertion", &emitted, support::FailurePoint::None, None);
}

#[test]
fn not_error_streams_reject_item_errors_after_draining() {
    let emitted = render(&fixture(serde_json::json!([{"type":"not_error"}])));
    support::compile_and_run(
        "item_error",
        &emitted,
        support::FailurePoint::Item,
        Some("stream item failed: \"fixture item error\""),
    );
}

#[test]
fn not_error_streams_reject_call_errors() {
    let emitted = render(&fixture(serde_json::json!([{"type":"not_error"}])));
    support::compile_and_run(
        "call_error",
        &emitted,
        support::FailurePoint::Call,
        Some("call failed: \"fixture call error\""),
    );
}

#[test]
fn streaming_docs_compile_and_present_the_drained_collection() {
    use crate::e2e::codegen::{E2eCodegen, rust::RustE2eCodegen};
    let (e2e, config) = configuration();
    let mut example = fixture(serde_json::json!([{"type":"not_error"}]));
    example.docs =
        serde_json::from_value(serde_json::json!({"topic":"streaming", "stem":"events", "shows":["chunks"]}))
            .expect("docs presentation");
    let emitted = RustE2eCodegen
        .render_snippet_body_with_functions(&example, &e2e, &config, &[], &[], &[], &[])
        .expect("render actual streaming docs snippet");
    assert!(emitted.contains("let chunks: Vec<_> ="), "{emitted}");
    support::compile_snippet_and_run(&emitted);
}

#[test]
fn docs_rebinding_only_changes_the_generated_collection_declaration() {
    use crate::e2e::codegen::rust::bind_streaming_snippet_collection;
    assert_eq!(
        bind_streaming_snippet_collection("    let _: Vec<_> = collect(stream);", "chunks"),
        "    let chunks: Vec<_> = collect(stream);"
    );
    for untouched in [
        r#"    let stream = read_events("let _: Vec<_> =");"#,
        "    let chunks_suffix: Vec<_> = collect(stream);",
        "    let _chunks: Vec<_> = collect(stream);",
    ] {
        assert_eq!(bind_streaming_snippet_collection(untouched, "chunks"), untouched);
    }
    for prefix in [
        "let message = r#\"préfix\nlet _: Vec<_> = sentinel\nsuffix\"#;\n",
        "let message = r##\"prefix\nlet _: Vec<_> = sentinel\nsuffix\"# tail\"##;\n",
        "let message = \"prefix\nlet _: Vec<_> = sentinel\nsuffix\";\n",
        "/* prefix\nlet _: Vec<_> = sentinel\n/* nested */ suffix */\n",
    ] {
        let body = format!("{prefix}    let _: Vec<_> = collect(stream);");
        assert_eq!(
            bind_streaming_snippet_collection(&body, "chunks"),
            format!("{prefix}    let chunks: Vec<_> = collect(stream);")
        );
    }
}

#[test]
fn streaming_docs_preserve_multiline_literal_input() {
    use crate::e2e::codegen::{E2eCodegen, rust::RustE2eCodegen};
    let (e2e, config) = configuration();
    let input = "prefix\nlet _: Vec<_> = sentinel\nsuffix";
    let mut example = fixture(serde_json::json!([{"type":"not_error"}]));
    example.input = serde_json::json!({"message": input});
    example.args = serde_json::from_value(serde_json::json!([
        {"name":"message", "field":"message", "type":"string"}
    ]))
    .expect("string argument mapping");
    let emitted = RustE2eCodegen
        .render_snippet_body_with_functions(&example, &e2e, &config, &[], &[], &[], &[])
        .expect("render multiline input streaming docs");
    support::compile_snippet_with_input(&emitted, input);
}
