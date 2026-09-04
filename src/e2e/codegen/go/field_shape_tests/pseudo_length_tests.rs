//! Negative controls for the `count_min`/`count_equals`/`min_length`/`max_length`
//! no-silent-skip fixes in `assertion_render_helpers.rs`: a `.length`/`.count`/`.size`
//! PSEUDO field measures a derived scalar (e.g. a string's length) through an optional
//! pointer, and must stay guard-only on nil rather than gain a failing `else`.
//!
//! Split out of `field_shape_tests.rs`, which is at its recorded 1000-line cap.

use super::*;

/// Negative control for the `count_min`/`count_equals` no-silent-skip fix in
/// `assertion_render_helpers.rs::render_count_assertion`. `label.length` is a `.length`
/// PSEUDO field -- a derived scalar measurement (a string's length) taken through `label`'s
/// optional pointer, not a named collection field in its own right. `label` being nil means
/// "not populated", the same "no presence claim" semantics `render_guarded_scalar_comparison`
/// already gives optional scalars like `QualityScore` -- so this must stay guard-only, unlike
/// a real collection field (`elements`, `chunks`, `detected_languages`), which now fails on
/// nil instead of silently skipping. Mirrors the `go_batch`
/// `pointer_pseudo_length_count_min_nil_safe` case, which runs this exact rendered shape
/// against a nil `*string` and requires `Test_FieldShape` to PASS. ~keep
#[test]
fn pointer_pseudo_length_count_min_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.length",
        &[],
        false,
        "count_min",
        Some(serde_json::json!(1)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-length measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}

/// `count_equals` shares `render_count_assertion` with `count_min` -- confirm the `Equal`
/// method path gets the same pseudo-length exemption.
#[test]
fn pointer_pseudo_count_count_equals_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.count",
        &[],
        false,
        "count_equals",
        Some(serde_json::json!(1)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-count measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}

/// `render_length_assertion` sibling of `pointer_pseudo_length_count_min_stays_guard_only_on_nil`:
/// `min_length` on a `.length` pseudo field must get the identical exemption. ~keep
#[test]
fn pointer_pseudo_length_min_length_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.length",
        &[],
        false,
        "min_length",
        Some(serde_json::json!(1)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-length measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}

/// `max_length` shares `render_length_assertion` with `min_length` -- confirm the
/// `LessOrEqual` method path also gets the pseudo-length exemption.
#[test]
fn pointer_pseudo_size_max_length_stays_guard_only_on_nil() {
    let output = render_field_assertion(
        label_field(),
        "label.size",
        &[],
        false,
        "max_length",
        Some(serde_json::json!(10)),
    );
    assert!(
        output.contains("if result.Label != nil {"),
        "expected a guard on the optional pointer, got: {output}"
    );
    assert!(
        !output.contains("} else {"),
        "a pseudo-size measurement through an optional pointer must not gain a failing else, got: {output}"
    );
}
