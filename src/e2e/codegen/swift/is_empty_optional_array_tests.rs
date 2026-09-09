//! Regression tests for `is_empty`/`not_empty` on a non-optional array field reached
//! through an optional PARENT (`data.children` where `data: Option<Data>` but
//! `children: Vec<T>` is not itself optional). Split into its own file rather than added
//! to `swift/assertions.rs`, which is already over the repo's 1,000-line cap (see
//! `file-modularization` in CLAUDE.md).
//!
//! Reproduces a consumer's `data_extraction_properties_empty` /
//! `data_extraction_json_empty_object` fixtures (`is_empty` on `data.children`), which
//! failed to COMPILE:
//!
//! ```text
//! error: optional type 'Bool?' cannot be used as a boolean; test for '!= nil' instead
//!         XCTAssertTrue(result.data()?.children().isEmpty, "expected empty value")
//! ```
//!
//! `field_is_array` was true (`children` is a `Vec<T>`) but `field_is_optional` was false
//! (`children`'s own type is not `Option<_>`) -- only `accessor_is_optional` (the `?.`
//! introduced by the optional PARENT `data`) sees the problem, and the `field_is_array` arm
//! never consulted it. ~keep

use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

/// A resolver shaped like tslp's `data.children`: `data` is an optional parent field,
/// `data.children` is a non-optional array field on it.
fn optional_parent_array_resolver() -> FieldResolver {
    let optional: HashSet<String> = ["data".to_string()].into_iter().collect();
    let array: HashSet<String> = ["data.children".to_string()].into_iter().collect();
    FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &array, &HashSet::new())
}

/// A resolver for a plain array field with no optional parent in the chain -- the negative
/// control proving the fix does not touch the already-correct unwrapped-accessor shape.
fn plain_array_resolver() -> FieldResolver {
    let array: HashSet<String> = ["items".to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &array,
        &HashSet::new(),
    )
}

fn assertion(assertion_type: &str, field: &str) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

fn render(resolver: &FieldResolver, assertion: &Assertion) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
        "Sample",
    );
    out
}

/// The exact tslp shape: `is_empty` on `data.children` must produce a `Bool`, not a `Bool?`,
/// by coalescing the optional-chained `.isEmpty` with `?? true` (absent parent counts as
/// empty). Pinned to the exact rendered line so a revert of the fix's `if accessor_is_optional`
/// branch reproduces the original `Bool?` compile error again.
#[test]
fn is_empty_on_array_field_through_optional_parent_coalesces_to_bool() {
    let out = render(
        &optional_parent_array_resolver(),
        &assertion("is_empty", "data.children"),
    );
    assert_eq!(
        out, "        XCTAssertTrue((result.data()?.children().isEmpty ?? true), \"expected empty value\")\n",
        "got: {out}"
    );
    assert!(
        !out.contains("isEmpty, \"expected"),
        "must not emit the bare Bool? form: {out}"
    );
}

/// `not_empty` has the identical latent shape (same `field_is_array`-only arm, no
/// `field_is_optional` on the field itself) -- not exercised by tslp's own fixtures, but the
/// same construct, so it must be fixed and pinned alongside `is_empty`.
#[test]
fn not_empty_on_array_field_through_optional_parent_coalesces_to_bool() {
    let out = render(
        &optional_parent_array_resolver(),
        &assertion("not_empty", "data.children"),
    );
    assert_eq!(
        out, "        XCTAssertTrue(result.data()?.children().isEmpty == false, \"expected non-empty value\")\n",
        "got: {out}"
    );
}

/// Negative control: a plain array field with no optional parent in the chain must keep
/// emitting the unwrapped `.isEmpty` form. If this test also passed with the fix reverted it
/// would prove nothing about the fix; it instead proves the `accessor_is_optional` branch is
/// additive, not a blanket rewrite of `field_is_array` handling.
#[test]
fn is_empty_on_plain_array_field_is_unchanged() {
    let out = render(&plain_array_resolver(), &assertion("is_empty", "items"));
    assert_eq!(
        out, "        XCTAssertTrue(result.items().isEmpty, \"expected empty value\")\n",
        "got: {out}"
    );
}

#[test]
fn not_empty_on_plain_array_field_is_unchanged() {
    let out = render(&plain_array_resolver(), &assertion("not_empty", "items"));
    assert_eq!(
        out, "        XCTAssertTrue(!result.items().isEmpty, \"expected non-empty value\")\n",
        "got: {out}"
    );
}
