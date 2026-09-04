//! Coverage for the ancestor-prefix guard in `test_function.rs`'s assertion loop: when an
//! assertion's field crosses an optional ANCESTOR (not the field itself), the emitted
//! `if <ancestor> != nil { ... }` wrap must fail the test on a nil ancestor unless a sibling
//! `not_empty` assertion on that exact ancestor path already covers presence.
//!
//! Field paths here mirror the real `results[0].metadata....` shape (an outer, always-present
//! array index over an inner optional struct) rather than a bare optional field, because the
//! wrap this file covers only fires when the ancestor accessor itself contains a bracket --
//! see `is_struct_value` in `test_function.rs`. A bare optional field with no array ancestor
//! (e.g. plain `metadata`) takes a different, unguarded rendering path entirely.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_function::{GoTestFunctionContext, render_test_function};

fn base_fixture(id: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn render(fixture: &Fixture, optional_fields: &[&str]) -> String {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "extract".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_optional: optional_fields.iter().map(|f| f.to_string()).collect(),
        fields_array: std::iter::once("results".to_string()).collect(),
        ..E2eConfig::default()
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
    out
}

/// `results[0].metadata` is optional and no sibling `not_empty` asserts its presence: the
/// `equals` assertion on `results[0].metadata.title`, wrapped in
/// `if result.Results[0].Metadata != nil { ... }`, must gain a failing `else` -- without it a
/// nil `Metadata` silently passes the test on exactly the regression the assertion exists to
/// catch.
#[test]
fn ancestor_guard_without_sibling_presence_fails_on_nil() {
    let fixture = base_fixture(
        "edge_metadata_title",
        vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.title".to_string()),
                value: Some(serde_json::Value::String("Simple Table Test".to_string())),
                ..Default::default()
            },
        ],
    );

    let out = render(&fixture, &["results[0].metadata"]);

    assert!(
        out.contains("if result.Results[0].Metadata != nil {"),
        "expected an ancestor guard on Results[0].Metadata; got:\n{out}"
    );
    assert!(
        out.contains(
            "} else {\n\t\tt.Errorf(\"expected %s to be present, got nil\", `result.Results[0].Metadata`)\n\t}"
        ),
        "expected a failing else branch naming the nil ancestor; got:\n{out}"
    );
}

/// Identical shape to the test above, except a sibling `not_empty` on the exact ancestor
/// path (`results[0].metadata`) already fails the test when `Metadata` is nil. The `equals`
/// assertion's wrap must stay guard-only -- an `else` here would only duplicate that failure.
#[test]
fn ancestor_guard_with_sibling_presence_check_stays_guard_only() {
    let fixture = base_fixture(
        "edge_metadata_title_with_not_empty",
        vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            },
            Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("results[0].metadata".to_string()),
                ..Default::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("results[0].metadata.title".to_string()),
                value: Some(serde_json::Value::String("Simple Table Test".to_string())),
                ..Default::default()
            },
        ],
    );

    let out = render(&fixture, &["results[0].metadata"]);

    // The `not_empty` assertion on `results[0].metadata` itself already fails on nil; the
    // `equals` assertion's ancestor wrap must not add a second, duplicate failure.
    assert!(
        !out.contains("expected %s to be present, got nil"),
        "sibling not_empty on the exact ancestor should suppress the failing else; got:\n{out}"
    );
    // The full guard-only block, byte for byte -- including the unrelated `len(result.Results)
    // == 0` non-empty precondition every `results[0]...`-anchored assertion carries (target.rs's
    // `array_guard`, triggered by the literal `[0]` in the field expression). That precondition
    // is orthogonal to this fix; pinning it here just keeps the expectation honest about what
    // the wrap actually contains, rather than a substring that happens to still match once it's
    // there. ~keep
    let expected_lines = [
        "\tif result.Results[0].Metadata != nil {",
        "\t\tif len(result.Results) == 0 {",
        "\t\t\tt.Fatalf(\"expected non-empty %s\", `result.Results`)",
        "\t\t}",
        "\t\tif string(result.Results[0].Metadata.Title) != `Simple Table Test` {",
        "\t\t\tt.Errorf(\"equals mismatch: got %v\", result.Results[0].Metadata.Title)",
        "\t\t}",
        "\t}",
    ];
    let expected = expected_lines.join("\n");
    assert!(
        out.contains(expected.as_str()),
        "expected the equals assertion to stay guard-only inside the ancestor wrap; got:\n{out}"
    );
}

/// `results[0].metadata` (the ancestor) AND `results[0].metadata.score` (the leaf) are
/// independently optional. The ancestor wrap's new failing `else` must nest cleanly around
/// the leaf's own guard-only nullable check (a genuinely optional value with no presence
/// claim, the same exemption `QualityScore` gets) without emitting two competing `else`
/// branches or breaking the block structure.
///
/// No `type_defs`/`enums` are wired into this harness, so `target_field_is_pointer` (the
/// IR-anchored answer `comparisons.rs` needs to add a `*` deref) resolves `None` -> `false`
/// here regardless of `fields_optional` -- unlike a real fixture (e.g. `SheetCount`), the
/// leaf's own guard checks `!= nil` without dereferencing. That does not affect what this
/// test is pinning: the ancestor wrap's nesting around whatever guard-only check the leaf
/// produces. ~keep
#[test]
fn nested_ancestor_and_field_guard_do_not_collide() {
    let fixture = base_fixture(
        "edge_metadata_score",
        vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            },
            Assertion {
                assertion_type: "greater_than_or_equal".to_string(),
                field: Some("results[0].metadata.score".to_string()),
                value: Some(serde_json::Value::from(0.5)),
                ..Default::default()
            },
        ],
    );

    let out = render(&fixture, &["results[0].metadata", "results[0].metadata.score"]);

    // As in the sibling-presence test above, the leaf's own comparison also carries a literal
    // `[0]`, so it too gets its own `len(result.Results) == 0` non-empty precondition (from
    // `target.rs`'s `array_guard`) ahead of the guard-only nullable check -- that block is
    // unrelated to this fix and is pinned here for the same honesty reason. ~keep
    let expected_lines = [
        "\tif result.Results[0].Metadata != nil {",
        "\t\tif len(result.Results) == 0 {",
        "\t\t\tt.Fatalf(\"expected non-empty %s\", `result.Results`)",
        "\t\t}",
        "\t\tif result.Results[0].Metadata.Score != nil {",
        "\t\t\tif result.Results[0].Metadata.Score < 0.5 {",
        "\t\t\t\tt.Errorf(\"expected >= 0.5, got %v\", result.Results[0].Metadata.Score)",
        "\t\t\t}",
        "\t\t}",
        "\t} else {",
        "\t\tt.Errorf(\"expected %s to be present, got nil\", `result.Results[0].Metadata`)",
        "\t}",
    ];
    let expected = expected_lines.join("\n");
    assert!(
        out.contains(expected.as_str()),
        "expected the outer ancestor guard's failing else to nest around the leaf's own \
         guard-only nullable check without a second else; got:\n{out}"
    );
}
