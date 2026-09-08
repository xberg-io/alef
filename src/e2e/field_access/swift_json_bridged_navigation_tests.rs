//! Coverage for `FieldResolver::swift_json_bridged_navigation`, the positive-data sibling of
//! `swift_json_bridged_traversal_prefix` that records exactly HOW a fixture path steps past a
//! swift-bridge JSON-bridged leaf, so the swift e2e backend can decode-and-navigate instead of
//! refusing outright.

use crate::e2e::field_access::{FieldResolver, JsonNavStep, SwiftFirstClassMap};
use std::collections::{HashMap, HashSet};

fn resolver_with_json_bridged_field(field_name: &str) -> FieldResolver {
    let swift_first_class_map = SwiftFirstClassMap {
        json_bridged_field_names: HashSet::from([field_name.to_string()]),
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

/// The exact shape from `fixtures/contract/language_detection_config.json`: an `equals` on a
/// numeric-indexed element of a JSON-bridged array leaf, with nothing further after the index.
#[test]
fn indexed_element_directly_after_the_bridged_leaf_yields_one_index_step() {
    let resolver = resolver_with_json_bridged_field("detected_languages");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].detected_languages[0]")
        .expect("an indexed element after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].detected_languages");
    assert_eq!(steps, vec![JsonNavStep::Index(0)]);
}

/// The un-indexed projection shape: a dotted key with no bracket at all, e.g.
/// `results[0].metadata.output_format`.
#[test]
fn dotted_key_after_the_bridged_leaf_yields_one_key_step() {
    let resolver = resolver_with_json_bridged_field("metadata");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.output_format")
        .expect("a dotted key after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(steps, vec![JsonNavStep::Key("output_format".to_string())]);
}

/// Multiple dotted keys chain into multiple `Key` steps, deepest last.
///
/// `format.html.title` is NOT three real JSON keys: `metadata.format` resolves to
/// `FormatMetadata`, an internally-tagged serde enum whose wire form flattens the variant's
/// fields beside the `format_type` discriminator, so `html` is a typed-accessor variant
/// segment, not a JSON key. `JSONSerialization` would look up a `"html"` key that does not
/// exist. See `field_access::format_metadata_variants` for the full rationale; this is the
/// bug documented in the "swift e2e JSON navigation" defect this test now covers correctly. ~keep
#[test]
fn multiple_dotted_keys_chain_into_multiple_key_steps() {
    let resolver = resolver_with_json_bridged_field("metadata");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.html.title")
        .expect("a multi-segment dotted path after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("format".to_string()),
            JsonNavStep::Key("title".to_string()),
        ]
    );
}

/// CONTROL: a non-variant key that happens to follow `format` (i.e. `format` here is NOT the
/// `FormatMetadata` field but an ordinary nested key on some other object) must still be kept
/// as a real `Key` step -- the skip is anchored on the literal preceding segment being
/// `"format"`, exactly as zig's reference implementation is, not on some broader "any enum
/// variant name anywhere" rule. `revision` is not one of `FORMAT_METADATA_VARIANTS`, so it must
/// survive even directly after a `format` segment.
#[test]
fn a_non_variant_key_after_format_is_still_kept() {
    let resolver = resolver_with_json_bridged_field("metadata");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.revision")
        .expect("a dotted key after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("format".to_string()),
            JsonNavStep::Key("revision".to_string()),
        ]
    );
}

/// The exact regression this fix targets: `metadata.format.excel.sheet_count` must reach
/// `sheet_count` WITHOUT ever looking up a literal `"excel"` key -- `excel` is a
/// `FormatMetadata` variant name, and `FormatMetadata` is internally tagged, so there is no
/// `"excel"` key on the wire at all.
#[test]
fn excel_variant_segment_is_skipped_not_turned_into_a_key_step() {
    let resolver = resolver_with_json_bridged_field("metadata");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.excel.sheet_count")
        .expect("a variant-crossing dotted path after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("format".to_string()),
            JsonNavStep::Key("sheet_count".to_string()),
        ]
    );
    assert!(
        !steps.contains(&JsonNavStep::Key("excel".to_string())),
        "must not emit a Key step for the variant name, got: {steps:?}"
    );
}

/// Same shape as above for the `html` variant (`FormatMetadata::Html`), matching the second of
/// the two failing CI assertions this fix targets (`testMetadataAccess`).
#[test]
fn html_variant_segment_is_skipped_not_turned_into_a_key_step() {
    let resolver = resolver_with_json_bridged_field("metadata");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.html.title")
        .expect("a variant-crossing dotted path after a JSON-bridged leaf must be navigable");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("format".to_string()),
            JsonNavStep::Key("title".to_string()),
        ]
    );
    assert!(
        !steps.contains(&JsonNavStep::Key("html".to_string())),
        "must not emit a Key step for the variant name, got: {steps:?}"
    );
}

/// An index immediately after the leaf followed by further dotted keys — `chunks[0].content` —
/// mixes both step kinds in order.
#[test]
fn index_then_keys_mixes_step_kinds_in_order() {
    let resolver = resolver_with_json_bridged_field("chunks");

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].chunks[0].metadata.total_chunks")
        .expect("an index followed by dotted keys must be navigable");

    assert_eq!(leaf_field, "results[0].chunks");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Index(0),
            JsonNavStep::Key("metadata".to_string()),
            JsonNavStep::Key("total_chunks".to_string()),
        ]
    );
}

/// CONTROL: a wildcard `[]` has no numeric index to decode, so the walk must refuse rather than
/// invent one — this is the shape that must keep falling through to the existing skip.
#[test]
fn wildcard_bracket_is_not_navigable() {
    let resolver = resolver_with_json_bridged_field("headings");

    assert_eq!(resolver.swift_json_bridged_navigation("metadata.headings[].text"), None);
}

/// CONTROL: a string map-key bracket is not a numeric index either, and must refuse the same way.
#[test]
fn string_key_bracket_is_not_navigable() {
    let resolver = resolver_with_json_bridged_field("labels");

    assert_eq!(resolver.swift_json_bridged_navigation("labels[key].items"), None);
}

/// CONTROL: a trailing `.length`/`.count`/`.size` is the pre-existing synthetic virtual-count
/// idiom, not a literal JSON object key — the walk must stay out of that mechanism's way.
#[test]
fn trailing_count_suffix_is_left_to_the_existing_count_suffix_mechanism() {
    let resolver = resolver_with_json_bridged_field("og_locale_alternates");

    assert_eq!(
        resolver.swift_json_bridged_navigation("metadata.og_locale_alternates.length"),
        None
    );
}

/// A field that never traverses past a JSON-bridged leaf at all (an ordinary nested path) has
/// nothing for this walk to do.
#[test]
fn a_path_with_no_bridged_leaf_yields_no_navigation() {
    let empty = HashSet::new();
    let resolver = FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty);

    assert_eq!(resolver.swift_json_bridged_navigation("results[0].mime_type"), None);
}

/// A `SwiftFirstClassMap` anchored on `ExtractionResult -> ExtractedDocument -> Metadata ->
/// FormatMetadata`, where `metadata` is bridged on some unrelated type (poisoning the flat
/// `json_bridged_field_names` set) but explicitly NOT bridged on `ExtractedDocument`, while
/// `Metadata::format` IS bridged.
fn resolver_anchored_on_extraction_result(root_type: Option<&str>) -> FieldResolver {
    let field_types = HashMap::from([
        (
            "ExtractionResult".to_string(),
            HashMap::from([("results".to_string(), "ExtractedDocument".to_string())]),
        ),
        (
            "ExtractedDocument".to_string(),
            HashMap::from([("metadata".to_string(), "Metadata".to_string())]),
        ),
        (
            "Metadata".to_string(),
            HashMap::from([("format".to_string(), "FormatMetadata".to_string())]),
        ),
    ]);
    let json_bridged_by_type = HashMap::from([
        ("Metadata".to_string(), HashMap::from([("format".to_string(), true)])),
        (
            "ExtractedDocument".to_string(),
            HashMap::from([("metadata".to_string(), false)]),
        ),
    ]);
    let swift_first_class_map = SwiftFirstClassMap {
        field_types,
        // ~keep This is what reproduces the poisoning: `metadata` is bridged on some
        // unrelated type, so the flat set says "bridged" for every type that has a field
        // named `metadata`, including `ExtractedDocument`, where it is explicitly not.
        json_bridged_field_names: HashSet::from(["metadata".to_string()]),
        json_bridged_by_type,
        root_type: root_type.map(str::to_string),
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

/// Regression for the flat-set poisoning: `metadata` is bridged on an unrelated type, so the
/// old bare-name lookup stopped the walk at `metadata` on `ExtractedDocument` too, even though
/// `ExtractedDocument::metadata` is a real `Metadata` class with no `.toString()`. The
/// type-aware cursor must walk past `metadata` and stop only at `Metadata::format`, the segment
/// actually marked bridged for its own owner type.
///
/// Here the bridge anchor IS `format` itself (unlike the other tests in this file, where it is
/// `metadata`), so `excel` is the FIRST segment after the leaf and must still be recognized as
/// a variant name -- the skip is keyed on the literal preceding segment being `"format"`,
/// which this shape supplies via the leaf's own name rather than via an earlier `Key` step.
#[test]
fn should_anchor_json_bridge_on_owner_type_not_bare_field_name() {
    let resolver = resolver_anchored_on_extraction_result(Some("ExtractionResult"));

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.excel.sheet_count")
        .expect("format is bridged on Metadata and must be found");

    assert_eq!(leaf_field, "results[0].metadata.format");
    assert_eq!(steps, vec![JsonNavStep::Key("sheet_count".to_string())]);
}

/// CONTROL, pinning the documented fallback: with no `root_type` anywhere the per-segment
/// cursor can never anchor, so every segment falls back to the flat, name-keyed
/// `json_bridged_field_names` set exactly as before this fix — the walk stops at the bare
/// `metadata` segment even though the type-aware map (unreachable here) says otherwise.
#[test]
fn should_fall_back_to_the_flat_set_when_the_cursor_cannot_be_anchored() {
    let resolver = resolver_anchored_on_extraction_result(None);

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("results[0].metadata.format.excel.sheet_count")
        .expect("the flat set still marks metadata bridged when unanchored");

    assert_eq!(leaf_field, "results[0].metadata");
    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("format".to_string()),
            JsonNavStep::Key("sheet_count".to_string()),
        ]
    );
}
