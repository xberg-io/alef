//! What shape the swift-bridge getter for an assertion's LEAF segment has, and which assertions
//! that shape makes unspellable.
//!
//! Split out of `assertions.rs` because these are one concern with one source of truth — the
//! binding's own getter classification, carried on `SwiftFirstClassMap` — consulted by several
//! unrelated arms of the assertion renderer. Keeping the verdicts here means `assertions.rs`
//! decides what to *emit* while this module decides what the leaf *is*.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// Suffixes that ask for a collection's element count.
const COUNT_SUFFIXES: [&str; 3] = ["length", "count", "size"];

/// Render the skip line for a path that steps *past* a JSON-bridged leaf, if it does.
///
/// ~keep swift-bridge collapses a JSON-bridged field to one `RustString`, so the leaf has neither
/// `.count` nor a subscript, and every way of stepping past it is equally unspellable. The guard
/// this replaced was keyed on the trailing accessor's spelling, so it caught a count suffix and
/// missed an index or wildcard on the very same field — the generator wrote the correct
/// "JSON-bridges it to RustString" skip for one and a broken assertion for the other, on adjacent
/// lines. Deciding from the single fact that makes any of them impossible collapses four cases
/// into one.
pub(super) fn json_bridged_traversal_skip(field_resolver: &FieldResolver, field: Option<&str>) -> Option<String> {
    let field = field.filter(|f| !f.is_empty())?;
    let bridged = field_resolver.swift_json_bridged_traversal_prefix(field)?;
    Some(skip_line(FieldSkip::CountOnJsonBridgedLeafInSwift, &bridged))
}

/// Render the skip line for a count suffix whose collection leaf is not a countable `RustVec`.
///
/// ~keep Runs only after `is_valid_for_result` accepted the path, so the field IS resolvable, and
/// `NotAvailableOnResultType` — an `AuthoringGap`, therefore fatal under the strict gate — was the
/// wrong wording for it: the backend dropped the assertion as an honest ABI limit while the gate
/// demanded the consumer repair a field path that was never wrong, two verdicts about one fact
/// with nothing comparing them. `CountOnJsonBridgedLeafInSwift` states the real reason and carries
/// the classification that reason implies.
///
/// Broader than [`json_bridged_traversal_skip`] on purpose: it also refuses a count on a leaf the
/// IR never described, where emitting `.count` would be a guess.
pub(super) fn non_countable_leaf_count_skip(field_resolver: &FieldResolver, field: Option<&str>) -> Option<String> {
    let field = field?;
    let collection = COUNT_SUFFIXES
        .iter()
        .find_map(|suffix| field.strip_suffix(&format!(".{suffix}")))?;
    if collection.is_empty() || field_resolver.leaf_is_vec_via_swift_map(field_resolver.resolve(collection)) {
        return None;
    }
    Some(skip_line(FieldSkip::CountOnJsonBridgedLeafInSwift, field))
}

/// Recover a real element count for a `count_min`/`count_equals` assertion whose field IS a
/// JSON-bridged collection leaf, rather than refusing it outright.
///
/// ~keep swift-bridge collapses `Option<Vec<T>>` / `Vec<Vec<_>>` / a map getter to one
/// `RustString` of JSON text, so `swift_count_target` correctly refuses `.count` on the raw
/// getter -- but the JSON text IS the serialized collection, and `JSONSerialization` (already
/// available: every generated e2e file imports `Foundation`) can parse it back into a Swift
/// `[Any]` whose `.count` is the real element count, with no matching `Codable` type required on
/// the Swift side. `null` (Rust's `None`) and any text that fails to parse as a JSON array both
/// read as zero elements, which is the correct count for an absent `Option<Vec<T>>`.
///
/// Scoped to the field being the bridged leaf itself, not a path that steps PAST it to a nested
/// element (`chunks[0].content` still refuses via [`json_bridged_traversal_skip`], which fires
/// earlier and returns before this is ever reached) -- decoding one further index or key
/// generically is a distinct, larger feature this function does not attempt.
pub(super) fn swift_json_bridged_count_expr(
    field_resolver: &FieldResolver,
    field: Option<&str>,
    field_expr: &str,
) -> Option<String> {
    let field = field.filter(|f| !f.is_empty())?;
    let resolved = field_resolver.resolve(field);
    let is_collection = field_resolver.is_array(field)
        || field_resolver.is_array(resolved)
        || field_resolver.is_collection_root(field)
        || field_resolver.is_collection_root(resolved);
    if !is_collection || !field_resolver.leaf_is_json_bridged_via_swift_map(resolved) {
        return None;
    }
    // An intermediate `?.` in the chain (an optional ancestor) makes the whole chain
    // `Optional<RustString>`, so the trailing call needs its own `?.` too; coalesce the missing
    // case to the JSON text for an absent collection ("null") rather than an empty string, which
    // is not valid JSON and would make `jsonObject(with:)` throw regardless of the true count.
    let json_text_expr = if field_expr.contains("?.") {
        format!("({field_expr}?.toString() ?? \"null\")")
    } else {
        format!("{field_expr}.toString()")
    };
    Some(format!(
        "((try? JSONSerialization.jsonObject(with: Data({json_text_expr}.utf8))) as? [Any])?.count ?? 0"
    ))
}

#[cfg(test)]
mod json_bridged_count_tests {
    use super::swift_json_bridged_count_expr;
    use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
    use std::collections::{HashMap, HashSet};

    fn resolver_with_json_bridged_array(field_name: &str) -> FieldResolver {
        let swift_first_class_map = SwiftFirstClassMap {
            json_bridged_field_names: HashSet::from([field_name.to_string()]),
            ..SwiftFirstClassMap::default()
        };
        FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([field_name.to_string()]),
            &HashSet::new(),
            &HashMap::new(),
            swift_first_class_map,
        )
    }

    /// The confirmed-recoverable case: `results[0].chunks` is both a known array field (per
    /// `fields_array`) and a JSON-bridged leaf, so a count is expressible by decoding the bridged
    /// `RustString` rather than refusing outright.
    #[test]
    fn bare_json_bridged_array_leaf_yields_a_real_decode_and_count_expression() {
        let resolver = resolver_with_json_bridged_array("chunks");

        let expr = swift_json_bridged_count_expr(&resolver, Some("chunks"), "result.results()[0].chunks()");

        let Some(expr) = expr else {
            panic!("a JSON-bridged array leaf must yield a real count expression");
        };
        assert_eq!(
            expr,
            "((try? JSONSerialization.jsonObject(with: Data(result.results()[0].chunks().toString().utf8))) \
             as? [Any])?.count ?? 0"
        );
    }

    /// An optional ancestor in the chain (`?.`) makes the whole expression `Optional<RustString>`,
    /// so the trailing call must also use `?.` and coalesce a missing value to valid JSON ("null")
    /// rather than an empty string, which `JSONSerialization` would reject regardless of the true
    /// count.
    #[test]
    fn optional_ancestor_chain_coalesces_to_null_json_text() {
        let resolver = resolver_with_json_bridged_array("chunks");

        let expr = swift_json_bridged_count_expr(&resolver, Some("chunks"), "result.document()?.chunks()");

        let Some(expr) = expr else {
            panic!("a JSON-bridged array leaf behind an optional ancestor must still yield a count expression");
        };
        assert!(
            expr.contains("(result.document()?.chunks()?.toString() ?? \"null\")"),
            "got: {expr}"
        );
    }

    /// A field that is neither an array nor a JSON-bridged leaf (an ordinary scalar) must not get
    /// a decode-and-count expression — that would silently count the characters of an arbitrary
    /// string's JSON parse failure (always `None` from `jsonObject`) as zero, masking the real
    /// reason no assertion should have been rendered here in the first place.
    #[test]
    fn non_collection_field_yields_no_count_expression() {
        let empty = HashSet::new();
        let resolver = FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty);

        let expr = swift_json_bridged_count_expr(&resolver, Some("title"), "result.title()");

        assert!(expr.is_none(), "got: {expr:?}");
    }

    /// No field path (a bare-result count) is out of scope for this recovery — it addresses only
    /// a named collection LEAF, mirroring every other function in this module.
    #[test]
    fn no_field_yields_no_count_expression() {
        let resolver = resolver_with_json_bridged_array("chunks");

        let expr = swift_json_bridged_count_expr(&resolver, None, "result");

        assert!(expr.is_none(), "got: {expr:?}");
    }
}

/// Render the skip line for an emptiness assertion whose field every collection oracle calls a
/// collection, but whose Swift leaf is a JSON-bridged `RustString`.
///
/// ~keep This is the guard that makes `not_empty`/`is_empty`'s degraded branch impossible to ship
/// silently. `field_is_array` is correctly `false` for such a leaf (the Swift surface really is a
/// string, so `.isEmpty` on it does not compile), which used to drop the assertion into the plain
/// `field_is_optional` arm and emit `XCTAssertTrue(<expr> != nil, "expected non-empty value")`.
/// The bridged getter is declared non-optional, so that comparison is a tautology Swift only
/// warns about — a check that cannot fail, wearing a message claiming it can, which is strictly
/// worse than no check at all because it reads as coverage. There is no correct assertion to emit
/// instead: the bridged JSON text is non-empty (`"[]"`, `"null"`) for exactly the empty
/// collections the fixture is trying to rule out. Refusing loudly through the registered
/// [`FieldSkip`] funnel is the only honest option, and it is a limitation of the swift-bridge ABI
/// rather than anything a fixture or `alef.toml` edit can repair.
pub(super) fn unspellable_collection_emptiness_skip(
    field_resolver: &FieldResolver,
    field: Option<&str>,
) -> Option<String> {
    let field = field.filter(|f| !f.is_empty())?;
    let resolved = field_resolver.resolve(field);
    let is_collection = field_resolver.is_array(field)
        || field_resolver.is_array(resolved)
        || field_resolver.is_collection_root(field)
        || field_resolver.is_collection_root(resolved);
    if !is_collection || !field_resolver.leaf_is_json_bridged_via_swift_map(resolved) {
        return None;
    }
    Some(skip_line(FieldSkip::CountOnJsonBridgedLeafInSwift, field))
}

/// The skip line a count/emptiness arm renders when [`super::accessors::swift_count_target`]
/// refuses to name a countable target.
///
/// ~keep `count_min`/`count_equals` each wrote their own prose here ("is a scalar String without
/// meaningful .count", registered as `AssertionTypeSkip::ScalarWithoutMeaningfulCountInSwift`),
/// which names the wrong cause and files the skip under the wrong axis: the leaf is not a scalar
/// String misconfigured as an array, it is a real collection whose swift-bridge getter is one
/// JSON `RustString`, which is a property of the FIELD's shape, not of the assertion type. All
/// four arms now render one wording that states the actual fact. Every one of them was dead code
/// while `swift_count_target` returned `Some` on every path, so making it refuse is what makes
/// this line load-bearing at all.
pub(super) fn non_countable_leaf_skip_line(field: Option<&str>) -> String {
    skip_line(
        FieldSkip::CountOnJsonBridgedLeafInSwift,
        field
            .filter(|f| !f.is_empty())
            .unwrap_or(super::assertions::BARE_RESULT_TOKEN),
    )
}

/// Whether the leaf's own getter returns `Option<..>`, so a caller chaining onto the rendered
/// accessor must write `?.` rather than `.`.
///
/// ~keep The accessor renderer deliberately omits the leaf `?` — it cannot know what will be
/// chained on — and a `?.` already in the chain only proves an ANCESTOR was optional. Reading the
/// ancestor's `?` as evidence that the leaf was unwrapped emitted `.toString()` against an
/// `Optional<RustString>` leaf, which has no such member. `false` when the IR did not describe the
/// leaf, which preserves the pre-existing behaviour for unmapped fields.
pub(super) fn leaf_getter_is_optional(field_resolver: &FieldResolver, field: Option<&str>) -> bool {
    field
        .filter(|f| !f.is_empty())
        .and_then(|f| field_resolver.swift_leaf_getter_is_optional(f))
        .unwrap_or(false)
}

fn skip_line(kind: FieldSkip, field: &str) -> String {
    format!("        // skipped: {}\n", kind.message(field))
}

/// Render the skip line for a field-access chain this generator refuses to build at all: a
/// string-key (JSON-bridged map) subscript followed by a further `RustVec` subscript.
///
/// ~keep [`json_bridged_traversal_skip`] already refuses this shape when the swift-bridge scan
/// positively classified the map field as JSON-bridged, before an accessor is ever built. A
/// resolver built without IR data (config-only fixtures, or a call site that never wired
/// `with_ir_fields`) never populates that classification, so the mixed path can still reach
/// [`super::accessors::materialise_vec_temporaries`], which reports the hazard by returning
/// `None` rather than hoisting a `RustVec` subscript against the plain Swift `String` a decoded
/// map value actually is. See that function's own doc for the full mechanism.
pub(super) fn mixed_map_then_vec_traversal_skip(field: &str) -> String {
    skip_line(FieldSkip::MixedMapThenVecTraversalInSwift, field)
}

#[cfg(test)]
mod mixed_map_then_vec_tests {
    use super::json_bridged_traversal_skip;
    use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
    use std::collections::{HashMap, HashSet};

    fn ir_backed_resolver() -> FieldResolver {
        let swift_first_class_map = SwiftFirstClassMap {
            json_bridged_field_names: HashSet::from(["labels".to_string()]),
            ..SwiftFirstClassMap::default()
        };
        FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            swift_first_class_map,
        )
    }

    /// The confirmed-safe direction: when the swift-bridge scan positively classifies `labels`
    /// as JSON-bridged (real IR data wired in), a mixed map-then-vec fixture path is refused
    /// HERE, before any accessor is built — `accessors::materialise_vec_temporaries`'s own
    /// defensive refusal never has to run for a resolver built this way. ~keep
    #[test]
    fn ir_backed_json_bridged_map_field_is_refused_before_an_accessor_is_built() {
        let resolver = ir_backed_resolver();

        let skip = json_bridged_traversal_skip(&resolver, Some("labels[key].items[0]"));

        let Some(skip) = skip else {
            panic!("IR-backed json-bridged map field must be refused before accessor-building");
        };
        assert!(skip.contains("'labels'"), "got: {skip}");
    }

    /// The reachable gap review flagged: an IR-less / config-opaque resolver never populates
    /// `json_bridged_field_names` (it stays empty), so this same fixture shape is NOT refused
    /// here — it falls through to accessor building, where
    /// `accessors::materialise_vec_temporaries` must catch it instead (see that module's own
    /// `mixed_map_then_vec_subscript_is_refused` test). ~keep
    #[test]
    fn opaque_resolver_does_not_refuse_here_the_gap_is_closed_downstream() {
        let empty = HashSet::new();
        let resolver = FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty);

        let skip = json_bridged_traversal_skip(&resolver, Some("labels[key].items[0]"));

        assert!(
            skip.is_none(),
            "an IR-less resolver has no positive fact to refuse on; got: {skip:?}"
        );
    }
}
