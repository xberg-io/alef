mod collection;
mod enum_union;
mod swift_leaf;

use super::super::ir_collection::is_collection_path;
use super::super::leaf_anchor::LeafAnchor;
use super::super::parse::{
    normalize_indices_to_wildcards, normalize_numeric_indices, parse_path, strip_numeric_indices,
};
use super::super::types::{FieldResolver, PathSegment, StringyField};
use std::borrow::Cow;
use std::collections::HashSet;

impl FieldResolver {
    /// Returns `true` when `fixture_field` (or its resolved alias, or a
    /// normalised form) is configured as a display-as-text field.
    ///
    /// Accepts both the raw fixture field path and the alias-resolved path so
    /// callers don't need to resolve first.
    pub fn is_display_as_text(&self, fixture_field: &str) -> bool {
        if self.display_as_text_fields.is_empty() {
            return false;
        }
        if self.display_as_text_fields.contains(fixture_field) {
            return true;
        }
        let resolved = self.resolve(fixture_field);
        self.display_as_text_fields.contains(resolved)
    }

    /// Resolve a fixture field path to the actual struct path.
    /// Falls back to the field itself if no alias exists.
    pub fn resolve<'a>(&'a self, fixture_field: &'a str) -> &'a str {
        self.aliases
            .get(fixture_field)
            .map(String::as_str)
            .unwrap_or(fixture_field)
    }

    /// True when the leaf segment of `field` is a `Vec<T>` field on any IR type.
    ///
    /// Used by swift codegen to keep `.count` straight on method-call accessors
    /// (`result.output()` returns RustVec — `.count` works directly, no
    /// `.toString()` needed). The check is on the bare leaf name, so it is best-
    /// effort when distinct types share a field name with different kinds.
    pub fn leaf_is_vec_via_swift_map(&self, field: &str) -> bool {
        let leaf = field.split('.').next_back().unwrap_or(field);
        let leaf = leaf.split('[').next().unwrap_or(leaf);
        self.swift_first_class_map.is_vec_field_name(leaf)
    }

    /// The prefix of `field` that names a JSON-bridged Swift leaf which the path then steps
    /// *past*, if any.
    ///
    /// ~keep swift-bridge collapses a JSON-bridged field to one `RustString`, so the leaf has
    /// neither `.count` nor a subscript. Every way of stepping past it — a `length`/`count`/
    /// `size` suffix, an index, a wildcard, or a further field — is therefore equally
    /// unspellable, and keying the refusal on the traversal rather than on the trailing
    /// accessor's spelling is what makes those four cases one case. The guard this replaced
    /// matched only a trailing count suffix, so an indexed path slipped through and the
    /// generator emitted a broken assertion on the line directly above the correct
    /// "JSON-bridges it to RustString" skip comment it wrote for the count suffix on that same
    /// field — one field, two opposite verdicts, adjacent lines.
    ///
    /// Returns `None` when the path ends *at* the bridged leaf: the leaf itself is a readable
    /// `RustString`, so an `equals`/`contains`/`is_empty` assertion on it is fine.
    pub fn swift_json_bridged_traversal_prefix(&self, field: &str) -> Option<String> {
        self.swift_json_bridged_prefix(field, false)
    }

    /// The same fact as [`Self::swift_json_bridged_traversal_prefix`], asked by a caller that will
    /// step past `field`'s OWN leaf even though the path stops there — iterating it, or reading a
    /// member off each of its elements.
    ///
    /// ~keep A docs snippet's `iterate` operation spells `for item in <accessor>`, which needs
    /// elements the `RustString` does not have, so the impossibility is the traversal's, not the
    /// path's. Expressing it as one more caller of the shared walk — rather than a second
    /// `is_json_bridged_field_name` lookup at the call site — keeps the one predicate the binding
    /// generator answers (`field_needs_json_bridge`) as the only source of this verdict.
    pub fn swift_json_bridged_iteration_prefix(&self, field: &str) -> Option<String> {
        self.swift_json_bridged_prefix(field, true)
    }

    /// ~keep Resolves the alias BEFORE walking segments, and only that -- not `field` itself as
    /// a fallback. `field` is a virtual/authored label (e.g. `metadata.open_graph.title`); its
    /// dot-segments do not correspond to real struct hops, so walking it directly can find an
    /// incidental bare-name match at the WRONG depth. `metadata.open_graph.title`'s own bare
    /// segment `open_graph` happens to equal the real bridged field's name, so walking the raw
    /// label returned the prefix `metadata.open_graph` -- two hops -- when the real struct path
    /// is `metadata.document.open_graph[title]` -- three hops through an intermediate `document`
    /// struct the alias never mentions. That wrong, shorter prefix was then handed to
    /// `resolver.accessor()` as a real path by `presentation::clamp_swift_json_bridged_paths`,
    /// which has no alias entry for the truncated form, so it rendered `.metadata().openGraph()`
    /// -- a non-compiling accessor -- in the Swift snippet. The e2e generator's own use of this
    /// same wrong prefix (naming the field in a skip COMMENT, never as a real path) hid the bug:
    /// wrong text in a comment is silently wrong, not a compiler error. `resolve()` is an
    /// identity function whenever `field` names no alias, so unconditionally resolving first
    /// changes nothing for every caller that was already passing a real (unaliased) struct path.
    fn swift_json_bridged_prefix(&self, field: &str, steps_past_leaf: bool) -> Option<String> {
        self.swift_json_bridged_prefix_direct(self.resolve(field), steps_past_leaf)
    }

    fn swift_json_bridged_prefix_direct(&self, field: &str, steps_past_leaf: bool) -> Option<String> {
        let segments: Vec<&str> = field.split('.').collect();
        let last = segments.len().saturating_sub(1);
        let mut prefix: Vec<&str> = Vec::with_capacity(segments.len());
        for (index, segment) in segments.iter().enumerate() {
            let bare = segment.split('[').next().unwrap_or(segment);
            prefix.push(bare);
            let steps_past = index < last || segment.contains('[') || steps_past_leaf;
            if steps_past && self.swift_first_class_map.is_json_bridged_field_name(bare) {
                return Some(prefix.join("."));
            }
        }
        None
    }

    /// IR type backing the Swift result variable, if known. Used by
    /// `swift_build_accessor` to seed its per-segment type cursor.
    pub fn swift_root_type(&self) -> Option<&String> {
        self.swift_first_class_map.root_type.as_ref()
    }

    /// Whether fields on `type_name` should be accessed as Swift properties
    /// (first-class Codable struct → `public let`) vs swift-bridge method calls
    /// (typealias-to-opaque RustBridge class). Mirrors `SwiftFirstClassMap::is_first_class`.
    pub fn swift_is_first_class(&self, type_name: Option<&str>) -> bool {
        self.swift_first_class_map.is_first_class(type_name)
    }

    /// Advance the per-segment type cursor by one field name. Mirrors
    /// `SwiftFirstClassMap::advance`.
    pub fn swift_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.swift_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Swift
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn swift_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.swift_first_class_map.stringy_fields(type_name)
    }

    /// IR type backing the Dart result variable, if known.
    pub fn dart_root_type(&self) -> Option<&String> {
        self.dart_first_class_map.root_type.as_ref()
    }

    /// Advance the Dart type cursor through a field, returning the target type name.
    pub fn dart_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.dart_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Dart
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn dart_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.dart_first_class_map.stringy_fields(type_name)
    }

    /// Check if a resolved field path is optional.
    pub fn is_optional(&self, field: &str) -> bool {
        if self.is_optional_direct(field) {
            return true;
        }
        // Anchored IR answer: the leaf is optional on the exact type this path reaches from the
        // call's declared result type. Consulted only after the config-declared sets, so an
        // explicit `fields_optional` entry still wins, and it can only add a `true` — a `None`
        // root type (no IR, or an unresolvable call) makes it a no-op. ~keep
        if super::super::ir_result_fields::is_optional_path(&self.ir_result_field_map, self.resolve(field)) {
            return true;
        }
        // Namespace-prefix fallback: paths like `interaction.action_results[0].data`
        // strip the virtual `interaction.` prefix before consulting `optional_fields`,
        // matching the same convention used by `is_valid_for_result`.
        if let Some(suffix) = self.namespace_stripped_path(field)
            && self.is_optional_direct(suffix)
        {
            return true;
        }
        false
    }

    /// Whether the target binding emits this exact field as a pointer, when the anchored IR can
    /// resolve it. This is independent of Rust optionality: Go slices and sealed interfaces are
    /// nullable values, while unresolved required named fields use `*json.RawMessage`.
    pub fn target_field_is_pointer(&self, field: &str) -> Option<bool> {
        super::super::ir_result_fields::pointer_at_path(&self.ir_result_field_map, self.resolve(field))
    }

    pub fn target_field_is_data_interface(&self, field: &str) -> bool {
        super::super::ir_result_fields::data_interface_at_path(&self.ir_result_field_map, self.resolve(field))
            .unwrap_or(false)
    }

    /// Whether `field` resolves to a `Map<K, V>` field whose value type `V` is a plain,
    /// never-nil Go value kind (see `ir_result_fields::map_value_is_scalar_at_path`). `None`
    /// when the anchored IR cannot resolve the path — never `Some(false)` for "unknown", since
    /// callers use this to positively override an otherwise-nilable classification and must
    /// never do so on a guess.
    pub fn map_value_is_scalar(&self, field: &str) -> Option<bool> {
        super::super::ir_result_fields::map_value_is_scalar_at_path(&self.ir_result_field_map, self.resolve(field))
    }

    /// Check whether `field`'s resolved leaf type is one alef cannot vouch for as implementing
    /// `Display` — a struct/enum from the crate's own IR, per
    /// [`ir_result_fields::leaf_is_named_type`](super::super::ir_result_fields::leaf_is_named_type).
    ///
    /// `false` (never a warning) whenever the anchored result type is unresolved, matching the
    /// permissive fallback every other IR-backed check in this module uses for that state.
    pub fn is_display_unsafe(&self, field: &str) -> bool {
        super::super::ir_result_fields::leaf_is_named_type(&self.ir_result_field_map, self.resolve(field))
    }

    fn is_optional_direct(&self, field: &str) -> bool {
        Self::optional_set_contains(&self.optional_fields, &self.array_fields, field)
    }

    /// Whether `field` matches an entry the consumer's OWN `[e2e].fields_optional` list named —
    /// never a name `with_ir_fields` merged into `optional_fields` from the IR's own `Option<T>`
    /// declarations. See `config_declared_optional_fields`'s field doc for why
    /// `declaring_config_key` needs this distinct from [`Self::is_optional_direct`].
    fn is_config_declared_optional(&self, field: &str) -> bool {
        Self::optional_set_contains(&self.config_declared_optional_fields, &self.array_fields, field)
    }

    /// The index-normalization rules `fields_optional`-style path matching applies, shared by
    /// [`Self::is_optional_direct`] (classification, against the IR-merged set) and
    /// [`Self::is_config_declared_optional`] (provenance, against the unmerged config-only set)
    /// so the two can never drift on what counts as "the same path" while disagreeing only on
    /// which set they consult.
    fn optional_set_contains(set: &HashSet<String>, array_fields: &HashSet<String>, field: &str) -> bool {
        if set.contains(field) {
            return true;
        }
        let index_normalized = normalize_numeric_indices(field);
        if index_normalized != field && set.contains(index_normalized.as_str()) {
            return true;
        }
        // Also check with all numeric indices stripped: "choices[0].message.tool_calls"
        // should match optional_fields entry "choices.message.tool_calls".
        let de_indexed = strip_numeric_indices(field);
        if de_indexed != field && set.contains(de_indexed.as_str()) {
            return true;
        }
        let normalized = field.replace("[].", ".");
        if normalized != field && set.contains(normalized.as_str()) {
            return true;
        }
        for af in array_fields {
            if let Some(rest) = field.strip_prefix(af.as_str())
                && let Some(rest) = rest.strip_prefix('.')
            {
                let with_bracket = format!("{af}[].{rest}");
                if set.contains(with_bracket.as_str()) {
                    return true;
                }
            }
        }
        false
    }

    /// Check whether a single bare JSON key (not a dotted path) may be entirely absent from
    /// the wire format, per [`Self::with_wire_optional_fields`].
    ///
    /// Callers that walk a parsed JSON tree segment-by-segment (currently the Zig e2e
    /// generator) should consult this once per `.get(key)` step, not once for the whole
    /// resolved path: `wire_optional_fields` is IR-derived from bare field names, with no
    /// notion of nesting depth, so matching happens per key the same way the field was
    /// recorded — unlike [`Self::is_optional`], which matches config-declared, fully
    /// dotted paths.
    pub fn is_wire_optional_key(&self, key: &str) -> bool {
        self.wire_optional_fields.contains(key)
    }

    /// Check if a fixture field has an explicit alias mapping.
    pub fn has_alias(&self, fixture_field: &str) -> bool {
        self.aliases.contains_key(fixture_field)
    }

    /// Check whether `field_name` is configured as an explicit result field.
    ///
    /// Returns true only when the caller has populated `result_fields` AND the
    /// field name is present. Empty `result_fields` always returns false — use
    /// `is_valid_for_result` for the default-allow semantics.
    pub fn has_explicit_field(&self, field_name: &str) -> bool {
        if self.result_fields.is_empty() {
            return false;
        }
        self.result_fields.contains(field_name)
    }

    /// Whether the call's result is a fieldless value — a raw byte payload — so that no member
    /// path can name anything on it at all.
    ///
    /// ~keep The same stored fact [`Self::is_valid_for_result`] and
    /// [`Self::result_field_oracle_knows`] already refuse every non-empty path on, exposed as its
    /// own question for the callers that need to know *why* a path is unavailable rather than
    /// only *that* it is. A backend whose renderer reinterprets a field-bearing assertion as an
    /// assertion on the whole result (`result_is_simple`) deliberately does not let the
    /// availability oracle veto the path — but it must still not emit a member access, and this
    /// is the one fact that distinguishes the two cases. Asking here rather than re-deriving
    /// "does the return type have fields" per backend is what keeps a second answer from drifting
    /// away from the oracle's.
    pub fn result_has_no_fields(&self) -> bool {
        self.result_is_byte_payload
    }

    /// Check whether a fixture field path is valid for the configured result type.
    ///
    /// The IR is authoritative whenever it recognizes the resolved path's first segment
    /// as a real struct field name (populated via [`Self::with_ir_fields`]):
    /// reachable-through-the-binding wins regardless of `result_fields`, and
    /// known-excluded-from-the-binding loses regardless of `result_fields`. `result_fields`
    /// is a hand-maintained allowlist with no automatic connection to the real struct, and
    /// it can drift in BOTH directions at once — one shipped config was found with a field
    /// genuinely exposed via a real getter missing from `result_fields` (silently
    /// downgrading every assertion on it to a "not available" comment) *and*, in the same
    /// list, a field that carries `#[serde(skip)]` with no getter still listed as
    /// available (which would generate a passing-looking assertion against an attribute
    /// that doesn't exist at runtime). Neither direction is fixable by trusting
    /// `result_fields` harder or consulting more hand-maintained config — the IR is the
    /// only signal here that isn't itself hand-maintained per fixture. ~keep
    ///
    /// When the IR has never heard of the first segment at all — a virtual namespace
    /// prefix like `"browser."`, a streaming/synthetic pseudo-field, or simply because the
    /// codegen call site hasn't wired IR data in via `with_ir_fields` — this falls back to
    /// the config-only check: the resolved path's first segment is in `result_fields`, or
    /// the path uses a single virtual namespace prefix (e.g. `"browser."`, `"interaction."`)
    /// whose second segment IS in `result_fields`, or (last resort, see
    /// [`Self::is_known_via_sibling_field_config`]) another per-field config map already
    /// references the field even though `result_fields` doesn't.
    pub fn is_valid_for_result(&self, fixture_field: &str) -> bool {
        // A byte payload has no fields at all, so no per-field distinction is possible: every
        // non-empty path is rejected before any of the name-keyed or IR-anchored checks below get
        // a chance to default-allow it the way they do for a call with simply no IR wired in. See
        // `result_is_byte_payload`'s field doc for why this has to be a positive, call-specific
        // fact rather than inferred from an absent anchored root type. ~keep
        if self.result_is_byte_payload {
            return false;
        }
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);

        // IR oracle: only consulted for names the IR actually recognizes. A name it has
        // never seen (namespace prefixes, synthetic fields, or simply no IR data wired up)
        // falls through to the config-only checks below unaffected.
        if self.ir_reachable_fields.contains(first_segment) {
            return true;
        }
        if self.ir_known_excluded_fields.contains(first_segment) {
            return false;
        }

        if self.result_fields.is_empty() {
            return true;
        }
        if self.result_fields.contains(first_segment) {
            return true;
        }
        // Namespace-prefix fallback: if the first segment is NOT a known result field
        // but stripping it yields a path whose own first segment IS a known result
        // field, treat the path as valid.  This supports fixture field paths like
        // `"browser.browser_used"` where `"browser"` is a virtual grouping prefix
        // and the real field is `"browser_used"`.
        if let Some(suffix) = self.namespace_stripped_path(resolved) {
            let suffix_first = suffix.split('.').next().unwrap_or(suffix);
            let suffix_first = suffix_first.split('[').next().unwrap_or(suffix_first);
            if self.result_fields.contains(suffix_first) {
                return true;
            }
        }
        self.is_known_via_sibling_field_config(fixture_field, resolved)
    }

    /// Whether any *IR* was wired into this resolver, as opposed to config alone.
    ///
    /// [`Self::result_field_oracle_knows`] deliberately treats a non-empty `[e2e].result_fields`
    /// as an oracle in its own right, so with no IR present it answers `Some(false)` for every
    /// name that list omits. That is the right answer for a path this generator *derived* — the
    /// consumer's own config is the only statement of intent available. It is the wrong answer for
    /// a path a fixture author wrote by hand, because `result_fields` is an incomplete allow-list
    /// by construction and the author is entitled to name a virtual or namespaced path no config
    /// key lists. Callers validating authored input gate the refutation on this predicate so only
    /// real IR evidence can drop an entry. ~keep
    pub fn has_ir_result_evidence(&self) -> bool {
        !self.ir_reachable_fields.is_empty() || !self.ir_known_excluded_fields.is_empty()
    }

    /// Whether the availability oracle *positively recognizes* `fixture_field`'s first segment,
    /// as opposed to [`Self::is_valid_for_result`]'s deliberate default-allow answer for a name
    /// it has never heard of.
    ///
    /// The two answers must stay distinct because they serve opposite risks. An assertion is
    /// rendered against a hand-authored fixture path, so defaulting an unrecognized name to
    /// "valid" is right: virtual namespace prefixes, synthetic and streaming pseudo-fields all
    /// legitimately name things no struct declares, and skipping them would silently drop real
    /// coverage. A *derived* docs-snippet accessor has no such author — it is inferred from an
    /// assertion that may not even be about the result — so the same default emits a member
    /// access nothing declares. `crawl_stream`'s `rate_limit.min_duration_ms` is the shape:
    /// `rate_limit` is an assertion grouping, not a field, and the IR declares only
    /// `rate_limit_ms` elsewhere. ~keep
    ///
    /// * `Some(true)` — the IR reaches this field name through the binding, or the consumer
    ///   listed it in `result_fields` (directly or behind a virtual namespace prefix).
    /// * `Some(false)` — an oracle was available and did not recognize the name.
    /// * `None` — no oracle at all: no IR was wired in and `result_fields` is empty, so nothing
    ///   was consulted and nothing can be concluded. Mirrors `e2e::validate`'s
    ///   `IrFieldShape::IrAbsent`; callers must fall back to their pre-oracle behaviour rather
    ///   than treat silence as rejection, or every IR-less call site would reject everything.
    pub fn result_field_oracle_knows(&self, fixture_field: &str) -> Option<bool> {
        // Same positive byte-payload fact `is_valid_for_result` guards against, asked before any
        // of the flat sets below get a chance to answer `None` (unknown) for a name they simply
        // have never seen. `Some(false)` here — a definite refusal, not an abstention — is what
        // lets `anchor_leaf` fall through to `anchor_leaf_via_result_fields` and still correctly
        // find no compiling prefix. ~keep
        if self.result_is_byte_payload {
            return Some(false);
        }
        if self.ir_reachable_fields.is_empty()
            && self.ir_known_excluded_fields.is_empty()
            && self.result_fields.is_empty()
        {
            return None;
        }
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);
        // The anchored oracle answers first and last: when the call's own result type is known,
        // whether IT declares this member is the whole question, and a name reachable on some
        // other struct entirely is not evidence about this one. Asked of the WHOLE path, not just
        // its root: a deeper segment nobody declares is the same phantom member access as a root
        // one, and judging only the root is what let `result.document.document_structure` reach a
        // published snippet. Only reached when a root type resolved AND the walk stays inside the
        // struct graph this map carries; `None` from here falls through to the flat, name-keyed
        // answer below, which is what every IR-less call site still gets. ~keep
        if let Some(declared) = super::super::ir_result_fields::root_declares_path(&self.ir_result_field_map, resolved)
        {
            return Some(declared || self.namespace_prefix_reaches_a_declared_field(resolved));
        }
        // A `fields_method_calls` entry that names exactly how to cross a tagged-union boundary is
        // a real accessor, not a phantom one — every backend that renders one (gleam, dart, kotlin,
        // swift) does so via this same `tagged_union_split`/`union_variant_payload` pair. Consulted
        // BEFORE the blanket unwalkable-field refusal below, so a path extending past a
        // method-call-covered union resolves against the variant's own payload type instead of
        // being refused at the union itself. ~keep
        if let Some(declared) = self.tagged_union_method_call_declares(fixture_field) {
            return Some(declared);
        }
        // `root_declares_path` abstains (`None`) on a declared-but-unresolvable prefix segment on
        // purpose, so a map value or `serde_json::Value` still derives its accessor. A tagged-union
        // field is the same shape, but has a definite answer this map now carries: `accessor()`
        // cannot walk a plain field access past it either, so a path that tries reads as refused
        // here rather than falling through to the permissive flat check below -- UNLESS a
        // `fields_method_calls` entry above already vouched for the crossing. ~keep
        if super::super::ir_result_fields::path_crosses_unwalkable_field(&self.ir_result_field_map, resolved) {
            return Some(false);
        }
        if self.ir_known_excluded_fields.contains(first_segment) {
            return Some(false);
        }
        if self.ir_reachable_fields.contains(first_segment) || self.result_fields.contains(first_segment) {
            return Some(true);
        }
        // Same namespace-prefix rescue `is_valid_for_result` applies, so a path the consumer
        // deliberately spelled `browser.browser_used` is not rejected for its virtual prefix.
        if let Some(suffix) = self.namespace_stripped_path(resolved) {
            let suffix_first = suffix.split('.').next().unwrap_or(suffix);
            let suffix_first = suffix_first.split('[').next().unwrap_or(suffix_first);
            if self.ir_reachable_fields.contains(suffix_first) || self.result_fields.contains(suffix_first) {
                return Some(true);
            }
        }
        Some(false)
    }

    /// The namespace-prefix rescue, against the anchored result type: a path deliberately
    /// spelled `browser.browser_used` must not be rejected for its virtual first segment when
    /// the result type declares `browser_used`. Same rule
    /// [`Self::result_field_oracle_knows`] applies to the flat sets, asked of the one type the
    /// call actually returns.
    fn namespace_prefix_reaches_a_declared_field(&self, resolved: &str) -> bool {
        let Some(suffix) = self.namespace_stripped_path(resolved) else {
            return false;
        };
        let suffix_first = suffix.split('.').next().unwrap_or(suffix);
        let suffix_first = suffix_first.split('[').next().unwrap_or(suffix_first);
        super::super::ir_result_fields::root_declares_first_segment(&self.ir_result_field_map, suffix_first)
            == Some(true)
    }

    /// The `alef.toml` key through which the consumer declared `fixture_field`, if any.
    ///
    /// ~keep A path the availability oracle refuses is usually not a defect — assertion
    /// groupings, streaming pseudo-fields and virtual namespace prefixes all legitimately name
    /// something no struct declares, and dropping their derived accessor is the correct silent
    /// outcome. A path the consumer *wrote down in config* is different: it is a claim about the
    /// result type, made by hand, and a refused claim is drift only they can fix. Naming the key
    /// that carries it is what makes the difference reportable.
    ///
    /// `result_fields` lists top-level names only, so it answers for a single-segment path and
    /// never for a dotted one — otherwise every nested path under a listed root would read as
    /// consumer-declared when only its first segment was.
    pub fn declaring_config_key(&self, fixture_field: &str) -> Option<&'static str> {
        let resolved = self.resolve(fixture_field);
        if self.aliases.contains_key(fixture_field) {
            return Some("fields");
        }
        if self.method_calls.contains(resolved) {
            return Some("fields_method_calls");
        }
        if self.is_array(resolved) {
            return Some("fields_array");
        }
        if self.is_config_declared_optional(resolved) {
            return Some("fields_optional");
        }
        let is_single_segment = !resolved.contains('.') && !resolved.contains('[');
        (is_single_segment && self.result_fields.contains(resolved)).then_some("result_fields")
    }

    /// True when `fixture_field` (or its alias-resolved path) is referenced by one of
    /// the other per-field config maps (`fields`, `fields_optional`, `fields_array`,
    /// `fields_method_calls`) even though it is absent from `result_fields`.
    ///
    /// Last-resort fallback for codegen call sites that haven't wired IR data in via
    /// `with_ir_fields` (`is_valid_for_result` only reaches this once the IR has had, and
    /// declined, the chance to answer). These maps only make sense to populate for a field
    /// that genuinely exists on the result type — an alias target, an optionality flag, an
    /// array marker, or a method-call accessor all require the config author to have
    /// looked at the real struct. A field that is truly unavailable (no getter generated
    /// for it at all) has nothing to configure here, so this check does not make
    /// unavailable fields pass — it only rescues fields the config demonstrably already
    /// knows about. ~keep
    fn is_known_via_sibling_field_config(&self, fixture_field: &str, resolved: &str) -> bool {
        self.aliases.contains_key(fixture_field)
            || self.is_optional_direct(resolved)
            || self.is_array(resolved)
            || self.method_calls.contains(resolved)
    }

    /// If `path`'s first dot-separated segment is NOT in `result_fields` and
    /// contains no `[…]` indexing (i.e. it looks like a pure namespace label),
    /// return the remainder of the path after that first segment.  Returns `None`
    /// when the first segment already matches a result field or when stripping it
    /// would leave an empty string.
    pub fn namespace_stripped_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        // When the consumer hasn't configured `result_fields`, there is no way
        // to tell a virtual namespace prefix (e.g. `interaction.action_results`)
        // from a real nested-struct field path (e.g. `metrics.total_lines`).
        // Defaulting to "strip" was lossy — every dotted field path was reduced
        // to its leaf segment, so backends (notably the C e2e codegen) emitted
        // accessors against the wrong parent type. Opt the stripping in only
        // when the consumer explicitly listed the top-level result fields.
        if self.result_fields.is_empty() {
            return None;
        }
        let dot_pos = path.find('.')?;
        let first = &path[..dot_pos];
        // Only strip if the first segment contains no brackets (i.e. is a bare
        // label, not an array access like `pages[0]`).
        if first.contains('[') {
            return None;
        }
        // Only strip if the first segment is NOT itself a known result field —
        // real fields should never be treated as namespace prefixes.
        if self.result_fields.contains(first) {
            return None;
        }
        // ~keep `result_fields` is hand-maintained, so it under-reports: a consumer who listed a
        // nested leaf without also listing its parent made the parent look like a virtual
        // namespace, and the parent segment was silently dropped — turning a real nested step into
        // an accessor on the wrong receiver, against a field the result type does not declare. The
        // IR already knows which names are real fields of the call's own result type, so ask it
        // rather than infer absence from a config omission.
        if self.ir_declares_struct_field_on_root(first) {
            return None;
        }
        let suffix = &path[dot_pos + 1..];
        if suffix.is_empty() { None } else { Some(suffix) }
    }

    /// Resolve a raw fixture field path to the path the value actually occupies on
    /// the result, with any virtual namespace prefix removed.
    ///
    /// ~keep Fixtures group assertions under virtual labels (`batch.completed_count`)
    /// that have no counterpart in the emitted result — the value sits at
    /// `completed_count`. Backends that navigate the serialized result by path
    /// (C's accessor chain, brew's jq expression, zig's `std.json.Value` lookup) must
    /// strip that label or they address a member that does not exist. Stripping is
    /// conditional: the remainder's own first segment has to be a real result field,
    /// so a genuinely nested path (`metrics.total_lines`) keeps its prefix.
    ///
    /// ~keep The single definition of where a fixture field's value lives — host-language
    /// accessors (`accessor`, `rust_unwrap_binding`), serialized-path navigation (zig, brew, C)
    /// and shape classification (`is_array`) all read it. Each of those used to re-derive it, and
    /// two of the copies were gated on `result_fields.contains(..)` rather than
    /// `is_valid_for_result(..)`, so they could place the same field somewhere else. Add callers
    /// here; do not add a fourth copy.
    ///
    /// ~keep The envelope projection is asked FIRST, and it is asked of
    /// [`Self::anchor_leaf`] rather than re-derived here. On an envelope root the strip rule below
    /// cannot tell a real nested hop from a virtual label — `ir_declares_struct_field_on_root`
    /// only ever inspects the root's OWN fields, so a genuine `metadata.output_format` reached
    /// through a `result_fields` projection looked exactly like a grouping label and lost the
    /// `metadata` hop entirely. `anchor_leaf` already answers "which `result_fields` prefix reaches
    /// this" for the synthetic handlers; the generic path asking it, instead of growing a third
    /// copy of the prefix search, is what keeps the two from disagreeing about one fixture field.
    pub fn result_relative_path<'a>(&'a self, fixture_field: &'a str) -> Cow<'a, str> {
        let resolved = self.resolve(fixture_field);
        if let Some(projected) = self.envelope_projected_path(resolved) {
            return Cow::Owned(projected);
        }
        let Some(stripped) = self.namespace_stripped_path(resolved) else {
            return Cow::Borrowed(resolved);
        };
        let stripped_first = stripped.split('.').next().unwrap_or(stripped);
        let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
        if self.is_valid_for_result(stripped_first) {
            Cow::Borrowed(stripped)
        } else {
            Cow::Borrowed(resolved)
        }
    }

    /// Where `resolved` sits once the call's own envelope projection is accounted for, or `None`
    /// when no projection applies and the caller's existing strip rule should decide.
    ///
    /// ~keep The `root_declares_path == Some(true)` confirmation is load-bearing, not belt-and-
    /// braces. [`Self::anchor_leaf`] accepts whatever [`Self::result_field_oracle_knows`] accepts,
    /// and that oracle falls back to the flat, name-keyed sets whenever the IR walk declines to
    /// answer — so on a config-only resolver (no anchored root at all, which is every fixture
    /// suite whose result type never resolved) ANY `result_fields` entry would read as a
    /// confirming prefix and relocate every path in the suite. Demanding that the IR positively
    /// declare the whole prefixed path keeps this additive: it can only move a path the crate's
    /// own type graph proves is there.
    fn envelope_projected_path(&self, resolved: &str) -> Option<String> {
        let LeafAnchor::Prefixed(prefix) = self.anchor_leaf(resolved)? else {
            return None;
        };
        let candidate = format!("{prefix}.{resolved}");
        let declared = super::super::ir_result_fields::root_declares_path(&self.ir_result_field_map, &candidate);
        (declared == Some(true)).then_some(candidate)
    }

    /// Whether the IR positively declares `field_name` as a struct-typed field of the call's
    /// declared result type.
    ///
    /// ~keep Reads the roots the enum and collection maps already anchored via
    /// `resolve_declared_result_type`, so it needs no new wiring and answers `false` whenever no
    /// IR was supplied — which leaves the pre-existing config-only behaviour intact. Only
    /// struct-typed fields are recorded in `field_types`, which is exactly the set a dotted path
    /// can legitimately continue through.
    fn ir_declares_struct_field_on_root(&self, field_name: &str) -> bool {
        let roots = [
            (
                self.ir_collection_map.root_type.as_deref(),
                &self.ir_collection_map.field_types,
            ),
            (self.ir_enum_map.root_type.as_deref(), &self.ir_enum_map.field_types),
        ];
        roots.into_iter().any(|(root, field_types)| {
            root.is_some_and(|root| field_types.get(root).is_some_and(|f| f.contains_key(field_name)))
        })
    }

    /// Check if a field path is an array/Vec type, per the `fields_array` config — falling back
    /// to the IR-anchored answer ([`is_collection_path`]) when the config is silent.
    ///
    /// Accepts the raw fixture spelling as well as an already-resolved path: the second lookup
    /// asks [`Self::result_relative_path`] where the value actually sits — alias applied, virtual
    /// namespace prefix stripped — and classifies *that*.
    ///
    /// ~keep Asking rather than re-deriving is the point. `accessor()` strips a virtual grouping
    /// label (`interaction.action_results` addresses `action_results`), so a bare
    /// `array_fields.contains(field)` answered "not an array" about the very slice the accessor
    /// had just emitted; Go's `contains` renderer turned that disagreement into
    /// `string(result.ActionResults)`, which is not a legal conversion for a `[]T` and fails the
    /// generated package's build. Routing through `result_relative_path` — already the shared
    /// answer for the zig, brew and C generators — keeps one definition of where the value lives
    /// instead of growing a second hand-rolled copy of the fallback beside `is_optional`'s.
    ///
    /// Recursion is bounded: `result_relative_path` consults `is_valid_for_result` with a single
    /// dot-free segment, whose `namespace_stripped_path` returns `None` immediately, so the
    /// re-entry through `is_known_via_sibling_field_config` terminates one level down.
    ///
    /// ~keep `is_optional` already falls back to the IR (`ir_result_fields::is_optional_path`)
    /// when `fields_optional` is silent; this method never did, so a field whose `Vec`-ness is
    /// known ONLY through the IR (no per-element path anywhere in the fixture suite ever
    /// populated `fields_array`) read as scalar. An `Option<Vec<T>>` field in exactly that state
    /// — `is_optional` true, `is_array` wrongly false — is what every caller that branches on
    /// `is_optional(..) && is_array(..)` (the `Option<Vec<T>>` unwrap-before-`.len()` arm in
    /// `assertion_helpers.rs`, Go's slice-vs-pointer deref choice in `go/assertions.rs`) takes as
    /// "optional scalar", emitting a bare `.len()`/`*field` against the still-wrapped `Option`.
    /// `is_collection_path` is the exact oracle `is_collection_root` already asks for the
    /// sibling "is this field a collection AT ALL" question; several backends (dart, kotlin,
    /// csharp, swift, and one call site in this crate's own `rust/assertions.rs`) already OR
    /// `is_array(..) || is_collection_root(..)` at the call site to route around this gap —
    /// duplicating the fallback per call site instead of fixing the shared oracle, so any call
    /// site that never learned the workaround (every helper in `rust/assertion_helpers.rs`, all
    /// of `go/assertions.rs` and `go/test_function.rs`) still misclassifies. Asking here once
    /// removes the need for that workaround everywhere.
    pub fn is_array(&self, field: &str) -> bool {
        if self.array_fields.contains(field) {
            return true;
        }
        let relative = self.result_relative_path(field);
        if relative != field && self.array_fields.contains(relative.as_ref()) {
            return true;
        }
        if is_collection_path(&self.ir_collection_map, self.resolve(field)) {
            return true;
        }
        is_collection_path(&self.ir_collection_map, relative.as_ref())
    }

    /// Check whether `field` (a raw or already-resolved fixture path) is
    /// configured as a `fields_json_scalar` entry — i.e. its Kotlin type is
    /// an untyped JSON scalar (`Any?`, from `Option<serde_json::Value>`)
    /// rather than `Option<String>`, so `.orEmpty()` is undefined on it.
    ///
    /// Consults `json_scalar_fields` (a per-call resolved set, not stored on
    /// the resolver) against every spelling `fields_optional`/`is_optional`
    /// already treats as interchangeable — bracket-wildcard (`a[].b`) and
    /// fully de-indexed (`a.b`) — and, mirroring `is_optional`'s namespace
    /// fallback, against the path with a virtual grouping prefix (e.g.
    /// `interaction.`) stripped. Fixture field paths like
    /// `interaction.action_results[0].data` resolve to the struct path
    /// `action_results[0].data` for accessor generation via
    /// `namespace_stripped_path`; the same stripped path must be consulted
    /// here so `fields_json_scalar` entries configured against the struct
    /// path (not the virtual fixture namespace) are honored.
    pub fn is_json_scalar(&self, field: &str, json_scalar_fields: &HashSet<String>) -> bool {
        if Self::matches_json_scalar_spelling(field, json_scalar_fields) {
            return true;
        }
        let resolved = self.resolve(field);
        if resolved != field && Self::matches_json_scalar_spelling(resolved, json_scalar_fields) {
            return true;
        }
        self.namespace_stripped_path(resolved)
            .is_some_and(|stripped| Self::matches_json_scalar_spelling(stripped, json_scalar_fields))
    }

    fn matches_json_scalar_spelling(path: &str, json_scalar_fields: &HashSet<String>) -> bool {
        if json_scalar_fields.contains(path) {
            return true;
        }
        let normalized = normalize_indices_to_wildcards(path);
        if normalized != path && json_scalar_fields.contains(normalized.as_str()) {
            return true;
        }
        let de_indexed = strip_numeric_indices(path);
        de_indexed != path && json_scalar_fields.contains(de_indexed.as_str())
    }

    /// Whether `field_name` is a binding-visible member of the IR type named `type_name`,
    /// regardless of which type the call's own root is anchored at.
    ///
    /// Reads [`super::super::types::IrResultFieldMap::declared_fields`] directly rather than
    /// walking from `root_type`: that map already records every crate type's declared fields
    /// (`build_ir_result_field_map` iterates the whole `type_defs` list), so a type reached only
    /// by traversal — an `Iterate`'s loop-item type, never the call's own result type — still has
    /// an answer here. `None` when the map has no entry for `type_name` at all (an opaque type,
    /// or IR data was never wired in); callers must fall back to their pre-oracle behaviour for
    /// that case rather than treat silence as rejection.
    pub fn is_declared_field_of_type(&self, type_name: &str, field_name: &str) -> Option<bool> {
        self.ir_result_field_map
            .declared_fields
            .get(type_name)
            .map(|fields| fields.contains(field_name))
    }

    /// Check if a resolved field path traverses a tagged-union variant.
    ///
    /// Returns `Some((prefix, variant, suffix))` where:
    /// - `prefix` is the path up to (but not including) the tagged-union field
    ///   (e.g., `"metadata.format"`)
    /// - `variant` is the tagged-union accessor segment
    ///   (e.g., `"excel"`)
    /// - `suffix` is the remaining path after the variant
    ///   (e.g., `"sheet_count"`)
    ///
    /// Returns `None` if no tagged-union segment exists in the path.
    ///
    /// See `resolver::tagged_union_crossing` for the underlying scan
    /// ([`Self::find_crossing`]) and for [`Self::tagged_union_method_call_declares`], which walks
    /// a whole CHAIN of these -- split into its own module because a single crossing's payload
    /// type can itself declare another crossing, and that recursive walk earns its own concern
    /// rather than growing this one past the file's line budget.
    pub fn tagged_union_split(&self, fixture_field: &str) -> Option<(String, String, String)> {
        let resolved = self.resolve(fixture_field);
        self.find_crossing("", resolved)
    }

    /// Split a bracket-wildcard path (`foo[].bar`) into its array-root path and
    /// element sub-path, or `None` when the path has no wildcard.
    ///
    /// A wildcard means "every element", so callers render an any-element
    /// construct over the array root rather than an accessor into one index.
    /// Build the element side with `accessor(&element, lang, "<lambda param>")`
    /// — passing the closure parameter as the result var is what lets a nested
    /// element sub-path resolve against the loop variable instead of the result.
    ///
    /// Alias resolution happens BEFORE the split, so a renamed sub-field lands on
    /// the element side; the raw split is only a fallback for when resolution drops
    /// the marker. Explicit numeric indices (`choices[0].message`) return `None` and
    /// keep their existing index-preserving path through `accessor`. ~keep
    ///
    /// The split is NOT recursive: it consumes the FIRST `[].` only. A doubly-nested path
    /// (`pages[].links[].url`) therefore returns an element sub-path that still carries a
    /// wildcard, and handing that to `accessor` lowers the inner `[]` to index 0 (see
    /// `parse_path`) — the caller's loop covers `pages` while the assertion inside it silently
    /// reads `links[0]`. Gate the element sub-path with
    /// `crate::e2e::codegen::field_skip::nested_wildcard_skip_line` before building an
    /// accessor from it. ~keep
    pub fn wildcard_split(&self, fixture_field: &str) -> Option<(String, String)> {
        let raw_dot = fixture_field.find("[].")?;
        let resolved = self.resolve(fixture_field);
        match resolved.find("[].") {
            Some(dot) => Some((resolved[..dot].to_string(), resolved[dot + 3..].to_string())),
            None => Some((
                fixture_field[..raw_dot].to_string(),
                fixture_field[raw_dot + 3..].to_string(),
            )),
        }
    }

    /// Check if a resolved field path contains a non-numeric map access.
    pub fn has_map_access(&self, fixture_field: &str) -> bool {
        let resolved = self.resolve(fixture_field);
        let segments = parse_path(resolved);
        segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        })
    }
}
