//! Verifies the Ruby e2e codegen emits a real RSpec `it` block for fixtures
//! whose only assertion is `not_error`.
//!
//! Pre-fix the codegen hit the `!expects_error && !has_usable && !is_streaming`
//! guard and fell through to `skip 'Non-HTTP fixture cannot be tested via Net::HTTP'`,
//! producing 8 pending tests in the Ruby suite.  A `not_error`-only fixture IS
//! testable — the call must not raise — so the guard must exclude such fixtures.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::ruby::RubyCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "url", field = "input.url", type = "string" },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn not_error_only_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_not_error_only".to_string(),
            category: Some("smoke".to_string()),
            description: "call succeeds without raising".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "url": "https://example.com" }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "smoke/smoke_not_error_only.json".to_string(),
            http: None,
        }],
    }
}

/// A fixture with only a `not_error` assertion must not emit a skip/pending block.
#[test]
fn not_error_only_fixture_does_not_emit_pending() {
    let (e2e, resolved) = build_config();
    let groups = vec![not_error_only_fixture_group()];
    let files = RubyCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");

    let spec_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("smoke_spec.rb"))
        .expect("Ruby smoke_spec.rb is emitted");

    let body = &spec_file.content;

    assert!(
        !body.contains("skip 'Non-HTTP fixture cannot be tested via Net::HTTP'"),
        "fixture with only not_error must not emit skip/pending, got:\n{body}"
    );
    assert!(
        body.contains("smoke_not_error_only"),
        "fixture id must appear in generated spec, got:\n{body}"
    );
    // The test body should contain a real assertion — either not_to be_nil (the
    // template fallback when has_usable is false) or some explicit expectation.
    assert!(
        body.contains("not_to be_nil") || body.contains("not_to raise_error"),
        "fixture with only not_error must emit a real assertion, got:\n{body}"
    );
}
