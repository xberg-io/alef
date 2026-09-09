//! Regression tests for `is_true`/`is_false` on an `Option<T>` field.
//!
//! Split into its own file rather than added to `swift/assertions.rs` or `swift/tests.rs`:
//! both are already over the repo's 1,000-line cap (see `file-modularization` in
//! CLAUDE.md), so new test coverage goes into a fresh module instead of growing either. ~keep

use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn optional_resolver(field: &str) -> FieldResolver {
    let optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn is_true_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_true".to_string(),
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

/// `Option<DataNode>` presence: before the fix this rendered `XCTAssertTrue(result.data())`
/// with no optional-chaining detected (the accessor for a LEAF optional field contains no
/// `?.` to trigger the pre-existing `accessor_is_optional` string check), so `Optional
/// <DataNode>` (or the opaque swift-bridge equivalent) was passed directly to
/// `XCTAssertTrue`, which requires `Bool` -- a compile error.
#[test]
fn is_true_on_optional_struct_field_checks_presence() {
    let out = render(&optional_resolver("data"), &is_true_assertion("data"));
    assert_eq!(out, "        XCTAssertNotNil(result.data())\n");
}

#[test]
fn is_false_on_optional_struct_field_checks_absence() {
    let out = render(
        &optional_resolver("data"),
        &Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("data".to_string()),
            ..Assertion::default()
        },
    );
    assert_eq!(out, "        XCTAssertNil(result.data())\n");
}

#[test]
fn is_true_on_non_optional_field_is_unchanged() {
    let out = render(&empty_resolver(), &is_true_assertion("active"));
    assert_eq!(out, "        XCTAssertTrue(result.active())\n");
}
