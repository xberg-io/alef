//! Regression coverage for `Option` normalization on streaming-virtual field assertions.
//!
//! Split into its own file rather than added to `rust/assertions.rs`: that file is already at
//! the repo's 1,000-line cap ceiling recorded in `tests/file_size_baseline.txt` (see
//! `file-modularization` in CLAUDE.md), so new coverage goes into a fresh module. ~keep
//!
//! Before the fix, `assertion_streaming` decided whether to append `.as_ref()` from
//! `FieldResolver::is_optional` alone — the *declared* IR type of the asserted field. The
//! expression actually emitted comes from `StreamingFieldResolver::accessor`, which for every
//! field but the collected-list passthrough builds a chain that has already flattened the
//! `Option` away and pinned a concrete type. Appending `.as_ref()` to those produced
//! `error[E0282]: type annotations needed` in the generated test, because `Vec<T>` and `String`
//! each implement `AsRef` for several targets and nothing constrains the parameter.

use std::collections::{HashMap, HashSet};

use super::assertion_streaming::try_render_streaming_virtual_field_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// A resolver that reports `field` as optional — the state the IR merge produces for any
/// `Option<T>`-declared field, and the state that used to drive the `.as_ref()` append.
fn optional_resolver(field: &str, array_fields: &[&str]) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::from([field.to_string()]),
        &HashSet::new(),
        &array_fields
            .iter()
            .map(ToString::to_string)
            .collect::<HashSet<String>>(),
        &HashSet::new(),
    )
}

fn render(field: &str, assertion_type: &str, array_fields: &[&str]) -> String {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!(1)),
        ..Default::default()
    };
    let resolver = optional_resolver(field, array_fields);
    let mut out = String::new();
    let handled =
        try_render_streaming_virtual_field_assertion(&mut out, &assertion, "sample_dep", &resolver, None, true);
    assert!(
        handled,
        "'{field}' must be handled as a streaming-virtual field, got: {out}"
    );
    out
}

#[test]
fn should_not_append_as_ref_to_nested_flat_map_tool_calls_chain_when_field_is_declared_optional() {
    let out = render("tool_calls", "not_empty", &[]);
    assert!(
        out.contains("flat_map") && out.contains("collect::<Vec<_>>()"),
        "expected the flattened tool_calls chain, got: {out}"
    );
    assert!(
        !out.contains("collect::<Vec<_>>().as_ref()"),
        "`Vec<T>` implements `AsRef` for more than one target, so a bare `.as_ref()` on the \
         collected chain is an unannotatable `AsRef<T>` call (E0282) in the generated test; \
         got: {out}"
    );
    assert!(
        out.contains("!chunks.iter().flat_map"),
        "the collected chain is a `Vec`, so emptiness must be checked with `!expr.is_empty()`; \
         got: {out}"
    );
}

#[test]
fn should_not_append_as_ref_to_collected_tool_calls_chain_for_count_min() {
    let out = render("tool_calls", "count_min", &[]);
    assert!(
        !out.contains(".as_ref().map_or(0, |v| v.len())"),
        "the collected chain is a `Vec`, so its length is `.len()`, not an Option projection; \
         got: {out}"
    );
    assert!(
        out.contains("collect::<Vec<_>>().len()"),
        "expected `.collect::<Vec<_>>().len()`, got: {out}"
    );
}

/// Sibling shape: `finish_reason` is `Option<...>` on the wire, so the same declared-optional
/// signal fired for it — but its accessor already ends in `.unwrap_or_default()` and yields a
/// `String`, which implements `AsRef` for `str`, `[u8]`, `OsStr` and `Path`. Same E0282, no
/// `flat_map` involved.
#[test]
fn should_not_append_as_ref_to_unwrap_or_default_finish_reason_string() {
    let out = render("finish_reason", "not_empty", &[]);
    assert!(
        !out.contains(".unwrap_or_default().as_ref()"),
        "`String` implements `AsRef` for several targets, so appending `.as_ref()` to the \
         finish_reason accessor is unannotatable (E0282); got: {out}"
    );
    assert!(
        out.contains("!chunks.last()") && out.contains(".unwrap_or_default().is_empty()"),
        "expected a direct `!expr.is_empty()` on the `String` accessor, got: {out}"
    );
}

/// Control: the one accessor shape whose type the caller decides. `chunks` / `stream.items`
/// resolve to the collected-list local verbatim, and for a non-streaming fixture whose result
/// struct really has an `Option<Vec<T>>` field of that name, `render_test_function` binds
/// `let chunks = &result.chunks;`. `is_some_and` takes `self` by value, so that borrow genuinely
/// needs `.as_ref()` — a blanket removal of the append would regress this shape.
#[test]
fn should_still_append_as_ref_when_accessor_passes_the_declared_optional_local_through() {
    let out = render("chunks", "not_empty", &["chunks"]);
    assert!(
        out.contains("chunks.as_ref().is_some_and(|v| !v.is_empty())"),
        "the passthrough accessor keeps the declared `Option` wrapper, so the borrow still needs \
         `.as_ref()` before `is_some_and`; got: {out}"
    );
}

#[test]
fn should_still_append_as_ref_on_passthrough_local_for_is_empty() {
    let out = render("chunks", "is_empty", &["chunks"]);
    assert!(
        out.contains("chunks.as_ref().is_none_or(|v| v.is_empty())"),
        "the passthrough accessor keeps the declared `Option` wrapper; got: {out}"
    );
}
