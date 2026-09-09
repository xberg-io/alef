//! The Swift leaf-fact walk: what one getter on the leaf's REAL owner type looks like.
//!
//! Split out of `classify.rs` at the concept boundary. Each function here answers a
//! question about ONE getter by walking the Swift type cursor to that leaf's owner,
//! rather than by the bare-leaf-name lookup the rest of that module performs -- the
//! distinction the owner-type fix turns on. Keeping them together is what makes the
//! shared `swift_leaf_fact` cursor visibly shared. ~keep

use super::super::super::types::{FieldResolver, SwiftFirstClassMap};

impl FieldResolver {
    /// True when the leaf segment of `field` is a JSON-bridged Swift leaf on any IR type — the
    /// binding generator collapsed it to a single `RustString` holding the whole field
    /// JSON-encoded, per `SwiftFirstClassMap::json_bridged_field_names`.
    ///
    /// ~keep A POSITIVE fact, not the complement of [`Self::leaf_is_vec_via_swift_map`]: that
    /// complement also contains every genuine scalar and every field the map has no data for at
    /// all (an empty/never-scanned `SwiftFirstClassMap`), and neither of those is a JSON bridge.
    /// `swift_count_target` used to treat "not a recorded vec field name" as "must be a bridged
    /// scalar, wrap `.toString()`", which is exactly the confusion `json_bridged_field_names`'s
    /// own doc comment warns against — a field the IR proves is a genuine `Vec<T>` but the Swift
    /// map simply never recorded (an empty map, or a field reached only through an opaque owner
    /// type the map never scanned) got the same `.toString()` treatment as an actually-bridged
    /// scalar, silently counting the CHARACTERS of a debug string instead of the Vec's elements.
    ///
    /// ~keep Answers from [`Self::swift_leaf_is_json_bridged`]'s per-owner walk whenever the IR
    /// anchors the path, and only falls back to the flat bare-leaf-name set when it does not.
    /// The flat set is indexed over EVERY `TypeDef` in the crate, so one type declaring
    /// `items: Option<Vec<T>>` — which swift-bridge JSON-bridges — marked the name `items`
    /// bridged for a sibling type whose `items: Vec<T>` is a genuine `RustVec`, and the
    /// `not_empty`/`is_empty` arms then dropped that real collection out of `field_is_array`
    /// into the presence-only branch. Same reasoning as `ir_enum`'s: the classification has to
    /// be type-driven, and a bare-name rule misclassifies one of the two owners whichever way it
    /// defaults.
    pub fn leaf_is_json_bridged_via_swift_map(&self, field: &str) -> bool {
        if let Some(answer) = self.swift_leaf_is_json_bridged(field) {
            return answer;
        }
        let leaf = field.split('.').next_back().unwrap_or(field);
        let leaf = leaf.split('[').next().unwrap_or(leaf);
        self.swift_first_class_map.is_json_bridged_field_name(leaf)
    }

    /// Whether the swift-bridge getter for `field`'s LAST segment collapses the whole field to
    /// one JSON-encoded `RustString`, resolved against the leaf's REAL owner type.
    ///
    /// The `getter_optionality` sibling of [`Self::swift_leaf_getter_is_optional`], sharing its
    /// type cursor and its anchor precedence, and carrying the same contract: `None` means the IR
    /// did not describe the leaf, and callers must keep their existing behaviour rather than
    /// assume either answer.
    pub fn swift_leaf_is_json_bridged(&self, field: &str) -> Option<bool> {
        self.swift_leaf_fact(field, |map, owner, leaf, opaque| {
            if opaque {
                map.json_bridged_getter(owner, leaf)
            } else {
                Some(false)
            }
        })
    }

    /// Walk the Swift type cursor to `field`'s leaf owner and ask `fact` about that one getter.
    ///
    /// ~keep Seeded from `ir_collection_map`'s root, not the Swift map's own: the latter is the
    /// `result_type` override / `result_fields` heuristic, which is `None` whenever a consumer's
    /// config never named the type, while `ir_collection_map.root_type` is
    /// `resolve_declared_result_type`'s answer from the call's own signature and is the anchor
    /// the enum and collection maps already share.
    pub(super) fn swift_leaf_fact(
        &self,
        field: &str,
        fact: impl Fn(&SwiftFirstClassMap, &str, &str, bool) -> Option<bool>,
    ) -> Option<bool> {
        let map = &self.swift_first_class_map;
        let resolved = self.resolve(field);
        let segments: Vec<&str> = resolved.split('.').filter(|s| !s.is_empty()).collect();
        let last = segments.len().checked_sub(1)?;
        let mut current = self
            .ir_collection_map
            .root_type
            .clone()
            .or_else(|| self.ir_enum_map.root_type.clone())
            .or_else(|| map.root_type.clone())?;
        // ~keep A raw bridge getter never converts its child into the separately emitted Codable DTO.
        let mut opaque = false;
        for (index, segment) in segments.iter().enumerate() {
            opaque |= !map.is_first_class(Some(&current));
            let bare = segment.split('[').next().unwrap_or(segment);
            if index == last && !segment.contains('[') {
                return fact(map, &current, bare, opaque);
            }
            current = map.advance(Some(&current), bare)?;
        }
        None
    }
    /// Whether the swift-bridge getter for `field`'s LAST segment returns `Option<..>`, so that a
    /// caller chaining onto the rendered accessor must write `?.` rather than `.`.
    ///
    /// ~keep Distinct from [`Self::is_optional`], which answers the broader "is this path
    /// possibly-absent" from config plus IR and is keyed by bare path. This walks the Swift type
    /// cursor to the leaf's actual owner and reports what that one getter's declared return type
    /// is. `None` means the IR did not describe the leaf, in which case callers must keep their
    /// existing behaviour rather than assume either answer.
    pub fn swift_leaf_getter_is_optional(&self, field: &str) -> Option<bool> {
        self.swift_leaf_fact(field, |map, owner, leaf, opaque| {
            if opaque {
                map.getter_is_optional(owner, leaf)
            } else {
                Some(self.is_optional(field))
            }
        })
    }
}
