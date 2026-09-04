//! Regression coverage for `is_empty` on a collection-typed field (alef defect: Elixir arm).
//!
//! `render_assertion`'s `is_empty` arm had no collection branch at all -- only
//! `assert is_nil(field) or coerced_field == ""` -- while its `not_empty` counterpart a few
//! lines above already treats `[]`/`%{}` as empty via `field not in [nil, "", [], %{}]`. An
//! empty `Vec<T>` field (Elixir `[]`) is neither `nil` nor `""`, so `is_empty` on such a field
//! was simply false and could never pass.
//!
//! Reproduces a consumer's `data_extraction_properties_empty` /
//! `data_extraction_json_empty_object` fixtures (`is_empty` on `data.children`, a
//! `Vec<DataNode>` reached through the `data` struct):
//!
//! ```text
//! 1) test data_extraction_properties_empty (E2e.DataExtractionTest)
//!    Expected truthy, got false
//!    code: assert is_nil(result.data.children) or result.data.children == ""
//! ```
//!
//! Lives in its own file rather than growing `assertions.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn array_resolver(field: &str) -> FieldResolver {
    let array: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &array,
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
        "SampleModule",
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

/// The exact tslp shape: `is_empty` on a `Vec<T>` field (`data.children`) must accept the
/// empty-list wire form, not just nil/"". Pinned to the exact rendered line so a revert of the
/// fix's `in [nil, "", [], %{}]` membership check reproduces the original always-false
/// assertion again.
#[test]
fn is_empty_on_array_field_accepts_empty_list_and_map_forms() {
    let out = render(
        &array_resolver("data.children"),
        &assertion("is_empty", "data.children"),
    );
    assert_eq!(
        out, "      assert result.data.children in [nil, \"\", [], %{}]\n",
        "got: {out}"
    );
}

/// Symmetry control: `is_empty` and `not_empty` on the same collection field must test the
/// exact same four-element membership list (`in` vs `not in`), or the two can silently drift
/// apart again the way `is_empty` alone drifted from `not_empty` before this fix.
#[test]
fn is_empty_and_not_empty_agree_on_the_empty_forms_for_a_collection_field() {
    let resolver = array_resolver("items");
    let is_empty_out = render(&resolver, &assertion("is_empty", "items"));
    let not_empty_out = render(&resolver, &assertion("not_empty", "items"));
    assert_eq!(is_empty_out, "      assert result.items in [nil, \"\", [], %{}]\n");
    assert_eq!(not_empty_out, "      assert result.items not in [nil, \"\", [], %{}]\n");
}

/// Negative control: an ordinary non-array, non-numeric scalar field must also take the
/// membership-list form -- proving the fix applies to `is_empty`'s whole non-numeric branch
/// (scalars included), not only fields registered as arrays.
#[test]
fn is_empty_on_ordinary_scalar_field_is_unchanged() {
    let out = render(&empty_resolver(), &assertion("is_empty", "title"));
    assert_eq!(out, "      assert result.title in [nil, \"\", [], %{}]\n", "got: {out}");
}

#[test]
fn is_empty_on_a_length_backed_expression_still_compares_to_zero() {
    // `is_numeric_expr` recognizes any accessor beginning with `length(` -- simulate that
    // shape directly via a resolver whose bare-result path renders as such. Field resolution
    // for ordinary named fields never itself emits `length(...)`, so this pins the numeric
    // branch's own literal comparison by asserting on `is_numeric_expr` is exercised through
    // a field the resolver treats as the whole (simple) result.
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion("is_empty", "unused"),
        "length(result.items)",
        &empty_resolver(),
        "SampleModule",
        &HashSet::new(),
        &HashMap::new(),
        true,
        false,
        false,
        false,
        false,
    );
    assert_eq!(out, "      assert length(result.items) == 0\n", "got: {out}");
}
