//! IR-derived enum and tagged-union variant classification.
//!
//! Split out of `classify.rs` at the concept boundary -- this is every classify method whose
//! answer comes from `ir_enum`: whether a field is enum-typed at all
//! ([`FieldResolver::is_enum`]), which concrete enum backs it
//! ([`FieldResolver::ir_enum_type_name`]), what a tagged-union variant's single payload field
//! is and whether that payload is itself a collection
//! ([`FieldResolver::union_variant_payload`], [`FieldResolver::union_variant_payload_is_collection`]),
//! its wire discriminator ([`FieldResolver::tagged_enum_wire_discriminator`]), whether that enum
//! carries variant data at all ([`FieldResolver::ir_enum_is_data_carrying`]), and two
//! per-backend representation facts derived from the same enum-type lookup
//! ([`FieldResolver::java_enum_emits_get_value`], [`FieldResolver::ruby_enum_serialized_as_hash`]).
//! Keeping them together is what makes the shared `ir_enum_type_name` dependency visible: every
//! method below either walks `ir_enum` directly or calls another method in this same file that
//! does. ~keep

use super::super::super::ir_enum::{enum_type_at_path, is_enum_path};
use super::super::super::types::FieldResolver;
use super::super::super::types::WasmEnumRepresentation;

impl FieldResolver {
    pub(crate) fn wasm_enum_representation(&self, field: &str) -> Option<WasmEnumRepresentation<'_>> {
        let enum_name = self.ir_enum_type_name(field)?;
        if self.wasm_untagged_enum_names.contains(&enum_name) {
            return Some(WasmEnumRepresentation::Untagged);
        }
        match self.ir_enum_map.tagged_enum_wire.get(&enum_name) {
            Some(wire) => Some(WasmEnumRepresentation::Tagged { tag: &wire.tag }),
            None => Some(WasmEnumRepresentation::External),
        }
    }
    /// Check whether `field` is enum-typed: an explicit `fields_enum` config entry (exact or
    /// alias-resolved) always wins, and — when the config is silent — the IR-derived
    /// classification (`with_ir_enum_map`) gets the final say. See `ir_enum` module docs for
    /// why the IR check has to walk the whole path rather than matching on the leaf name
    /// alone.
    pub fn is_enum(&self, field: &str) -> bool {
        let resolved = self.resolve(field);
        if self.enum_fields.contains(field) || self.enum_fields.contains(resolved) {
            return true;
        }
        is_enum_path(&self.ir_enum_map, resolved)
    }

    /// Resolve the concrete IR enum type name backing `field`'s leaf segment, when the IR
    /// walk (see `ir_enum` module docs) positively confirms it. `None` covers both "the IR
    /// doesn't know" (unresolved root type, config-only classification) and "not an enum
    /// field" — callers that need to distinguish those must call `is_enum` first.
    ///
    /// Used by backends whose emitted accessor for an enum-typed field depends on which
    /// concrete Rust representation that specific enum has (e.g. Java's plain-enum-with-
    /// `getValue()` vs. tagged/untagged-union-wrapper split), not just "is this field an
    /// enum" in the abstract.
    pub fn ir_enum_type_name(&self, field: &str) -> Option<String> {
        let resolved = self.resolve(field);
        enum_type_at_path(&self.ir_enum_map, resolved)
    }

    /// The single field a tagged-union variant carries, as `(field_name, payload_type_name)`,
    /// per [`super::super::super::ir_enum::build_ir_enum_map`]'s `variant_payload_types`.
    ///
    /// Meant to be called with the union type name [`Self::ir_enum_type_name`] resolves for a
    /// [`Self::tagged_union_split`] prefix, and the variant segment that split returned, so a
    /// caller can keep walking a fixture path's suffix through the variant's own payload type
    /// once it has narrowed to that variant — the same shape `metadata.format.excel.sheet_count`
    /// needs after splitting into `("metadata.format", "excel", "sheet_count")`: this answers
    /// which type `sheet_count` continues into. Returns `None` for a variant with zero fields
    /// (nothing to narrow into) or more than one (no single payload type), or when the IR never
    /// described the union type at all.
    pub fn union_variant_payload(&self, union_type: &str, variant: &str) -> Option<(&str, &str)> {
        self.ir_enum_map
            .variant_payload_types
            .get(union_type)?
            .get(variant)
            .map(|(field_name, type_name)| (field_name.as_str(), type_name.as_str()))
    }

    /// Whether `variant`'s single payload field (per [`Self::union_variant_payload`]) is itself
    /// `Vec`-typed (`Variant(Vec<Item>)`) rather than a struct that merely wraps a collection
    /// field (`Variant(Payload)`, where `Payload.items: Vec<Item>`).
    ///
    /// A fixture path that names only the union field and the variant, with no field inside the
    /// payload (e.g. `outcome.found`, split by [`Self::ir_tagged_union_split`] into a prefix,
    /// `union_type`, `variant`, and an EMPTY suffix), is asserting against the payload value
    /// itself. [`Self::union_variant_field_is_collection`] cannot answer that: it requires a
    /// non-empty field name to walk the payload type's own fields, and correctly answers `false`
    /// for an empty one. This is the distinct question a caller must ask instead once it finds
    /// the suffix is empty — see `csharp`/`kotlin`'s `try_render_generic_union_assertion`. ~keep
    pub fn union_variant_payload_is_collection(&self, union_type: &str, variant: &str) -> bool {
        self.ir_enum_map
            .variant_payload_is_collection
            .get(union_type)
            .is_some_and(|variants| variants.contains(variant))
    }

    /// The Rust variant identifier that a serde `wire` value names, for the enum type backing
    /// `field` — and only when a rename actually separates the two spellings (see
    /// [`super::super::super::types::IrEnumMap::enum_wire_variants`] for the exclusions).
    ///
    /// A generator that renders an enum value on the RUST surface (`format!("{:?}", ..)`, which
    /// is all `Debug` guarantees) but compares it against a fixture's WIRE value needs this to
    /// bring the two onto one surface. `None` means either "the IR cannot resolve this field to
    /// a concrete enum" or "no rename is in effect", and every caller must treat both the same
    /// way: keep the fixture value untranslated, which is the behaviour that predates this
    /// lookup and is correct whenever the identifier IS the wire value.
    pub fn enum_variant_for_wire_value(&self, field: &str, wire: &str) -> Option<&str> {
        let enum_name = self.ir_enum_type_name(field)?;
        self.ir_enum_map
            .enum_wire_variants
            .get(&enum_name)?
            .get(wire)
            .map(String::as_str)
    }

    /// The serde WIRE value a Rust `variant` identifier produces, for the enum type backing
    /// `field` — the inverse direction of [`Self::enum_variant_for_wire_value`], read off the same
    /// restricted map and therefore carrying the same guarantees: an answer exists only when a
    /// rename actually separates the two spellings AND the pairing is unambiguous in both
    /// directions (see [`super::super::super::ir_enum::build_enum_wire_variants`]).
    ///
    /// A generator that renders a value read off the WIRE surface — a wasm `to_api_str()` getter,
    /// a `serde_wasm_bindgen` payload — but compares it against a fixture's expected value needs
    /// this whenever the fixture named the variant by its Rust identifier. `None` means either
    /// "the IR cannot resolve this field to a concrete enum" or "no rename is in effect", and a
    /// caller must treat both the same way: keep the fixture value untranslated, which is correct
    /// whenever the identifier IS the wire value and is the behaviour that predates this lookup.
    ///
    /// The inversion is total rather than lossy: every variant produces exactly one wire value,
    /// so the wire-keyed map holds at most one entry per Rust identifier. ~keep
    pub fn enum_wire_value_for_variant(&self, field: &str, variant: &str) -> Option<&str> {
        let enum_name = self.ir_enum_type_name(field)?;
        self.ir_enum_map
            .enum_wire_variants
            .get(&enum_name)?
            .iter()
            .find(|(_, identifier)| identifier.as_str() == variant)
            .map(|(wire, _)| wire.as_str())
    }

    /// The serde discriminator key and wire value for a concrete tagged-enum variant.
    pub fn tagged_enum_wire_discriminator(&self, union_type: &str, variant: &str) -> Option<(&str, &str)> {
        let wire = self.ir_enum_map.tagged_enum_wire.get(union_type)?;
        Some((wire.tag.as_str(), wire.variants.get(variant)?.as_str()))
    }

    /// The serde `content` key for `union_type`, when it is adjacently tagged
    /// (`#[serde(tag = "..", content = "..")]`). `None` for an internally-tagged enum (no
    /// `content` attribute) as well as for a type the IR does not resolve to a tagged enum at
    /// all — callers that must tell those two apart check [`Self::ir_enum_type_name`] first.
    pub fn tagged_enum_content_key(&self, union_type: &str) -> Option<&str> {
        self.ir_enum_map.tagged_enum_wire.get(union_type)?.content.as_deref()
    }

    /// Whether the IR enum backing `field` carries data on at least one variant, per
    /// [`super::super::super::types::IrEnumMap::data_carrying_enum_names`]. `None` when the IR does
    /// not positively resolve `field` to a concrete enum type (unresolved root type, or a field
    /// classified as enum only via the hand-maintained `fields_enum` config) — a caller must decide
    /// its own fallback for "unknown" rather than this method guessing.
    ///
    /// Callers are the assertion generators whose emitted accessor for an enum-typed field is a
    /// scalar lowering the binding backend only declares on the unit-only shape: Dart's
    /// `.wireValue`, Kotlin/Android's `.toWire()`, the Kotlin JVM facade's `.getValue()`, and
    /// Swift's `.rawValue`. `Some(true)` means the binding rendered a payload-bearing union
    /// instead, and none of those accessors exist on it. ~keep
    pub fn ir_enum_is_data_carrying(&self, field: &str) -> Option<bool> {
        let name = self.ir_enum_type_name(field)?;
        Some(self.ir_enum_map.data_carrying_enum_names.contains(&name))
    }

    /// Whether the Java binding backend emits a `getValue()` accessor for the enum type
    /// backing `field`, per `backends::java::gen_bindings::emits_get_value`. `None` when the
    /// IR does not positively resolve `field` to a concrete enum type (unresolved root type,
    /// or a field classified as enum only via the hand-maintained `fields_enum` config) — the
    /// caller must decide its own fallback for "unknown" rather than this method guessing.
    pub fn java_enum_emits_get_value(&self, field: &str) -> Option<bool> {
        let name = self.ir_enum_type_name(field)?;
        Some(!self.java_wrapper_enum_names.contains(&name))
    }

    /// Whether Ruby's Magnus binding backend lowers the enum type backing `field`'s leaf segment
    /// to a plain `Hash` (per `ruby_hash_serialized_enum_names`) rather than a `Symbol`. `None`
    /// when the IR does not positively resolve `field` to a concrete enum type at all (unresolved
    /// root type, a path segment the IR does not recognize, or a field classified as enum only
    /// via the hand-maintained `fields_enum` config) — the caller decides its own fallback for
    /// "unknown" rather than this method guessing.
    pub fn ruby_enum_serialized_as_hash(&self, field: &str) -> Option<bool> {
        let name = self.ir_enum_type_name(field)?;
        Some(self.ruby_hash_serialized_enum_names.contains(&name))
    }
}
