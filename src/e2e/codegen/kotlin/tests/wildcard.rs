//! Kotlin bracket-wildcard assertion rendering tests, split out of `tests.rs`.

use super::super::assertions::{RenderAssertionContext, render_assertion};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn wildcard_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::from(["links".to_string()]),
        &HashSet::from(["links".to_string()]),
        &HashSet::new(),
    )
}

fn render_wildcard(field: &str, kotlin_android_style: bool) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!("internal")),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        RenderAssertionContext {
            result_var: "result",
            class_name: "",
            field_resolver: &wildcard_resolver(),
            result_is_simple: false,
            result_is_option: false,
            enum_fields: &HashSet::new(),
            json_scalar_fields: &HashSet::new(),
            fields_c_types: &HashMap::new(),
            is_streaming: false,
            kotlin_android_style,
            not_error_may_assert_presence: true,
        },
    );
    out
}

/// A bracket-wildcard fixture path means "every element", so the emitted Kotlin
/// must quantify with `any {}` over the whole array.
#[test]
fn kotlin_wildcard_contains_emits_any_over_all_elements() {
    let out = render_wildcard("links[].link_type", false);
    assert!(
        out.contains("result.links().any { e -> e.linkType().toString().contains(\"internal\") }"),
        "expected an any-element quantifier, got:\n{out}"
    );
}

/// THE CANARY. A fixture whose match lives only in element 1 is satisfied by an
/// any-element quantifier and missed by an index-0 lookup. This unit test can only
/// observe the emitted source, not execute it, so it pins the property that makes
/// the runtime difference: the wildcard must NOT lower to a single-element access.
/// Pre-fix the wildcard rendered `result.links().first().linkType()`, which reads
/// element 0 only and would report a false green; this assertion is red then.
#[test]
fn kotlin_wildcard_does_not_collapse_to_element_zero() {
    let out = render_wildcard("links[].link_type", false);
    assert!(
        !out.contains(".first()") && !out.contains("[0]") && !out.contains(".get(0)"),
        "wildcard must not lower to a single-element access, got:\n{out}"
    );
}

/// Regression lock: an explicit numeric index is not a wildcard and must keep
/// resolving to that exact element.
#[test]
fn kotlin_explicit_index_still_resolves_to_element_zero() {
    let out = render_wildcard("links[0].link_type", false);
    assert!(
        out.contains("result.links().first().linkType()"),
        "explicit index 0 must keep its index-preserving accessor, got:\n{out}"
    );
    assert!(
        !out.contains(".any {"),
        "explicit index must not become a quantifier, got:\n{out}"
    );
}

/// There is no separate kotlin_android assertion backend, so the element accessor
/// must be built with the SAME accessor language as the array accessor — otherwise
/// Android output silently emits JVM-style `()` getters inside the lambda.
#[test]
fn kotlin_android_wildcard_uses_property_accessors_on_both_sides() {
    let out = render_wildcard("links[].link_type", true);
    assert!(
        out.contains("result.links.any { e -> e.linkType.toString().contains(\"internal\") }"),
        "expected kotlin_android property accessors on both the array and the element, got:\n{out}"
    );
}
