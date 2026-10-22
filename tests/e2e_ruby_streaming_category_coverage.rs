//! Verifies a ruby fixture category is emitted whenever its fixtures render real examples,
//! and that the category gate agrees with the renderer about what "real" means.
//!
//! The defect: `ruby.rs` decided whether to emit a category's spec file at all using a
//! predicate that omitted `is_streaming`, while `spec_file.rs` decided what to put in the
//! file using one that included it. A category whose fixtures were all streaming therefore
//! produced no file — and nothing downstream notices an absent category, because `alef
//! verify` walks emitted markers, the empty-category check in `e2e/validate.rs` only fires
//! when *every* configured language skips a category, and `fixture_inclusion` never consults
//! an emitter's capability. Same silence class as the zig streaming drop.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

fn build_config(streaming: bool) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let streaming_block = if streaming {
        "\n[crates.e2e.call.streaming]\nenabled = true\n"
    } else {
        ""
    };
    let toml_src = format!(
        r#"
[workspace]
languages = ["ruby"]

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
  {{ name = "url", field = "input.url", type = "string" }},
]
{streaming_block}"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

/// A fixture carrying no assertions at all. `spec_file.rs` renders a real example for such a
/// fixture when the call is streaming, and only a `skip` stub when it is not — so this single
/// fixture shape isolates the `is_streaming` term that the category gate was missing.
fn assertionless_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "stream".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "stream_emits_events".to_string(),
            category: Some("stream".to_string()),
            description: "streaming call yields events".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "url": "https://example.com" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: Vec::new(),
            source: "stream/stream_emits_events.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

fn spec_file(files: &[alef::core::backend::GeneratedFile]) -> Option<String> {
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("stream_spec.rb"))
        .map(|f| f.content.clone())
}

/// The half that was unreachable: the renderer emits a real streaming example for this
/// fixture, but the category gate dropped the whole file before it was ever called.
#[test]
fn a_streaming_category_is_emitted_even_when_no_assertion_is_field_usable() {
    let (e2e, resolved) = build_config(true);
    let files = RubyCodegen
        .generate(&assertionless_fixture_group_vec(), &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    let body = spec_file(&files).expect("stream_spec.rb must be emitted for a streaming category");
    assert!(
        body.contains("stream_emits_events"),
        "the streaming fixture must appear in the emitted spec. Rendered:\n{body}"
    );
    assert!(
        !body.contains("Fixture has no assertions to validate"),
        "a streaming fixture must render a real example, not the no-assertions skip stub. Rendered:\n{body}"
    );
}

/// The control: the identical fixture on a non-streaming call is genuinely untestable, and
/// must still be dropped. This proves the fix narrows the gate to match the renderer rather
/// than simply emitting every category unconditionally.
#[test]
fn a_non_streaming_category_with_nothing_renderable_is_still_dropped() {
    let (e2e, resolved) = build_config(false);
    let files = RubyCodegen
        .generate(&assertionless_fixture_group_vec(), &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    assert!(
        spec_file(&files).is_none(),
        "a category whose only fixture renders nothing executable must not emit a spec file"
    );
}

fn assertionless_fixture_group_vec() -> Vec<FixtureGroup> {
    vec![assertionless_fixture_group()]
}
