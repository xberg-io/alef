//! Regression: Go must classify a namespace-prefixed fixture field by the path the accessor
//! actually addresses.
//!
//! A fixture field spelled `interaction.action_results` groups the assertion under a virtual
//! `interaction` label; the value sits at `action_results` on the result, and `FieldResolver::
//! accessor` already strips the label — the Go generator emits `result.ActionResults`.
//! `FieldResolver::is_array`, however, was a bare set lookup against the *unstripped* spelling,
//! so it answered "not an array" for the very slice the accessor had just produced. Go's
//! `contains` renderer turns that into `string(result.ActionResults)`, which does not compile:
//! a `[]T` cannot be converted to `string`. The correct emission is `jsonString(...)`, the
//! helper the generator already uses for every slice-valued `contains`.
//!
//! A genuinely nested field (`metrics.total_lines`, where `metrics` is a declared result field)
//! must keep its full path and stay unclassified, so the fix cannot be "always strip".

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::go::GoCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["go"]
result_fields = ["action_results", "final_url", "metrics"]
fields_array = ["action_results"]

[crates.e2e.call]
function = "interact"
module = "github.com/example/testlib"
result_var = "result"
returns_result = true
args = [
  { name = "url", field = "url", type = "mock_url" },
]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "interaction".to_string(),
        fixtures: vec![Fixture {
            id: "namespaced_array_field".to_string(),
            category: Some("interaction".to_string()),
            description: "Namespace-prefixed array field assertion".to_string(),
            input: serde_json::json!({ "url": "/page1" }),
            assertions: vec![
                Assertion {
                    assertion_type: "not_error".to_string(),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "contains".to_string(),
                    field: Some("interaction.action_results".to_string()),
                    value: Some(serde_json::json!("click")),
                    ..Assertion::default()
                },
            ],
            source: "test.json".to_string(),
            ..Fixture::default()
        }],
    }
}

fn generate_category_test() -> String {
    let cfg = build_config();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let files = GoCodegen
        .generate(&[build_fixture_group()], &e2e, &resolved, &[], &[], &[], &[])
        .expect("go generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("interaction_test.go"))
        .unwrap_or_else(|| {
            panic!(
                "interaction_test.go is emitted; got {:?}",
                files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
            )
        })
        .content
        .clone()
}

/// The accessor already strips the virtual prefix, so the slice conversion must be the
/// slice-aware one.
#[test]
fn namespace_prefixed_array_field_uses_the_slice_stringifier() {
    let generated = generate_category_test();
    // ~keep `jsonString` gained a leading `t *testing.T` parameter (commit 301c4e9b9, "align
    // emitted types and assertions") so a marshal failure can `t.Fatal` instead of being
    // swallowed; every call site, including this one, now passes `t` first.
    assert!(
        generated.contains("jsonString(t, result.ActionResults)"),
        "a slice-valued contains must serialize the slice; got:\n{generated}"
    );
}

/// The literal emission the bug produced. Asserting its absence separately keeps the failure
/// message pointed at the defect: `string([]T)` is not a legal Go conversion, so the generated
/// package would not build.
#[test]
fn namespace_prefixed_array_field_does_not_emit_a_scalar_string_conversion() {
    let generated = generate_category_test();
    assert!(
        !generated.contains("string(result.ActionResults)"),
        "`string(...)` on a slice does not compile; got:\n{generated}"
    );
}
