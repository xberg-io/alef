//! PHP enum declaration-parity, labeled-string and external serde coverage, split from tests.rs.

/// Coverage for the declaration-surface parity fix: `enum_constant_entries` (the class-constants
/// declaration) and `gen_flat_data_enum_methods`'s `allowed_tags` (the flat-enum `from_json`
/// validation list) must both defer to `codegen::conversions::enum_variant_declaration` -- the
/// same authority the napi backend's `gen_enum` already consults -- so a FOREIGN cfg-gated
/// variant this binding's configured feature set proves unreachable is never advertised as
/// constructible, while a host-owned cfg-gated variant (or a foreign one that is merely unproven,
/// not proven absent) keeps its prior unconditional behavior. ~keep
#[cfg(test)]
mod enum_declaration_parity_tests {
    use super::super::{enum_constant_entries, gen_enum_constants, gen_flat_data_enum_methods};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant};
    use ahash::AHashSet;
    use std::collections::HashSet;

    const HOST_CRATE: &str = "hostlib";

    fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// `Base` is always ungated; `Extra` carries `cfg`. `owner` selects whether the whole enum
    /// (and therefore `Extra`'s cfg) is host- or foreign-owned relative to [`HOST_CRATE`].
    fn sample_enum(owner: &str, cfg: &str) -> EnumDef {
        EnumDef {
            name: "SampleMode".to_string(),
            rust_path: format!("{owner}::SampleMode"),
            serde_tag: Some("kind".to_string()),
            variants: vec![unit_variant("Base", None), unit_variant("Extra", Some(cfg))],
            ..Default::default()
        }
    }

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    /// A foreign-owned `Extra` behind a feature absent from `configured_features` is proven
    /// unreachable -- neither the class constant nor the flat-enum `allowed_tags` entry may
    /// advertise it, since the binding->core conversion can never produce it either.
    #[test]
    fn foreign_variant_proven_unreachable_is_absent_from_both_declaration_surfaces() {
        let en = sample_enum("dep_crate", r#"feature = "testkit""#);
        let is_host_enum = false;
        let configured: HashSet<&str> = HashSet::new();

        let entries = enum_constant_entries(&en, is_host_enum, Some(&configured));
        assert_eq!(
            entries,
            vec![("BASE".to_string(), "Base".to_string())],
            "the proven-unreachable foreign variant must be dropped, got: {entries:?}"
        );
        let constants = gen_enum_constants(&en, None, is_host_enum, Some(&configured));
        assert!(!constants.contains("EXTRA"), "got:\n{constants}");

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, Some(&configured));
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" )"),
            "allowed_tags must list only the reachable variant, got:\n{methods}"
        );
        assert!(!methods.contains("\"Extra\""), "got:\n{methods}");
    }

    /// The same foreign `Extra`, but `configured_features` is `None` ("unknown" -- alef's static
    /// read cannot prove the feature off, since Cargo feature unification could still enable it).
    /// Both declaration surfaces must keep advertising it, unchanged from before this fix.
    #[test]
    fn foreign_variant_not_proven_unreachable_stays_on_both_declaration_surfaces() {
        let en = sample_enum("dep_crate", r#"feature = "testkit""#);
        let is_host_enum = false;

        let entries = enum_constant_entries(&en, is_host_enum, None);
        assert_eq!(
            entries,
            vec![
                ("BASE".to_string(), "Base".to_string()),
                ("EXTRA".to_string(), "Extra".to_string()),
            ],
            "an unproven foreign variant must stay declared, got: {entries:?}"
        );

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, None);
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" | \"Extra\" )"),
            "got:\n{methods}"
        );
    }

    /// A host-owned cfg-gated `Extra` (the enum's `rust_path` is rooted in [`HOST_CRATE`] itself)
    /// must never be dropped by [`enum_variant_declaration`] -- existing behavior, unchanged by
    /// this fix -- regardless of what `configured_features` says.
    #[test]
    fn host_owned_cfg_gated_variant_stays_on_both_declaration_surfaces() {
        let en = sample_enum(HOST_CRATE, r#"feature = "extra_feature""#);
        let is_host_enum = true;
        let configured: HashSet<&str> = HashSet::new();

        let entries = enum_constant_entries(&en, is_host_enum, Some(&configured));
        assert_eq!(
            entries,
            vec![
                ("BASE".to_string(), "Base".to_string()),
                ("EXTRA".to_string(), "Extra".to_string()),
            ],
            "a host-owned cfg-gated variant must stay declared, got: {entries:?}"
        );

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, Some(&configured));
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" | \"Extra\" )"),
            "got:\n{methods}"
        );
    }
}

/// Coverage for the `Custom(String)` label-loss fix: an externally-tagged enum whose only data
/// variant is a single-field `String` tuple variant (`EntityCategory`/`PiiCategory`/`OutputFormat`
/// in the xberg core crate) must round-trip the caller-supplied label through a flat PHP class
/// instead of being flattened to a bare string constant that can only ever produce
/// `Custom(Default::default())`. Before this fix, `is_tagged_data_enum` required
/// `enum_def.serde_tag.is_some()`, so every assertion below that depends on the broadened
/// predicate failed: `entity_category()` routed through `enum_constant_entries` /
/// `gen_string_to_enum_expr` instead of `gen_flat_data_enum*`, and the generated binding→core
/// conversion literally read `"custom" => xberg::EntityCategory::Custom(Default::default())`. ~keep
#[cfg(test)]
mod labeled_string_enum_tests {
    use super::super::{
        gen_flat_data_enum_from_impls, gen_flat_data_enum_methods, is_labeled_string_enum, is_tagged_data_enum,
    };
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
    use ahash::AHashSet;

    fn unit(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A single-field tuple variant (`Custom(String)`): tuple-variant fields are always named
    /// `_0`, `_1`, ... (see `codegen::conversions::is_tuple_variant`).
    fn label_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            // Real tuple variants carry `is_tuple: true` from extraction (this is what
            // `collect_all_variant_constructors` keys its "skip tuple variants" filter on --
            // separate from, but expected to agree with, the field-naming heuristic
            // `is_tuple_variant` derives from `_0`/`_1`/... names).
            is_tuple: true,
            ..Default::default()
        }
    }

    /// Mirrors `EntityCategory`/`PiiCategory`/`OutputFormat`: `#[serde(rename_all = "snake_case")]`
    /// (no `#[serde(tag = ...)]`, not `#[serde(untagged)]`), several unit variants, one `Custom(String)`.
    fn entity_category() -> EnumDef {
        EnumDef {
            name: "EntityCategory".to_string(),
            rust_path: "xberg::EntityCategory".to_string(),
            variants: vec![unit("Person"), unit("Organization"), label_variant("Custom")],
            serde_tag: None,
            serde_untagged: false,
            ..Default::default()
        }
    }

    fn mapper_with(data_enum_names: &[&str]) -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: data_enum_names.iter().map(|s| s.to_string()).collect(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    #[test]
    fn recognises_the_caller_supplied_label_shape() {
        assert!(
            is_labeled_string_enum(&entity_category()),
            "unit variants + one single-field String tuple variant, no serde tag, not untagged"
        );
    }

    #[test]
    fn rejects_an_internally_tagged_enum_already_handled_by_the_struct_variant_path() {
        let mut def = entity_category();
        def.serde_tag = Some("type".to_string());
        assert!(
            !is_labeled_string_enum(&def),
            "serde_tag.is_some() must take the existing path"
        );
    }

    #[test]
    fn rejects_an_untagged_enum_already_handled_by_the_json_value_path() {
        let mut def = entity_category();
        def.serde_untagged = true;
        assert!(
            !is_labeled_string_enum(&def),
            "serde_untagged must take the existing JSON-Value path"
        );
    }

    #[test]
    fn rejects_a_boxed_label_field() {
        let mut def = entity_category();
        def.variants[2].fields[0].is_boxed = true;
        assert!(
            !is_labeled_string_enum(&def),
            "a boxed field is outside the narrow, known-to-compile shape"
        );
    }

    #[test]
    fn rejects_a_multi_field_tuple_variant() {
        let mut def = entity_category();
        def.variants[2].fields.push(FieldDef {
            name: "_1".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        });
        assert!(
            !is_labeled_string_enum(&def),
            "a multi-field tuple variant is outside the narrow shape"
        );
    }

    #[test]
    fn rejects_a_non_string_tuple_field() {
        let mut def = entity_category();
        def.variants[2].fields[0].ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::I64);
        assert!(
            !is_labeled_string_enum(&def),
            "only a bare String payload is in the narrow shape"
        );
    }

    #[test]
    fn rejects_a_struct_variant_with_no_label_variant() {
        // No data variant at all -- nothing for this predicate to lower.
        let def = EnumDef {
            name: "UnitOnly".to_string(),
            rust_path: "xberg::UnitOnly".to_string(),
            variants: vec![unit("A"), unit("B")],
            ..Default::default()
        };
        assert!(!is_labeled_string_enum(&def));
    }

    #[test]
    fn is_tagged_data_enum_now_true_for_a_labeled_string_enum() {
        assert!(
            is_tagged_data_enum(&entity_category()),
            "is_tagged_data_enum must fold in is_labeled_string_enum so every dispatch site \
             (struct-field type mapping, PHPStan stub, constants-vs-flat-class routing) treats \
             it as a flat class"
        );
    }

    /// The core regression check: the generated `From<Binding> for Core` conversion must
    /// construct `Custom(val.custom.unwrap_or_default())` (or equivalent), never
    /// `Custom(Default::default())` -- the literal expression the pre-fix string-enum path always
    /// produced regardless of what the PHP caller supplied.
    #[test]
    fn from_impls_never_default_the_label_away() {
        let def = entity_category();
        let generated = gen_flat_data_enum_from_impls(&def, "xberg", None);

        assert!(
            !generated.contains("Custom(Default::default())"),
            "the label must never be discarded, got:\n{generated}"
        );
        assert!(
            generated.contains("xberg::EntityCategory::Custom(") && generated.contains("val.custom"),
            "binding→core must read the flat class's `custom` field into the tuple variant, got:\n{generated}"
        );
        assert!(
            generated.contains("xberg::EntityCategory::Custom(_0) => Self {")
                || generated.contains("xberg::EntityCategory::Custom(_0)=>Self{"),
            "core→binding must bind the real tuple field to populate `custom`, got:\n{generated}"
        );
        assert!(
            generated.contains("custom: Some(_0.into())") || generated.contains("custom: Some(_0)"),
            "core→binding must carry the label into the flat class's `custom` field, got:\n{generated}"
        );
    }

    /// The usability half of the fix: PHP callers need a way to construct a `Custom` value (there
    /// is no wire tag to build one from besides `from_json`), and unit variants need the same
    /// factories the class constants they replace used to provide directly as values.
    #[test]
    fn variant_constructors_cover_both_unit_and_label_variants() {
        let def = entity_category();
        let mapper = mapper_with(&["EntityCategory"]);
        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&def, &mapper, &empty, &empty, &empty, "xberg", None);

        assert!(
            methods.contains("#[php(name = \"person\")]") && methods.contains("xberg::EntityCategory::Person.into()"),
            "a unit-variant factory must be generated, got:\n{methods}"
        );
        assert!(
            methods.contains("#[php(name = \"custom\")]")
                && methods.contains("pub fn _factory_custom(custom: String) -> Self {"),
            "the label-variant factory must accept the label as a plain String param, got:\n{methods}"
        );
        assert!(
            methods.contains("xberg::EntityCategory::Custom(custom).into()"),
            "the label-variant factory must forward the caller's value into the real core variant, got:\n{methods}"
        );
    }

    /// ext-php-rs 0.16 camel-cases method-backed property names by default. The PHPStan stub and
    /// generated e2e access both expose the flat enum's discriminator and payload under their
    /// snake_case names, so every getter must opt out of that new default explicitly. ~keep
    #[test]
    fn flat_enum_getters_preserve_the_snake_case_properties_declared_to_php_callers() {
        let def = entity_category();
        let mapper = mapper_with(&["EntityCategory"]);
        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&def, &mapper, &empty, &empty, &empty, "xberg", None);

        assert!(
            methods.contains("#[php(getter, change_case = \"snake_case\")]\n    pub fn get_type_tag(&self) -> String"),
            "the discriminator getter must preserve the stub/e2e `$type_tag` property:\n{methods}"
        );
        assert!(
            methods.contains(
                "#[php(getter, change_case = \"snake_case\")]\n    pub fn get_custom(&self) -> Option<String>"
            ),
            "the payload getter must preserve the stub/e2e `$custom` property:\n{methods}"
        );
        assert!(
            !methods.contains("#[php(getter)]"),
            "a bare getter silently camel-cases multi-word properties on ext-php-rs 0.16:\n{methods}"
        );
    }
}

#[cfg(test)]
mod external_enum_serde_tests {
    use super::super::{gen_external_enum_serde_impls, gen_flat_data_enum};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
    use ahash::AHashSet;

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::from_iter(["OutputFormat".to_string()]),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    /// `xberg::OutputFormat`: externally tagged, `rename_all = "lowercase"`, unit variants plus one
    /// `#[serde(untagged)] Custom(String)`.
    fn output_format() -> EnumDef {
        EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "crate::OutputFormat".to_string(),
            serde_rename_all: Some("lowercase".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Plain".to_string(),
                    is_default: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Markdown".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    is_tuple: true,
                    serde_untagged: true,
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Same shape without the `#[serde(untagged)]` marker: serde writes the data variant as a
    /// single-keyed object whose value IS the payload, and an unknown bare string is an error.
    fn keyed_label_enum() -> EnumDef {
        let mut def = output_format();
        def.name = "EntityCategory".to_string();
        def.rust_path = "crate::EntityCategory".to_string();
        def.variants[2].serde_untagged = false;
        def
    }

    #[test]
    fn labeled_string_enum_struct_drops_the_derived_serde_impls() {
        let generated = gen_flat_data_enum(&output_format(), &mapper(), None);
        assert!(
            generated.contains("#[derive(Clone, Default)]") && !generated.contains("#[derive(Clone, Default, serde::"),
            "the derived impl writes {{\"type\":\"markdown\"}}, which is not serde's external wire; got:\n{generated}"
        );
        assert!(
            !generated.contains("#[serde("),
            "field-level serde attributes do not compile without a serde derive on the struct; got:\n{generated}"
        );
        assert!(
            generated.contains("type_tag: String"),
            "the tag field itself stays, generated conversions read it; got:\n{generated}"
        );
    }

    #[test]
    fn unit_variant_round_trips_as_a_bare_string() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#""markdown" => serializer.serialize_str("markdown")"#),
            "{generated}"
        );
        assert!(
            generated.contains(r#""markdown" => Ok(OutputFormat {"#),
            "visit_str must accept the bare tag serde writes; got:\n{generated}"
        );
    }

    #[test]
    fn untagged_variant_round_trips_as_its_bare_payload() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#""custom" => serializer.serialize_str(self.custom.as_deref().unwrap_or_default())"#),
            "an untagged variant writes its payload with no tag wrapper; got:\n{generated}"
        );
        assert!(
            generated.contains("custom: Some(value.to_string())"),
            "an unrecognised bare string IS the untagged variant's payload, not an error; got:\n{generated}"
        );
        assert!(
            generated.contains("value => Ok(OutputFormat {"),
            "with an untagged fallback the catch-all constructs it rather than erroring; got:\n{generated}"
        );
    }

    #[test]
    fn keyed_label_variant_round_trips_as_a_single_keyed_object() {
        let generated = gen_external_enum_serde_impls(&keyed_label_enum());
        assert!(
            generated.contains(r#"map.serialize_entry("custom", self.custom.as_deref().unwrap_or_default())"#),
            "a tagged newtype variant writes {{\"custom\": payload}}; got:\n{generated}"
        );
        assert!(
            generated.contains(
                r#"value => Err(serde::de::Error::unknown_variant(value, &["plain", "markdown", "custom"]))"#
            ),
            "without an untagged fallback an unrecognised bare string is an error; got:\n{generated}"
        );
    }

    #[test]
    fn the_empty_default_tag_round_trips_rather_than_becoming_the_untagged_payload() {
        let generated = gen_external_enum_serde_impls(&output_format());
        let empty_arm = generated
            .find(r#""" => Ok(OutputFormat::default())"#)
            .unwrap_or_else(|| panic!("no empty-tag arm; got:\n{generated}"));
        let untagged_arm = generated
            .find("value => Ok(OutputFormat {")
            .unwrap_or_else(|| panic!("no untagged catch-all; got:\n{generated}"));
        assert!(
            empty_arm < untagged_arm,
            "`Default::default()` leaves the tag empty and serializes as a bare `\"\"`; the \
             untagged catch-all would otherwise read it back as Custom(\"\") and break the \
             round trip; got:\n{generated}"
        );
    }

    #[test]
    fn constructed_values_list_every_field_rather_than_using_update_syntax() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            !generated.contains("..Default::default()"),
            "`..Default::default()` on an arm that already sets every field trips \
             clippy::needless_update, and the generated crate compiles under -D warnings; got:\n{generated}"
        );
        assert!(
            generated.contains(r#""markdown" => Ok(OutputFormat { type_tag: "markdown".to_string(), custom: None })"#),
            "a unit variant must still null the label field explicitly; got:\n{generated}"
        );
    }

    #[test]
    fn map_form_still_reads_the_classes_own_flat_shape() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#"entries.iter().any(|(key, _)| key == "type")"#),
            "the flat {{\"type\":..}} form this class used to emit must keep deserialising; got:\n{generated}"
        );
    }
}
