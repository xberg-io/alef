//! Regression coverage for `accessors::materialise_vec_temporaries`'s handling of OPTIONAL
//! (`()?[...]`) subscripts, the outer marker scanner's quote-awareness, and the consistent
//! refusal posture the function now uses for every kind of unsafe rewrite.
//!
//! Split into its own file rather than added inline: `accessors.rs`'s own test module already
//! covers the non-optional hoisting/refusal shapes, and `assertions.rs` sits exactly at its
//! `tests/file_size_baseline.txt` ceiling (1075 lines, zero headroom) — new coverage has to live
//! somewhere else regardless of which production file it exercises.

use super::accessors::materialise_vec_temporaries;
use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

/// A single optional `RustVec` subscript (`()?[0]`, e.g. an `Option<Vec<T>>` field) must hoist
/// its temporary exactly like a non-optional one, but the rewritten expression must keep the `?`
/// before the subscript — dropping it would subscript an `Optional<RustVec<T>>` local directly,
/// which does not compile.
#[test]
fn single_optional_subscript_preserves_the_question_mark() {
    let (setup, rewritten, is_map_subscript) =
        materialise_vec_temporaries("result.items()?[0].value()", "is_true_1").unwrap();

    assert_eq!(setup, vec!["let _vec_items_is_true_1_1 = result.items()".to_string()]);
    assert_eq!(rewritten, "_vec_items_is_true_1_1?[0].value()");
    assert!(!is_map_subscript);
}

/// The confirmed defect: a nested chain where the FIRST subscript is optional (`items()?[0]`)
/// and the SECOND is written PLAIN in the source text (`nested()[1]`, no `?`) because the
/// original (non-hoisted) generator only writes the first `?` in a chain and lets Swift's
/// optional-chaining auto-propagate through everything textually after it in ONE continuous
/// expression — `a?.b.c` needs no second `?` before `.c`. Splitting that chain across separate
/// `let` bindings breaks the continuity: `_vec_items_1?[0]` is a standalone `Item?`, and a PLAIN
/// `.nested()` on it does not compile — `_vec_items_1?[0].nested()` (still one continuous
/// optional-chain expression, fine as the SECOND hoist's own `let` RHS) is required, and the
/// THIRD segment then needs its OWN `?` before its subscript too. An implementation that reads
/// optionality only from each hoist's own literal marker text would emit
/// `_vec_nested_2[1].value()` here — no `?` at all — which is a compile error.
#[test]
fn nested_chain_carries_optionality_across_a_later_plain_marker() {
    let (setup, rewritten, is_map_subscript) =
        materialise_vec_temporaries("result.items()?[0].nested()[1].value()", "is_true_2").unwrap();

    assert_eq!(
        setup,
        vec![
            "let _vec_items_is_true_2_1 = result.items()".to_string(),
            "let _vec_nested_is_true_2_2 = _vec_items_is_true_2_1?[0].nested()".to_string(),
        ]
    );
    assert_eq!(rewritten, "_vec_nested_is_true_2_2?[1].value()");
    assert!(!is_map_subscript);
}

/// The addendum-reported defect: the OUTER marker scanner used to be quote-blind. A map key's
/// own content can legitimately contain the literal text `()[` — `quoted_key_literal` never
/// escapes brackets, only `\`, `"`, and whitespace control characters. Once hoisted, that text
/// sits verbatim inside the rewritten quoted subscript. This key is the TERMINAL (only)
/// subscript, so a quote-blind second scan would find a FAKE "next marker" embedded in the
/// key's own already-hoisted content — where none actually exists — and attempt a bogus second
/// hoist, splitting the key. The fix must still find exactly ONE hoist and keep the key intact.
#[test]
fn terminal_map_key_containing_parens_and_bracket_does_not_trigger_a_bogus_second_hoist() {
    let (setup, rewritten, is_map_subscript) =
        materialise_vec_temporaries("result.labels()[\"a()[b\"]", "equals_zz").unwrap();

    assert_eq!(
        setup,
        vec![
            "let _vec_labels_equals_zz_1 = (try? JSONSerialization.jsonObject(with: \
             (result.labels().toString() ?? \"{}\").data(using: .utf8)!) as? [String: String]) ?? [:]"
                .to_string(),
        ]
    );
    assert_eq!(rewritten, "_vec_labels_equals_zz_1[\"a()[b\"]");
    assert!(is_map_subscript);
}

/// `find_subscript_close` claims escape awareness; prove it against a key containing an escaped
/// quote (`a\"b`, i.e. the literal `"a\"b"` bracket content). A naive `find('"')` after the
/// escaped one would treat the ESCAPED quote as the closing delimiter and misparse the rest.
#[test]
fn key_with_an_escaped_quote_finds_the_true_closing_quote() {
    let (setup, rewritten, is_map_subscript) =
        materialise_vec_temporaries("result.labels()[\"a\\\"b\"]", "equals_eq").unwrap();

    assert_eq!(
        setup,
        vec![
            "let _vec_labels_equals_eq_1 = (try? JSONSerialization.jsonObject(with: \
             (result.labels().toString() ?? \"{}\").data(using: .utf8)!) as? [String: String]) ?? [:]"
                .to_string(),
        ]
    );
    assert_eq!(rewritten, "_vec_labels_equals_eq_1[\"a\\\"b\"]");
    assert!(is_map_subscript);
}

/// Companion escape case: a key whose raw content ends in a backslash (`a\`) is escaped by
/// `quoted_key_literal` into a DOUBLED trailing backslash before the closing quote (`"a\\"`). A
/// scanner that steps one byte at a time without escape-pairing would see the first of those two
/// backslashes as escaping the SECOND, mis-locating the closing quote one byte early.
#[test]
fn key_ending_in_a_backslash_finds_the_true_closing_quote() {
    let (setup, rewritten, is_map_subscript) =
        materialise_vec_temporaries("result.labels()[\"a\\\\\"]", "equals_bs").unwrap();

    assert_eq!(
        setup,
        vec![
            "let _vec_labels_equals_bs_1 = (try? JSONSerialization.jsonObject(with: \
             (result.labels().toString() ?? \"{}\").data(using: .utf8)!) as? [String: String]) ?? [:]"
                .to_string(),
        ]
    );
    assert_eq!(rewritten, "_vec_labels_equals_bs_1[\"a\\\\\"]");
    assert!(is_map_subscript);
}

/// The reachable hazard, OPTIONAL variant: a string-key (map) subscript followed by a FURTHER
/// **optional** `RustVec` subscript (`()?[`, not `()[`). The original hazard check only tested
/// the tail for `()[`, so an optional continuation slipped through undetected and would have
/// hoisted a `RustVec`-style subscript against the plain `String` a decoded map value actually
/// is. Both spellings of "there's more subscripting after this map read" must refuse.
#[test]
fn mixed_map_then_optional_vec_subscript_is_refused() {
    let result = materialise_vec_temporaries("result.labels()[\"key\"].items()?[0]", "not_empty_9");

    assert!(result.is_none(), "got: {result:?}");
}

/// The addendum's own reported example, run end to end: a map key containing `()[` immediately
/// followed by a real further subscript. The hazard check (tail contains another marker) refuses
/// this regardless of the outer-scanner fix — this test proves the refusal is a clean `None`,
/// not a panic or a garbled partial rewrite, when both defects could in principle interact.
#[test]
fn map_key_containing_bracket_followed_by_a_real_further_subscript_is_refused_cleanly() {
    let result = materialise_vec_temporaries("result.labels()[\"a()[b\"].nested()[0]", "equals_ex");

    assert!(result.is_none(), "got: {result:?}");
}

/// Consistency check: a malformed subscript with no closing `]` at all used to `break` the loop
/// silently, returning a PARTIALLY rewritten expression as if it were complete — a different,
/// weaker posture than the mixed-hazard `None` refusal a few lines above. Both failure shapes now
/// refuse the same way: the whole call returns `None`, never a partial result.
#[test]
fn a_subscript_with_no_closing_bracket_refuses_instead_of_returning_a_partial_rewrite() {
    let result = materialise_vec_temporaries("result.items()[0", "malformed_1");

    assert!(result.is_none(), "got: {result:?}");
}

/// Integration proof: routed through the REAL `render_assertion` entry point with a
/// config-only/opaque `FieldResolver` (no IR data, mirroring a resolver `with_ir_fields` was
/// never called for), a fixture field naming a plain map subscript (`labels[key]`) followed by
/// an OPTIONAL vec index (`items` marked optional) must resolve to
/// `result.labels()["key"].items()?[0]` and then reach the REGISTERED
/// `MixedMapThenVecTraversalInSwift` skip line — not emit a hoisted-but-broken accessor. The
/// earlier IR-backed guard (`json_bridged_traversal_skip`) cannot catch this: it only fires when
/// the swift-bridge scan positively recorded `labels` as JSON-bridged, which an empty
/// `SwiftFirstClassMap` never does.
#[test]
fn mixed_optional_map_then_vec_reaches_the_registered_skip_via_render_assertion() {
    let resolver = FieldResolver::new_with_swift_first_class(
        &HashMap::new(),
        &HashSet::from(["labels.items".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        SwiftFirstClassMap::default(),
    );
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("labels[key].items[0]".to_string()),
        value: Some(serde_json::json!("x")),
        ..Default::default()
    };

    let mut out = String::new();
    super::assertions::render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
        "Sample",
    );

    assert!(
        out.contains("mixes a JSON-bridged map subscript with a further RustVec subscript in Swift"),
        "must emit the registered mixed map-then-vec skip; got:\n{out}"
    );
    assert!(
        !out.contains("_vec_"),
        "must not emit a hoisted-but-refused accessor local; got:\n{out}"
    );
}
