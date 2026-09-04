//! Regression coverage for Elixir wildcard-field assertion traversal.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn array_resolver(field: &str) -> FieldResolver {
    let result_fields: HashSet<String> = [field.to_string()].into_iter().collect();
    let array_fields: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &array_fields,
        &HashSet::new(),
    )
}

fn render(assertion: &Assertion, resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        resolver,
        "Sample",
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        false,
        false,
        false,
    );
    out
}

fn contains_on(field: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String("beta".to_string())),
        ..Default::default()
    }
}

#[test]
fn elixir_wildcard_contains_quantifies_over_every_element() {
    let out = render(&contains_on("items[].name"), &array_resolver("items"));
    assert_eq!(
        out,
        "      assert Enum.any?((result.items || []), fn e -> String.contains?(to_string(e.name), \"beta\") end)\n",
        "got: {out}"
    );
}

/// Regression lock: an explicit numeric index is a different, correct feature and must
/// keep its exact `Enum.at/2` lowering (also pinned by tests/e2e_field_path_array.rs). ~keep
#[test]
fn elixir_explicit_index_still_lowers_to_enum_at() {
    let out = render(&contains_on("items[0].name"), &array_resolver("items"));
    assert!(out.contains("Enum.at(result.items, 0).name"), "got: {out}");
    assert!(!out.contains("Enum.any?"), "got: {out}");
}

/// CANARY. A code-generator unit test cannot execute Elixir, so it cannot literally run
/// a fixture whose only match lives in element 1. The observable proxy is exact: the
/// pre-fix renderer emitted `Enum.at(result.items, 0).name`, a lookup pinned to element
/// 0, so a value present only at element 1 could never be seen. This asserts no
/// positional lookup survives and the predicate is quantified over the whole list; it
/// fails against the pre-fix code, where `Enum.at(result.items, 0)` is present. ~keep
#[test]
fn elixir_wildcard_match_in_a_non_first_element_is_not_pinned_to_element_zero() {
    let mut assertion = contains_on("items[].name");
    assertion.value = Some(serde_json::Value::String("only-in-element-1".to_string()));
    let out = render(&assertion, &array_resolver("items"));
    assert!(!out.contains("Enum.at("), "index-pinned lookup survived: {out}");
    assert!(out.contains("Enum.any?((result.items || [])"), "got: {out}");
    assert!(out.contains("to_string(e.name)"), "got: {out}");
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the `Enum.any?`
/// ranged over `pages` while its body read `Enum.at(e.links, 0).url` — a whole-array
/// claim that only ever inspected element zero of the inner list. Pre-guard this test
/// fails: the skip line is absent and `Enum.at(e.links, 0)` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render(&contains_on("pages[].links[].url"), &array_resolver("pages"));
    assert_eq!(
        out, "      # skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
        "got: {out}"
    );
}
