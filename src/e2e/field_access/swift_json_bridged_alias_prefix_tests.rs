//! Regression coverage for `FieldResolver::swift_json_bridged_prefix` (via its two public
//! callers) when the fixture field is a virtual/authored alias whose own dot-segments do not
//! correspond to real struct hops.
//!
//! `metadata.open_graph.title` is a fixture-authored label that resolves to the real struct path
//! `metadata.document.open_graph[title]` — three hops through an intermediate `document` struct
//! the alias label never mentions, ending in a map-key subscript. Walking the RAW alias label
//! instead of the resolved path finds an incidental bare-name match at the wrong depth: the
//! alias's own second segment happens to be spelled `open_graph`, so a walk of the unresolved
//! label stops after two hops (`metadata.open_graph`) rather than three
//! (`metadata.document.open_graph`), silently dropping the `document` hop and losing the map-key
//! subscript both.

use super::*;
use std::collections::{HashMap, HashSet};

fn resolver_with_document_hop_alias() -> FieldResolver {
    let mut fields = HashMap::new();
    fields.insert(
        "metadata.open_graph.title".to_string(),
        "metadata.document.open_graph[title]".to_string(),
    );
    let mut json_bridged_field_names = HashSet::new();
    json_bridged_field_names.insert("open_graph".to_string());
    let swift_first_class_map = SwiftFirstClassMap {
        json_bridged_field_names,
        ..SwiftFirstClassMap::default()
    };
    FieldResolver::new_with_swift_first_class(
        &fields,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        swift_first_class_map,
    )
}

/// The traversal-prefix caller (used to write the e2e generator's skip comment, and to clamp a
/// `show` operation in the snippet generator) must resolve the alias BEFORE walking segments, so
/// it reports the real three-hop struct prefix — including the `document` hop the alias label
/// never spells — not the two-hop prefix an incidental bare-name match on the raw label finds.
#[test]
fn traversal_prefix_keeps_the_document_hop_and_the_map_key_subscript() {
    let resolver = resolver_with_document_hop_alias();

    let prefix = resolver.swift_json_bridged_traversal_prefix("metadata.open_graph.title");

    assert_eq!(
        prefix.as_deref(),
        Some("metadata.document.open_graph"),
        "must resolve the alias first and report the real struct prefix through `document`, not \
         an incidental match on the unresolved alias label"
    );
}

/// Same fact via the iteration-prefix caller, which additionally treats the field's OWN leaf as
/// a step-past (an `iterate` operation reads elements off the leaf).
#[test]
fn iteration_prefix_keeps_the_document_hop_and_the_map_key_subscript() {
    let resolver = resolver_with_document_hop_alias();

    let prefix = resolver.swift_json_bridged_iteration_prefix("metadata.open_graph.title");

    assert_eq!(
        prefix.as_deref(),
        Some("metadata.document.open_graph"),
        "must resolve the alias first and report the real struct prefix through `document`, not \
         an incidental match on the unresolved alias label"
    );
}

/// Negative control: a caller that already passes the real (unaliased) struct path must see no
/// change in behaviour — `resolve()` is an identity function whenever the path names no alias.
#[test]
fn traversal_prefix_is_unaffected_for_an_already_resolved_path() {
    let resolver = resolver_with_document_hop_alias();

    let prefix = resolver.swift_json_bridged_traversal_prefix("metadata.document.open_graph[title]");

    assert_eq!(prefix.as_deref(), Some("metadata.document.open_graph"));
}

#[test]
fn bridged_prefix_preserves_indices_before_the_bridged_leaf() {
    let resolver = resolver_with_document_hop_alias();
    for path in [
        "items[2].metadata.open_graph[title]",
        "items[2].metadata.open_graph.title",
    ] {
        assert_eq!(
            resolver.swift_json_bridged_traversal_prefix(path).as_deref(),
            Some("items[2].metadata.open_graph"),
            "only the JSON-bridged leaf loses its subscript: {path}"
        );
    }
    assert_eq!(
        resolver
            .swift_json_bridged_iteration_prefix("items[2].metadata.open_graph")
            .as_deref(),
        Some("items[2].metadata.open_graph")
    );
}
