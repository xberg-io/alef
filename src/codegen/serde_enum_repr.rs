//! The single place that answers "how does serde put this enum on the JSON wire?".
//!
//! serde has exactly four enum representations and the choice between them is made by the
//! `tag`/`content`/`untagged` container attributes alone — never by the shape of the variants.
//! Backends that re-derive the answer drift apart from each other and from Rust: the Swift
//! trait-bridge result encoder assumed serde's *external* default for every enum, so an
//! adjacently tagged enum went out as the bare string `"Variant"` and Rust rejected every
//! callback with `invalid type: string "...", expected adjacently tagged enum ...`, while Go
//! (which did consult `serde_content`) stayed correct.
//!
//! Every backend that emits or parses the JSON form of an IR enum must classify it through
//! [`serde_enum_repr`] so a future edit cannot reintroduce that divergence.

use crate::core::ir::{EnumDef, EnumVariant, TypeDef, TypeRef};

/// serde's four enum representations, carrying the key names each one needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerdeEnumRepr {
    /// serde's default. Unit variants are the bare string `"Variant"`; data variants are
    /// `{"Variant": payload}`.
    External,
    /// `#[serde(tag = "t")]`. The variant name is a field of the payload object:
    /// `{"t":"variant", ...payload fields}`.
    Internal { tag: String },
    /// `#[serde(tag = "t", content = "c")]`. The payload sits under its own key:
    /// `{"t":"variant","c":payload}`, and unit variants are just `{"t":"variant"}`.
    Adjacent { tag: String, content: String },
    /// `#[serde(untagged)]`. The payload alone, with no variant name anywhere; unit variants
    /// are `null`.
    Untagged,
}

impl SerdeEnumRepr {
    /// A stable lower-case discriminator suitable for a template context value.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Internal { .. } => "internal",
            Self::Adjacent { .. } => "adjacent",
            Self::Untagged => "untagged",
        }
    }

    /// The wire key holding the variant name, for the representations that have one.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Internal { tag } | Self::Adjacent { tag, .. } => Some(tag),
            Self::External | Self::Untagged => None,
        }
    }

    /// The wire key holding the payload — adjacent tagging only.
    pub fn content(&self) -> Option<&str> {
        match self {
            Self::Adjacent { content, .. } => Some(content),
            Self::External | Self::Internal { .. } | Self::Untagged => None,
        }
    }
}

/// Classify an IR enum's serde JSON representation.
///
/// `serde_untagged` wins over a stray tag because `#[serde(untagged)]` and `#[serde(tag = ...)]`
/// are mutually exclusive in serde, and the absence of `serde_tag` means *external*, not
/// untagged — the two are different wire forms and conflating them is exactly the bug this
/// module exists to prevent.
pub fn serde_enum_repr(enum_def: &EnumDef) -> SerdeEnumRepr {
    if enum_def.serde_untagged {
        return SerdeEnumRepr::Untagged;
    }
    match (enum_def.serde_tag.as_deref(), enum_def.serde_content.as_deref()) {
        (Some(tag), Some(content)) => SerdeEnumRepr::Adjacent {
            tag: tag.to_string(),
            content: content.to_string(),
        },
        (Some(tag), None) => SerdeEnumRepr::Internal { tag: tag.to_string() },
        (None, _) => SerdeEnumRepr::External,
    }
}

/// The discriminator key a backend synthesizes when it lowers a data-carrying IR enum to a
/// tagged object even though serde put no tag on the wire ([`SerdeEnumRepr::External`]).
///
/// Host object models frequently cannot express a Rust sum type directly — a `#[napi(object)]`,
/// a `#[wasm_bindgen]` struct, an ext-php-rs `#[php_class]`, a Ruby `Data.define` — so those
/// backends flatten every variant's payload into one struct and add a string discriminator. The
/// key is not read from the IR in that case (there is nothing to read), so it is a *convention*,
/// and a convention spelled inline at each emitter is a per-emitter oracle rather than one fact.
pub const DEFAULT_TAGGED_OBJECT_TAG_KEY: &str = "type";

/// The discriminator key for an IR enum a backend lowers to a tagged object: the enum's own
/// `#[serde(tag = "...")]` when it has one, else [`DEFAULT_TAGGED_OBJECT_TAG_KEY`].
///
/// Every emitter that names a discriminator must call this. Restating
/// `enum_def.serde_tag.as_deref().unwrap_or(<literal>)` locally is how the same IR enum could
/// acquire two different keys: fourteen call sites across the napi, wasm, php, swift, extendr,
/// rustler and elixir emitters spelled the fallback `"type"` while
/// `backends::magnus::gen_bindings::tagged_enums` spelled it `"kind"`. Nothing detects that —
/// each emitter's own tests agree with its own literal.
///
/// Scope, measured rather than assumed: the magnus `"kind"` fallback was **not reachable**, and
/// never had been. Its only production call site, `backends::magnus::gen_bindings::mod.rs`'s
/// `for enum_def in &api.enums` loop, is guarded by `enum_def.serde_tag.is_some() && …`, and that
/// guard is verbatim identical in the commit that introduced the `"kind"` literal. When the guard
/// admits an enum, `serde_tag` is `Some` and every backend reads it, so the fallback that
/// differed was dead on that path. Magnus's `serde_tag.is_none()` path emits per-variant
/// constructors and no discriminator at all. So this centralization removes a **latent** trap —
/// any future relaxation of that guard would have silently produced `hash[:kind]` against
/// `"type"`-keyed payloads — not an observed cross-binding failure.
///
/// This is a WIRE name, never a host-language public identifier — do not case it through
/// `naming::public_host_identifier`. ~keep
#[must_use]
pub fn tagged_object_tag_key(enum_def: &EnumDef) -> &str {
    enum_def.serde_tag.as_deref().unwrap_or(DEFAULT_TAGGED_OBJECT_TAG_KEY)
}

#[cfg(test)]
mod tagged_object_tag_key_tests {
    use super::*;

    fn enum_with(tag: Option<&str>, content: Option<&str>) -> EnumDef {
        EnumDef {
            name: "Sample".to_string(),
            serde_tag: tag.map(str::to_string),
            serde_content: content.map(str::to_string),
            ..EnumDef::default()
        }
    }

    /// One row: case name, `#[serde(tag)]`, `#[serde(content)]`, the key that must come back.
    type TagKeyCase<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a str);

    #[test]
    fn should_resolve_the_discriminator_key_for_every_container_attribute_shape() {
        let cases: [TagKeyCase<'_>; 5] = [
            (
                "no container attribute falls back to the convention",
                None,
                None,
                "type",
            ),
            ("an explicit tag wins over the fallback", Some("kind"), None, "kind"),
            (
                "an explicit tag wins even when it equals the fallback",
                Some("type"),
                None,
                "type",
            ),
            (
                "adjacent tagging keys on the tag, not the content",
                Some("kind"),
                Some("payload"),
                "kind",
            ),
            (
                "a content without a tag still falls back to the convention",
                None,
                Some("payload"),
                "type",
            ),
        ];

        for (case, tag, content, expected) in cases {
            assert_eq!(tagged_object_tag_key(&enum_with(tag, content)), expected, "{case}");
        }
    }

    /// The fallback and the key an explicitly-`type`-tagged enum yields must be the same string,
    /// so no emitter can be correct for one shape and wrong for the other.
    #[test]
    fn should_agree_with_the_named_constant() {
        assert_eq!(
            tagged_object_tag_key(&enum_with(None, None)),
            DEFAULT_TAGGED_OBJECT_TAG_KEY
        );
    }
}

/// True when serde merges `variant`'s single positional payload into the tag-bearing object, so
/// the wire carries the payload's own fields and NO key for the payload itself.
///
/// The `_0` in a newtype variant's `fields` (`Excel(ExcelMetadata)`) is synthesized by the
/// extractor, never written by the user. Under internal tagging serde flattens the payload in
/// beside the tag — `{"format_type":"excel","sheet_count":2}` — so a surface that renders that
/// field name verbatim advertises a `_0` key nothing emits: a Ruby `hash[:_0]` that is always
/// `nil`, a `.pyi` key no payload carries, a wasm `"0"` getter for an absent property.
///
/// Narrowed to [`SerdeEnumRepr::Internal`] on purpose — every other representation keeps a real
/// key for the payload (external: the variant name; adjacent: the content key; untagged: the
/// bare payload), so a tuple variant's `_0` is legitimate there and must keep working.
///
/// serde only flattens a newtype payload that itself serializes as a JSON *map* — internal
/// tagging is rejected outright by `serde_derive` for a payload that is not a struct/map, so the
/// wire shape this predicate describes exists only for a payload that resolves to a real `types`
/// entry. `types` is therefore required, not optional: a primitive payload, or a `Named` payload
/// this binding's IR did not resolve, answers `false` here exactly as
/// [`crate::backends::napi::gen_bindings::enums::tagged_enum_flattened_newtype`] (the
/// resolution-aware reference this predicate now mirrors) already required. Passing an empty
/// `types` slice therefore is NOT a safe stand-in for "I don't have the type list" — it makes
/// every payload answer unresolvable, silently reintroducing the callers' `_0`/`"0"` defect this
/// function exists to prevent. A caller with no type list in scope must thread one down from its
/// own caller instead. ~keep
///
/// This exists as one predicate because the split already happened once: alef 0.85.11 taught the
/// Magnus *binding* to emit `#[serde(flatten)]` here and left the surfaces that read the same
/// variant still emitting the `_0` hop, which turned every tagged-enum payload assertion in a
/// consumer's Ruby suite into a `KeyError` (recorded at
/// `crate::e2e::field_access::types::FieldAccessIndex::variant_payload_tuple`). ~keep
#[must_use]
pub fn serde_flattens_newtype_payload(enum_def: &EnumDef, variant: &EnumVariant, types: &[TypeDef]) -> bool {
    if !matches!(serde_enum_repr(enum_def), SerdeEnumRepr::Internal { .. }) {
        return false;
    }
    if !(variant.is_tuple && variant.fields.len() == 1) {
        return false;
    }
    let TypeRef::Named(inner_name) = &variant.fields[0].ty else {
        return false;
    };
    types.iter().any(|t| &t.name == inner_name)
}

#[cfg(test)]
mod serde_flattens_newtype_payload_tests {
    use super::*;
    use crate::core::ir::FieldDef;

    fn enum_with(tag: Option<&str>, content: Option<&str>, untagged: bool, variant: EnumVariant) -> EnumDef {
        EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: tag.map(str::to_string),
            serde_content: content.map(str::to_string),
            serde_untagged: untagged,
            variants: vec![variant],
            ..EnumDef::default()
        }
    }

    fn variant(is_tuple: bool, field_names: &[&str]) -> EnumVariant {
        EnumVariant {
            name: "Excel".to_string(),
            is_tuple,
            fields: field_names
                .iter()
                .map(|name| FieldDef {
                    name: (*name).to_string(),
                    ty: TypeRef::Named("ExcelMetadata".to_string()),
                    ..FieldDef::default()
                })
                .collect(),
            ..EnumVariant::default()
        }
    }

    /// The payload struct `variant`'s single field resolves to, so the resolution-aware
    /// predicate can actually see a match. Every "flattened" row needs this in `types`; the
    /// unresolvable row deliberately omits it. ~keep
    fn excel_metadata_types() -> Vec<TypeDef> {
        vec![TypeDef {
            name: "ExcelMetadata".to_string(),
            ..TypeDef::default()
        }]
    }

    /// One row: case name, tag, content, untagged, the variant, the `types` list, and whether
    /// serde flattens it.
    type FlattenCase<'a> = (
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        bool,
        EnumVariant,
        Vec<TypeDef>,
        bool,
    );

    fn flatten_cases() -> [FlattenCase<'static>; 8] {
        [
            (
                "internally tagged newtype is the flattened shape",
                Some("format_type"),
                None,
                false,
                variant(true, &["_0"]),
                excel_metadata_types(),
                true,
            ),
            (
                "an internally tagged newtype whose payload type does not resolve is not flattened",
                Some("format_type"),
                None,
                false,
                variant(true, &["_0"]),
                Vec::new(),
                false,
            ),
            (
                "adjacent tagging keeps the payload under the content key",
                Some("format_type"),
                Some("payload"),
                false,
                variant(true, &["_0"]),
                excel_metadata_types(),
                false,
            ),
            (
                "untagged writes the bare payload",
                None,
                None,
                true,
                variant(true, &["_0"]),
                excel_metadata_types(),
                false,
            ),
            (
                "external tagging keys the payload on the variant name",
                None,
                None,
                false,
                variant(true, &["_0"]),
                excel_metadata_types(),
                false,
            ),
            (
                "untagged wins over a stray tag, so the payload is still not flattened",
                Some("format_type"),
                None,
                true,
                variant(true, &["_0"]),
                excel_metadata_types(),
                false,
            ),
            (
                "a struct variant's fields are already flat and carry real names",
                Some("format_type"),
                None,
                false,
                variant(false, &["sheet_count"]),
                excel_metadata_types(),
                false,
            ),
            (
                "a multi-field tuple variant is not a newtype",
                Some("format_type"),
                None,
                false,
                variant(true, &["_0", "_1"]),
                excel_metadata_types(),
                false,
            ),
        ]
    }

    #[test]
    fn should_flatten_only_an_internally_tagged_newtype_variant_whose_payload_resolves() {
        let cases = flatten_cases();

        for (case, tag, content, untagged, variant, types, expected) in cases {
            let enum_def = enum_with(tag, content, untagged, variant);
            assert_eq!(
                serde_flattens_newtype_payload(&enum_def, &enum_def.variants[0], &types),
                expected,
                "{case}"
            );
        }
    }
}

/// The inverse of [`crate::codegen::naming::wire_variant_value`]: given a value read off the
/// JSON/wire surface (an e2e fixture's enum-typed input, a recorded response body), return the
/// Rust variant name that produces it — which is also the public member identifier every binding
/// declares for that variant.
///
/// A caller that instead re-cases the wire value (`wire.to_upper_camel_case()`) is using a wire
/// name as a host-language identifier casing rule, which the `centralized-naming` rule forbids.
/// The two agree only for variants whose wire spelling happens to round-trip, and diverge
/// silently for `#[serde(rename = "...")]` (`"md"` -> `Md`, declared `Markdown`), for
/// `rename_all = "lowercase"` over a multi-word variant (`PlainText` -> `"plaintext"` ->
/// `Plaintext`), and for any acronym-cased variant (`HTML` -> `"html"` -> `Html`).
///
/// Both spellings are accepted, mirroring the field-side precedent in the TypeScript e2e
/// generator's `refuse_undeclared_json_keys`: a fixture may legitimately name an enum value by
/// its Rust identifier or by its wire value, and rejecting the former would fail a correctly
/// authored fixture. `None` means no declared variant carries that wire value at all, which a
/// caller must not paper over by inventing an identifier. ~keep
#[must_use]
pub fn variant_name_for_wire<'a>(enum_def: &'a EnumDef, wire: &str) -> Option<&'a str> {
    let rename_all = enum_def.serde_rename_all.as_deref();
    enum_def
        .variants
        .iter()
        .find(|variant| {
            variant.name == wire
                || crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    rename_all,
                ) == wire
        })
        .map(|variant| variant.name.as_str())
}

#[cfg(test)]
mod variant_name_for_wire_tests {
    use super::*;

    fn output_format(rename_all: Option<&str>, variants: &[(&str, Option<&str>)]) -> EnumDef {
        EnumDef {
            name: "OutputFormat".to_string(),
            serde_rename_all: rename_all.map(str::to_string),
            variants: variants
                .iter()
                .map(|(name, rename)| EnumVariant {
                    name: (*name).to_string(),
                    serde_rename: rename.map(str::to_string),
                    ..EnumVariant::default()
                })
                .collect(),
            ..EnumDef::default()
        }
    }

    /// One row of the rename-resolution table: the case's name, the enum's `rename_all`, its
    /// variants as `(name, serde_rename)`, the wire value to resolve, and the variant that must
    /// come back (`None` for "no variant matches").
    type RenameCase<'a> = (
        &'a str,
        Option<&'a str>,
        &'a [(&'a str, Option<&'a str>)],
        &'a str,
        Option<&'a str>,
    );

    #[test]
    fn should_resolve_every_rename_strategy_back_to_the_declared_variant() {
        let cases: [RenameCase<'_>; 7] = [
            (
                "single word, no rename",
                None,
                &[("Markdown", None)],
                "Markdown",
                Some("Markdown"),
            ),
            (
                "single word under lowercase",
                Some("lowercase"),
                &[("Markdown", None)],
                "markdown",
                Some("Markdown"),
            ),
            (
                "multi word under snake_case",
                Some("snake_case"),
                &[("PlainText", None)],
                "plain_text",
                Some("PlainText"),
            ),
            (
                "multi word under lowercase collapses, so re-casing cannot recover it",
                Some("lowercase"),
                &[("PlainText", None)],
                "plaintext",
                Some("PlainText"),
            ),
            (
                "explicit rename wins over rename_all",
                Some("snake_case"),
                &[("Markdown", Some("md"))],
                "md",
                Some("Markdown"),
            ),
            (
                "acronym variant under lowercase",
                Some("lowercase"),
                &[("HTML", None)],
                "html",
                Some("HTML"),
            ),
            ("value no variant declares", None, &[("Markdown", None)], "pdf", None),
        ];

        for (case, rename_all, variants, wire, expected) in cases {
            let enum_def = output_format(rename_all, variants);
            assert_eq!(variant_name_for_wire(&enum_def, wire), expected, "{case}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enum_def(tag: Option<&str>, content: Option<&str>, untagged: bool) -> EnumDef {
        EnumDef {
            name: "Sample".to_string(),
            serde_tag: tag.map(str::to_string),
            serde_content: content.map(str::to_string),
            serde_untagged: untagged,
            ..EnumDef::default()
        }
    }

    #[test]
    fn should_be_external_when_no_serde_container_attribute_is_present() {
        let repr = serde_enum_repr(&enum_def(None, None, false));
        assert_eq!(repr, SerdeEnumRepr::External);
        assert_eq!(repr.kind(), "external");
        assert_eq!(repr.tag(), None);
        assert_eq!(repr.content(), None);
    }

    #[test]
    fn should_be_internal_when_only_tag_is_present() {
        let repr = serde_enum_repr(&enum_def(Some("type"), None, false));
        assert_eq!(
            repr,
            SerdeEnumRepr::Internal {
                tag: "type".to_string()
            }
        );
        assert_eq!(repr.kind(), "internal");
        assert_eq!(repr.tag(), Some("type"));
        assert_eq!(repr.content(), None);
    }

    #[test]
    fn should_be_adjacent_when_tag_and_content_are_both_present() {
        let repr = serde_enum_repr(&enum_def(Some("kind"), Some("payload"), false));
        assert_eq!(
            repr,
            SerdeEnumRepr::Adjacent {
                tag: "kind".to_string(),
                content: "payload".to_string()
            }
        );
        assert_eq!(repr.kind(), "adjacent");
        assert_eq!(repr.tag(), Some("kind"));
        assert_eq!(repr.content(), Some("payload"));
    }

    #[test]
    fn should_be_untagged_when_untagged_is_set() {
        let repr = serde_enum_repr(&enum_def(None, None, true));
        assert_eq!(repr, SerdeEnumRepr::Untagged);
        assert_eq!(repr.kind(), "untagged");
        assert_eq!(repr.tag(), None);
    }

    #[test]
    fn should_prefer_untagged_over_a_stray_tag() {
        assert_eq!(
            serde_enum_repr(&enum_def(Some("type"), None, true)),
            SerdeEnumRepr::Untagged
        );
    }

    #[test]
    fn should_ignore_content_without_a_tag() {
        assert_eq!(
            serde_enum_repr(&enum_def(None, Some("payload"), false)),
            SerdeEnumRepr::External
        );
    }
}
