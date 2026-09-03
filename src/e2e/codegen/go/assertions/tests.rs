//! Core `render_assertion` coverage: optional-field presence semantics, bracket-wildcard
//! traversal, and the IR-oracle vs `result_fields` precedence for field availability.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::*;
use crate::e2e::field_access::FieldResolver;
use std::collections::{HashMap, HashSet};

fn make_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

fn contains_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

/// Shared fixture setup: every test in this file renders one assertion against a
/// resolver-configured `result` value, with every other `AssertionRenderContext` field
/// held at its default (non-streaming, non-array, no optional locals) value.
fn render_with_resolver(assertion: &Assertion, resolver: &FieldResolver) -> String {
    let mut out = String::new();
    let context = AssertionRenderContext {
        effective_result_var: "result",
        import_alias: "pkg",
        field_resolver: resolver,
        optional_locals: &HashMap::new(),
        numeric_scalar_fields: &HashSet::new(),
        result_is_simple: false,
        result_is_array: false,
        is_streaming: false,
        streaming_item_type: None,
    };
    render_assertion(&mut out, &context, assertion);
    out
}

fn render_bare(assertion: &Assertion) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    render_with_resolver(assertion, &resolver)
}

/// Render `assertion` against a resolver where `optional_field` is the one field
/// declared `Option<T>` -- used to cover the Go `*T` (Option<struct>) presence-check
/// regression: `is_true` used to unconditionally deref (`*result.Data`), which
/// requires `T = bool` and does not compile for `Option<DataNode>`.
fn render_with_optional_field(assertion: &Assertion, optional_field: &str) -> String {
    let optional: HashSet<String> = [optional_field.to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    render_with_resolver(assertion, &resolver)
}

fn is_true_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_true".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

#[test]
fn is_true_on_optional_struct_field_checks_presence_not_a_bool_deref() {
    let out = render_with_optional_field(&is_true_assertion("data"), "data");
    assert_eq!(out, "\tassert.NotNil(t, result.Data, \"expected true (non-nil)\")\n");
}

#[test]
fn is_false_on_optional_struct_field_checks_absence() {
    let out = render_with_optional_field(
        &Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("data".to_string()),
            ..Default::default()
        },
        "data",
    );
    assert_eq!(out, "\tassert.Nil(t, result.Data, \"expected false (nil)\")\n");
}

#[test]
fn is_true_on_non_optional_field_is_unchanged() {
    let out = render_bare(&is_true_assertion("active"));
    assert_eq!(out, "\tassert.True(t, result.Active, \"expected true\")\n");
}

#[test]
fn wildcard_contains_scans_every_element_not_just_index_zero() {
    let out = render_bare(&contains_assertion("links[].link_type", "external"));
    assert!(out.contains("for _, e := range result.Links {"), "got: {out}");
    assert!(out.contains("e.LinkType"), "got: {out}");
    assert!(!out.contains("[0]"), "wildcard must not lower to index 0, got: {out}");
}

#[test]
fn explicit_numeric_index_still_targets_that_element() {
    let out = render_bare(&contains_assertion("links[0].link_type", "external"));
    assert!(out.contains("result.Links[0].LinkType"), "got: {out}");
    assert!(
        !out.contains("range"),
        "explicit index must not become a scan, got: {out}"
    );
}

/// Codegen-level canary for the wildcard defect. A fixture array whose match lives in
/// element 1 is only detected by code that visits every element; the pre-fix renderer
/// emitted a single `result.Links[0]` accessor, so this assertion pair fails against it.
/// It cannot execute the generated Go, so it pins the property structurally: an
/// element-1-only match is caught iff the emitted loop is unbounded and the value is
/// tested inside it. ~keep
#[test]
fn wildcard_match_in_element_one_is_reachable() {
    let out = render_bare(&contains_assertion("links[].link_type", "internal"));
    let loop_start = out
        .find("for _, e := range")
        .expect("expected an unbounded element scan");
    let check = out
        .find("strings.Contains")
        .expect("expected a per-element containment check");
    assert!(
        check > loop_start,
        "containment check must sit inside the scan, got: {out}"
    );
    assert!(
        !out.contains("result.Links[0]"),
        "an index-0 accessor would miss a match in element 1, got: {out}"
    );
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the emitted scan
/// ranged over `pages` while its body read `e.Links[0].Url` — a whole-array claim that
/// only ever inspected element zero of the inner slice. Pre-guard this test fails: the
/// skip line is absent and a `range` scan over `[0]` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render_bare(&contains_assertion("pages[].links[].url", "example.test"));
    assert_eq!(
        out, "\t// skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
        "got: {out}"
    );
}

#[test]
fn two_wildcard_assertions_on_one_array_use_distinct_locals() {
    let first = render_bare(&contains_assertion("links[].link_type", "external"));
    let second = render_bare(&contains_assertion("links[].link_type", "internal"));
    let local_of = |s: &str| {
        let start = s.find("found").expect("expected a found local");
        s[start..start + s[start..].find(' ').expect("local is space-delimited")].to_string()
    };
    assert_ne!(local_of(&first), local_of(&second), "locals must not collide");
}

/// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
/// (present, non-`binding_excluded`, on some IR type) but missing from the
/// hand-maintained `result_fields` config must still render a real assertion,
/// not a "skipped: field not available" comment — `go/test_function.rs` (and
/// the `go/test_file.rs` import-decision resolver) now thread
/// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
#[test]
fn go_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
    let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = make_assertion("data", "hello");
    let out = render_with_resolver(&assertion, &resolver);
    assert!(!out.contains("skipped"), "got: {out}");
}

/// The negative-control half of the same regression: `internal_diagnostics`
/// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
/// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
/// NOT `#[serde(skip)]`, which alone does not exclude a field from the
/// binding surface. Even though it is listed in `result_fields` (a stale/
/// wrong config entry), the IR must still win and reject it. ~keep
#[test]
fn go_ir_excluded_field_present_in_result_fields_is_still_skipped() {
    let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), excluded, HashSet::new());
    let assertion = make_assertion("internal_diagnostics", "hello");
    let out = render_with_resolver(&assertion, &resolver);
    assert!(out.contains("skipped"), "got: {out}");
}

fn not_empty_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

fn is_empty_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

/// `render_with_optional_field` marks "notes" as an `Option<T>` field with NO IR at all, so
/// `FieldResolver::target_field_is_pointer` returns `None` and (per `assertion_field_shape.rs`'s
/// documented `unwrap_or(false)` policy) resolves to "not a pointer". `not_empty` must honor that
/// exact answer instead of re-deriving pointer-ness from nullability alone: pre-fix, this emitted
/// `len(*result.Notes)` against a field the resolver never proved was a pointer -- the inverse of
/// the `SheetCount` defect. ~keep
#[test]
fn not_empty_on_optional_non_pointer_field_never_dereferences() {
    let out = render_with_optional_field(&not_empty_assertion("notes"), "notes");
    assert_eq!(
        out,
        "\tif result.Notes == nil || len(result.Notes) == 0 {\n\t\tt.Errorf(\"expected non-empty value\")\n\t}\n"
    );
}

/// Symmetric regression for `is_empty` -- see
/// `not_empty_on_optional_non_pointer_field_never_dereferences`. ~keep
#[test]
fn is_empty_on_optional_non_pointer_field_never_dereferences() {
    let out = render_with_optional_field(&is_empty_assertion("notes"), "notes");
    let expected = "\tif result.Notes != nil && len(result.Notes) != 0 {\n\
        \t\tt.Errorf(\"expected empty value, got %v\", result.Notes)\n\
        \t}\n";
    assert_eq!(out, expected);
}
