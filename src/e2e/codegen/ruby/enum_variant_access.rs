//! Which Ruby shape `backends::magnus` lowers each IR enum to, and therefore whether a fixture
//! field path that steps *into* an enum's variant payload names anything the Ruby binding
//! actually exposes.
//!
//! ~keep This replaces two literal guards (`f.contains("metadata.format.")` and
//! `f == "metadata.format"`) that carried the reason "Magnus serializes FormatMetadata to JSON,
//! so variants are unavailable in Ruby". The path was one consumer crate's own field name
//! hard-coded into a project-agnostic generator: an identically shaped hash-serialized enum
//! under any other field name was not recognised at all (rendering a property chain against a
//! Ruby `Hash`, e.g. `NoMethodError`), while a field that merely happened to share the literal
//! name `metadata.format` was skipped whatever it actually resolved to.
//!
//! The real condition is the enum's Ruby *lowering* — a property of the enum's own IR shape
//! (`backends::magnus::gen_bindings::classes::gen_enum::gen_enum`'s `has_data` check) — resolved
//! by walking the fixture path through the crate's own IR, exactly as `php/enum_variant_access.rs`
//! does for PHP's three-way split. Magnus's lowering is simpler than PHP's: a data-carrying enum
//! (any variant with at least one field, tagged or not) always serializes through
//! `serde_json::to_value` into a plain Ruby `Hash` (see `enum_magnus.rs.jinja`'s `IntoValue` impl
//! for the `has_data` branch); a unit-variant-only enum always becomes a `Symbol`. There is no
//! third, member-bearing shape the way PHP's flat `#[php_class]` is.

use std::collections::HashSet;

use heck::ToUpperCamelCase;

use crate::core::ir::EnumDef;
use crate::e2e::field_access::FieldResolver;

/// Names of every enum this crate declares that Magnus lowers to a plain Ruby `Hash` rather than
/// a `Symbol` — restated from `gen_enum`'s own `has_data` predicate
/// (`enum_def.variants.iter().any(|v| !v.fields.is_empty())`), which that module cannot export
/// because `backends::magnus::gen_bindings` is private to `backends::magnus`.
/// `should_match_the_binding_backends_partition` below pins the two agree.
pub(super) fn hash_serialized_enum_names(enums: &[EnumDef]) -> HashSet<String> {
    enums
        .iter()
        .filter(|enum_def| enum_def.variants.iter().any(|variant| !variant.fields.is_empty()))
        .map(|enum_def| enum_def.name.clone())
        .collect()
}

/// What the Ruby binding offers for a fixture field path that may cross a hash-serialized enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RubyEnumAccess {
    /// Nothing to refuse: the path crosses no enum, or every enum on it is a `Symbol`.
    Available,
    /// The path lands exactly on a hash-serialized enum. A real `Hash` value exists, but the
    /// serialization differs between languages and doesn't preserve `Display` formatting, so a
    /// plain string/equality comparison against it is not meaningful.
    SerializedAsHash,
    /// The path steps PAST a hash-serialized enum into a variant payload segment. No native
    /// accessor exists on the Ruby side for that segment; the whole subtree is a `Hash`.
    VariantAccessorUnavailable,
}

/// Classify `field` by walking each of its leading segments and asking the IR (via
/// `field_resolver.ruby_enum_serialized_as_hash`) whether that prefix resolves to a
/// hash-serialized enum. Entirely name-agnostic: it fires for any field path the crate's own IR
/// confirms crosses a hash-serialized enum, under whatever name that field happens to have, and
/// it does NOT fire for a field that merely shares a literal name with one but resolves to
/// something else (a plain scalar, a unit-variant/`Symbol` enum, or nothing the IR recognizes).
pub(super) fn classify(field_resolver: &FieldResolver, field: &str) -> RubyEnumAccess {
    if field_resolver.ruby_enum_serialized_as_hash(field) == Some(true) {
        return RubyEnumAccess::SerializedAsHash;
    }
    let segments: Vec<&str> = field.split('.').collect();
    for i in 1..segments.len() {
        let prefix = segments[..i].join(".");
        if field_resolver.ruby_enum_serialized_as_hash(&prefix) == Some(true) {
            return RubyEnumAccess::VariantAccessorUnavailable;
        }
    }
    RubyEnumAccess::Available
}

/// Render the one Ruby-accessible tagged-enum payload shape Alef can prove from the IR: a path
/// crosses a known single-payload variant and names one plain field on that payload.
///
/// ~keep Core's own wire format flattens a tagged enum's single-field variant beside the
/// discriminator (`#[serde(tag = "format_type")] enum FormatMetadata { Excel(ExcelMetadata) }`
/// serializes to `{"format_type":"excel","sheet_count":2,...}`), but `backends::magnus` does NOT
/// mirror that shape. `enum_magnus.rs.jinja` restates every such variant as a Rust STRUCT variant
/// (`Excel { _0: ExcelMetadata }` — see `gen_enum`'s `emits_tuple_variant`, which is false
/// whenever the enum has neither `serde_content` nor `serde_untagged`), so its own
/// `serde_json::to_value` nests the payload one level deeper, under the field's own name:
/// `{"format_type":"excel","_0":{"sheet_count":2,...}}`. An adjacently-tagged enum
/// (`#[serde(tag = "..", content = "..")]`) nests under that configured `content` key instead,
/// regardless of tuple/struct form, because serde puts adjacently-tagged payloads there
/// unconditionally. Either way there IS a runtime Hash level between the discriminator and the
/// payload field on the Ruby side — omitting that hop (as this function once did) produces a
/// `KeyError` for every tagged-enum payload assertion in the Ruby suite. `union_variant_payload`'s
/// field name is exactly the key `backends::magnus` declares for the wrapped field (both read the
/// same IR field), so using it (falling back to the `content` key when the enum sets one) keeps
/// this generator and that backend from drifting independently.
///
/// Unsupported/nested/indexed suffixes return `None` and retain the explicit generator-gap skip.
pub(super) fn variant_field_accessor(field_resolver: &FieldResolver, field: &str, result_var: &str) -> Option<String> {
    let resolved = field_resolver.resolve(field);
    let segments: Vec<&str> = resolved.split('.').collect();
    for enum_index in 1..segments.len().saturating_sub(1) {
        let prefix = segments[..enum_index].join(".");
        if field_resolver.ruby_enum_serialized_as_hash(&prefix) != Some(true) {
            continue;
        }

        let variant = segments[enum_index];
        let [payload_field] = segments.get(enum_index + 1..)? else {
            return None;
        };
        if !payload_field
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
        {
            return None;
        }
        let enum_type = field_resolver.ir_enum_type_name(&prefix)?;
        let variant = variant.to_upper_camel_case();
        let (wrapped_field, payload_type) = field_resolver.union_variant_payload(&enum_type, &variant)?;
        if field_resolver.is_declared_field_of_type(payload_type, payload_field) != Some(true) {
            return None;
        }
        let (serde_tag, wire_variant) = field_resolver.tagged_enum_wire_discriminator(&enum_type, &variant)?;
        let magnus_wrapper_key = field_resolver
            .tagged_enum_content_key(&enum_type)
            .unwrap_or(wrapped_field);

        let enum_hash = field_resolver.accessor(&prefix, "ruby", result_var);
        let tag = crate::e2e::escape::ruby_string_literal(serde_tag);
        let wire_variant = crate::e2e::escape::ruby_string_literal(wire_variant);
        let magnus_wrapper_key = crate::e2e::escape::ruby_string_literal(magnus_wrapper_key);
        let payload_field = crate::e2e::escape::ruby_string_literal(payload_field);
        return Some(format!(
            concat!(
                "{enum_hash}.then {{ |enum_hash| raise \"unexpected tagged enum variant\" ",
                "unless enum_hash.fetch({tag}.to_sym) == {wire_variant}; ",
                "enum_hash.fetch({magnus_wrapper_key}.to_sym).fetch({payload_field}.to_sym) }}"
            ),
            enum_hash = enum_hash,
            tag = tag,
            wire_variant = wire_variant,
            magnus_wrapper_key = magnus_wrapper_key,
            payload_field = payload_field,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{RubyEnumAccess, classify, hash_serialized_enum_names};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn named(name: &str) -> TypeRef {
        TypeRef::Named(name.to_string())
    }

    /// A neutral fixture crate carrying the two Ruby enum lowerings:
    ///
    /// * `EncodingDetails` — a data-carrying (internally tagged) enum, lowered to a `Hash`.
    /// * `DocumentKind` — unit variants only, lowered to a `Symbol`.
    ///
    /// `metadata` deliberately carries a plain `String` field named `format` — the exact literal
    /// spelling the old guard matched on — so a test can prove that name alone no longer triggers
    /// the skip.
    fn ir() -> (Vec<TypeDef>, Vec<EnumDef>) {
        let type_defs = vec![
            TypeDef {
                name: "ProcessingResult".to_string(),
                fields: vec![
                    field("summary", named("DocumentSummary")),
                    field("metadata", named("PageMetadata")),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DocumentSummary".to_string(),
                fields: vec![
                    field("encoding", named("EncodingDetails")),
                    field("kind", named("DocumentKind")),
                    field("adjacent", named("AdjacentDetails")),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "PageMetadata".to_string(),
                fields: vec![field("format", TypeRef::String)],
                ..TypeDef::default()
            },
            TypeDef {
                name: "SpreadsheetDetails".to_string(),
                fields: vec![field("sheet_count", TypeRef::Primitive(PrimitiveType::U32))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "WrappedPayload".to_string(),
                fields: vec![field("value", TypeRef::String)],
                ..TypeDef::default()
            },
        ];
        let enums = vec![
            EnumDef {
                name: "EncodingDetails".to_string(),
                serde_tag: Some("type'kind".to_string()),
                variants: vec![
                    EnumVariant {
                        name: "Spreadsheet".to_string(),
                        serde_rename: Some("sheet'kind".to_string()),
                        is_tuple: true,
                        fields: vec![field("_0", named("SpreadsheetDetails"))],
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Empty".to_string(),
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Pair".to_string(),
                        fields: vec![field("left", TypeRef::String), field("right", TypeRef::String)],
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DocumentKind".to_string(),
                variants: vec![EnumVariant {
                    name: "Report".to_string(),
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "AdjacentDetails".to_string(),
                serde_tag: Some("kind".to_string()),
                serde_content: Some("body".to_string()),
                variants: vec![EnumVariant {
                    name: "Wrapped".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", named("WrappedPayload"))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
        ];
        (type_defs, enums)
    }

    fn resolver() -> FieldResolver {
        let (type_defs, enums) = ir();
        let map = FieldResolver::ir_enum_fields(&type_defs, &enums);
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(map, Some("ProcessingResult".to_string()))
        .with_ruby_hash_serialized_enum_names(hash_serialized_enum_names(&enums))
        .with_ir_result_fields(
            FieldResolver::ir_result_field_facts(&type_defs, "ruby"),
            Some("ProcessingResult".to_string()),
        )
    }

    /// THE CANARY (positive control). A field path that crosses a hash-serialized enum, spelled
    /// under a name that is nothing like the old literal `metadata.format`, must still refuse —
    /// proving the condition is the enum's own IR-derived lowering, not the field's name.
    #[test]
    fn a_variant_path_through_a_hash_serialized_enum_is_refused_under_any_name() {
        let out = classify(&resolver(), "summary.encoding.spreadsheet.sheet_count");
        assert_eq!(out, RubyEnumAccess::VariantAccessorUnavailable);
    }

    /// Same enum, path stops exactly on it: a `Hash` value does exist, but comparisons against
    /// it as a string/Display value are not meaningful.
    #[test]
    fn a_path_landing_exactly_on_a_hash_serialized_enum_is_flagged_as_hash() {
        let out = classify(&resolver(), "summary.encoding");
        assert_eq!(out, RubyEnumAccess::SerializedAsHash);
    }

    /// THE NEGATIVE CONTROL. `metadata.format` is the exact literal the old guard matched on —
    /// here it resolves through the IR to a plain `String`, not an enum at all, so it must NOT be
    /// refused. This is what tells a real condition apart from a renamed string match.
    #[test]
    fn a_field_that_merely_shares_the_old_literal_name_is_not_refused() {
        let out = classify(&resolver(), "metadata.format");
        assert_eq!(out, RubyEnumAccess::Available);
    }

    /// A `Symbol`-lowered (unit-variant-only) enum has no `Hash` shape to worry about, whatever
    /// the field is named.
    #[test]
    fn a_symbol_lowered_enum_field_is_available() {
        let out = classify(&resolver(), "summary.kind");
        assert_eq!(out, RubyEnumAccess::Available);
    }

    /// An unresolved resolver (no IR wired in at all — the state every resolver had before
    /// `with_ruby_hash_serialized_enum_names` existed) must never refuse: absence of IR data is
    /// "unknown", not "hash-serialized".
    #[test]
    fn with_no_ir_wired_in_nothing_is_ever_refused() {
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            classify(&resolver, "summary.encoding.spreadsheet.sheet_count"),
            RubyEnumAccess::Available
        );
        assert_eq!(classify(&resolver, "metadata.format"), RubyEnumAccess::Available);
    }

    /// The partition must reproduce the binding backend's `has_data` split, since it decides both
    /// this skip and the Rust-side `IntoValue` shape.
    #[test]
    fn should_match_the_binding_backends_partition() {
        let (_, enums) = ir();
        let names = hash_serialized_enum_names(&enums);
        assert!(
            names.contains("EncodingDetails"),
            "a data-carrying enum lowers to a Hash"
        );
        assert!(
            !names.contains("DocumentKind"),
            "a unit-variant-only enum lowers to a Symbol"
        );
    }

    fn render(field: &str) -> String {
        let assertion = crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!("Excel")),
            ..Default::default()
        };
        let mut out = String::new();
        super::super::assertions::render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver(),
            false,
            &crate::e2e::config::E2eConfig::default(),
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
        );
        out
    }

    /// End to end through `render_assertion` itself (not just `classify`), under a field name
    /// that is nothing like the old literal `metadata.format` — proving the real entry point, not
    /// just the classifier in isolation, reaches the flattened Symbol-keyed Hash payload.
    #[test]
    fn render_assertion_reaches_a_hash_serialized_variant_field_under_any_name() {
        let out = render("summary.encoding.spreadsheet.sheet_count");
        assert!(out.contains("result.summary.encoding.then { |enum_hash|"), "got: {out}");
        assert!(
            out.contains("enum_hash.fetch(\"type'kind\".to_sym) == \"sheet'kind\""),
            "got: {out}"
        );
        assert!(
            out.contains("enum_hash.fetch('_0'.to_sym).fetch('sheet_count'.to_sym)"),
            "got: {out}"
        );
        assert!(!out.contains("# skipped:"), "got: {out}");
    }

    /// THE REGRESSION for the CI-confirmed `_0` hop defect: `backends::magnus` restates every
    /// single-field tagged-enum variant as a struct with a field named after the IR's own field
    /// name (`_0` for a tuple-origin variant), so the payload is nested one Hash level deeper
    /// than the tag check alone reaches. Without the `.fetch('_0'.to_sym)` hop, the generated
    /// Ruby raises `KeyError: key not found: :sheet_count` against the real binding — this test
    /// pins the hop is present in the generated source.
    #[test]
    fn render_assertion_hops_through_the_magnus_wrapper_field_before_the_payload_field() {
        let out = render("summary.encoding.spreadsheet.sheet_count");
        assert!(
            out.contains("enum_hash.fetch('_0'.to_sym).fetch('sheet_count'.to_sym)"),
            "got: {out}"
        );
    }

    /// An adjacently-tagged enum (`tag` + `content`) nests its payload under the CONFIGURED
    /// `content` key, not the IR field name — serde puts an adjacently-tagged payload there
    /// unconditionally, regardless of the field's own name. This must win over the `_0` fallback.
    #[test]
    fn render_assertion_hops_through_the_configured_content_key_for_adjacently_tagged_enums() {
        let out = render("summary.adjacent.wrapped.value");
        assert!(
            out.contains("enum_hash.fetch('body'.to_sym).fetch('value'.to_sym)"),
            "got: {out}"
        );
        assert!(!out.contains("fetch('_0'.to_sym)"), "got: {out}");
    }

    /// A supported single-field payload is executable and therefore absent from the skip ledger.
    #[test]
    fn render_assertion_supported_variant_field_has_no_skip_classification() {
        use crate::e2e::codegen::field_skip::FieldSkip;
        let out = render("summary.encoding.spreadsheet.sheet_count");
        assert_eq!(FieldSkip::extract_classified(&out), None, "got: {out}");
    }

    #[test]
    fn runtime_accessor_rejects_a_different_wire_variant() {
        let out = render("summary.encoding.spreadsheet.sheet_count");
        assert!(
            out.contains("raise \"unexpected tagged enum variant\" unless"),
            "got: {out}"
        );
        assert!(
            out.contains("fetch(\"type'kind\".to_sym) == \"sheet'kind\""),
            "got: {out}"
        );
    }

    #[test]
    fn nonexistent_payload_leaf_remains_a_generator_gap() {
        let out = render("summary.encoding.spreadsheet.not_declared");
        assert!(out.contains("# skipped: enum variant accessor"), "got: {out}");
        assert!(!out.contains("fetch(\"not_declared\".to_sym)"), "got: {out}");
    }

    #[test]
    fn nested_and_indexed_payload_suffixes_remain_generator_gaps() {
        for field in [
            "summary.encoding.spreadsheet.sheet_count.value",
            "summary.encoding.spreadsheet.sheet_count[0]",
        ] {
            let out = render(field);
            assert!(
                out.contains("# skipped: enum variant accessor"),
                "field={field}, got: {out}"
            );
        }
    }

    #[test]
    fn unknown_variant_remains_a_generator_gap() {
        for field in [
            "summary.encoding.unknown.sheet_count",
            "summary.encoding.empty.sheet_count",
            "summary.encoding.pair.left",
        ] {
            let out = render(field);
            assert!(
                out.contains("# skipped: enum variant accessor"),
                "field={field}, got: {out}"
            );
        }
    }
}
