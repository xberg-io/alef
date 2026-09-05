//! Regression coverage for `disambiguate_presentation_items` (defect: a fixture-authored
//! `docs.presentation` iterate `item` that reuses the call's own `result_var` renders a C#
//! `foreach (var result in ...)` directly under `var result = ...Call()` -- CS0136). Split out of
//! `snippet.rs`'s own `#[cfg(test)] mod tests`, which was already close to the 1,000-line cap
//! (`file-modularization`).

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

/// Regression for the CS0136 this closes: a fixture whose `docs.presentation` names its iterate
/// `item` the same as the call's own `result_var` (a natural singular/plural coincidence --
/// `results` -> `result`) must not render `foreach (var result in result.Results)` directly
/// under `var result = ...Call()`. The renamed loop variable must also propagate into the
/// per-item field accessors, not just the `foreach` header. ~keep
#[test]
fn an_iterate_item_named_result_is_renamed_to_avoid_shadowing_the_call_result() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "batch_present", "description": "Present a batch of results", "input": null,
        "docs": {"topic": "guides", "presentation": {"operations": [
            {"op": "iterate", "path": "results", "item": "result", "fields": ["label"]}
        ]}}
    }))
    .expect("fixture");
    let e2e = E2eConfig {
        call: CallConfig {
            function: "process_batch".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: ["results".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample_core".into(),
        ..ResolvedCrateConfig::default()
    };

    let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

    assert!(
        body.contains("var result = SampleCoreConverter.ProcessBatch();"),
        "{body}"
    );
    assert!(
        !body.contains("foreach (var result in"),
        "the loop variable must not shadow the outer `result` binding (CS0136):\n{body}"
    );
    assert!(body.contains("foreach (var resultItem in result.Results)"), "{body}");
    assert!(body.contains("Console.WriteLine(resultItem.Label);"), "{body}");
}

/// Unit-level companion to the regression above, isolating `disambiguate_presentation_items` from
/// the full render pipeline: a `show` operation's empty `item` must never match the reserved set
/// (it has no loop variable to collide), and a sibling `iterate` whose item already happens to
/// spell the renamed candidate (`resultItem`) is left alone -- each `foreach` is its own
/// non-overlapping scope, so two sibling loops sharing a loop-variable name is not a collision
/// the way shadowing the enclosing `result` binding is.
#[test]
fn disambiguation_skips_show_operations_and_leaves_non_colliding_siblings_alone() {
    fn iterate_op(item: &str) -> crate::e2e::codegen::presentation::PresentationOperation {
        crate::e2e::codegen::presentation::PresentationOperation {
            kind: "iterate",
            expression: "result.Items".to_string(),
            item: item.to_string(),
            fields: vec![format!("{item}.Label")],
            optional: false,
            display: false,
            destructure_source: String::new(),
            destructure_item: String::new(),
            shown_optional: false,
            field_optionals: vec![false],
            field_displays: vec![false],
            guard_binding: String::new(),
            guard_source: String::new(),
            guard_condition: String::new(),
        }
    }
    let show = crate::e2e::codegen::presentation::PresentationOperation {
        kind: "show",
        expression: "result.Count".to_string(),
        item: String::new(),
        fields: Vec::new(),
        optional: false,
        display: false,
        destructure_source: String::new(),
        destructure_item: String::new(),
        shown_optional: false,
        field_optionals: Vec::new(),
        field_displays: Vec::new(),
        guard_binding: String::new(),
        guard_source: String::new(),
        guard_condition: String::new(),
    };
    let operations = vec![show, iterate_op("result"), iterate_op("resultItem")];

    let disambiguated = disambiguate_presentation_items(operations, "result", false);

    assert_eq!(
        disambiguated[0].item, "",
        "a show operation's item must be left untouched"
    );
    assert_eq!(
        disambiguated[1].item, "resultItem",
        "must rename to avoid shadowing the outer `result`"
    );
    assert_eq!(disambiguated[1].fields, vec!["resultItem.Label".to_string()]);
    assert_eq!(
        disambiguated[2].item, "resultItem",
        "a sibling loop's own name is not a collision"
    );
}
