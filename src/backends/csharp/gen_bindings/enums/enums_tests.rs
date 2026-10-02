//! Unit tests for struct-shaped tagged-union variant field wire naming.
//!
//! A struct-shaped enum variant's field names live in a namespace distinct from the enum's own
//! `serde_rename_all` (which cases variant names, not struct-variant field names). The wire name
//! precedence is: field `serde_rename` > enum `rename_all_fields` (the struct-variant field
//! casing container rule) > raw field name. These tests pin each half of that precedence chain
//! independently, then together, and pin that an unrenamed field under a rule-less enum still
//! round-trips its raw name.
use super::*;
use crate::core::ir::{EnumVariant, FieldDef, TypeRef};

fn named_field(name: &str, serde_rename: Option<&str>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        serde_rename: serde_rename.map(str::to_string),
        ..FieldDef::default()
    }
}

fn struct_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        ..EnumVariant::default()
    }
}

fn tagged_union_enum(variants: Vec<EnumVariant>, rename_all_fields: Option<&str>) -> EnumDef {
    EnumDef {
        name: "SampleUnion".to_string(),
        serde_tag: Some("type".to_string()),
        variants,
        rename_all_fields: rename_all_fields.map(str::to_string),
        ..EnumDef::default()
    }
}

#[test]
fn externally_tagged_mixed_enum_accepts_string_units_and_object_payloads() {
    let mut enum_def = tagged_union_enum(
        vec![
            struct_variant("Email", vec![]),
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
        ],
        None,
    );
    enum_def.serde_tag = None;
    let emitted = gen_enum(&enum_def, "Sample.Namespace", &[]);
    assert!(emitted.contains("abstract record SampleUnion"));
    assert!(emitted.contains("reader.TokenType == JsonTokenType.String"));
    assert!(emitted.contains("var tagValue = property.Name"));
    assert!(emitted.contains("writer.WriteStringValue(tag)"));
    assert!(emitted.contains("writer.WritePropertyName(tag)"));
    assert!(emitted.contains("Externally tagged unit variants must be strings"));
}

#[test]
fn externally_tagged_enum_with_untagged_string_fallback_round_trips_bare_strings() {
    let enum_def = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![
            EnumVariant {
                name: "Plain".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                serde_untagged: true,
                ..EnumVariant::default()
            },
        ],
        serde_rename_all: Some("lowercase".into()),
        ..EnumDef::default()
    };

    let emitted = gen_enum(&enum_def, "Sample.Namespace", &[]);

    assert!(emitted.contains("\"plain\" => new OutputFormat.Plain()"));
    assert!(emitted.contains("_ => new OutputFormat.Custom(unitTag"));
    assert!(emitted.contains("if (value is OutputFormat.Custom untaggedCustom)"));
    assert!(emitted.contains("writer.WriteStringValue(untaggedCustom.Value)"));
    assert!(emitted.contains("Untagged variant must be a string"));
}

#[test]
fn externally_tagged_complex_payloads_stay_on_the_existing_plain_enum_path() {
    let cases = [
        vec![FieldDef {
            name: "_0".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            ..FieldDef::default()
        }],
        vec![
            FieldDef {
                name: "_0".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
            FieldDef {
                name: "_1".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
        ],
    ];
    for fields in cases {
        let mut enum_def = tagged_union_enum(
            vec![
                struct_variant("Email", vec![]),
                EnumVariant {
                    name: "Custom".into(),
                    fields,
                    ..EnumVariant::default()
                },
            ],
            None,
        );
        enum_def.serde_tag = None;
        let emitted = gen_enum(&enum_def, "Sample.Namespace", &[]);
        assert!(!emitted.contains("abstract record SampleUnion"));
    }

    let mut excluded = tagged_union_enum(
        vec![
            struct_variant("Email", vec![]),
            EnumVariant {
                name: "Custom".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                binding_excluded: true,
                ..EnumVariant::default()
            },
        ],
        None,
    );
    excluded.serde_tag = None;
    assert!(!gen_enum(&excluded, "Sample.Namespace", &[]).contains("abstract record SampleUnion"));
}

#[test]
fn struct_variant_field_with_serde_rename_uses_the_renamed_wire_name() {
    let enum_def = tagged_union_enum(
        vec![struct_variant(
            "Explicit",
            vec![named_field("resource_url", Some("resourceUrl"))],
        )],
        None,
    );

    let emitted = gen_tagged_union(&enum_def, "Sample.Namespace");

    assert!(
        emitted.contains(r#"JsonPropertyName("resourceUrl")"#),
        "an explicit serde_rename on the field must drive the wire name:\n{emitted}"
    );
    assert!(
        !emitted.contains(r#"JsonPropertyName("resource_url")"#),
        "the raw Rust field name must not leak onto the wire once serde_rename overrides it:\n{emitted}"
    );
}

#[test]
fn struct_variant_field_with_container_rename_all_fields_uses_the_cased_wire_name() {
    let enum_def = tagged_union_enum(
        vec![struct_variant("Explicit", vec![named_field("display_name", None)])],
        Some("camelCase"),
    );

    let emitted = gen_tagged_union(&enum_def, "Sample.Namespace");

    assert!(
        emitted.contains(r#"JsonPropertyName("displayName")"#),
        "the enum's rename_all_fields container rule must case a field with no explicit rename:\n{emitted}"
    );
    assert!(
        !emitted.contains(r#"JsonPropertyName("display_name")"#),
        "the raw Rust field name must not leak onto the wire once rename_all_fields applies:\n{emitted}"
    );
}

#[test]
fn struct_variant_field_explicit_rename_wins_over_container_rename_all_fields() {
    let enum_def = tagged_union_enum(
        vec![struct_variant(
            "Explicit",
            vec![named_field("display_name", Some("explicitName"))],
        )],
        Some("camelCase"),
    );

    let emitted = gen_tagged_union(&enum_def, "Sample.Namespace");

    assert!(
        emitted.contains(r#"JsonPropertyName("explicitName")"#),
        "an explicit serde_rename must win over the enum's rename_all_fields container rule:\n{emitted}"
    );
    assert!(
        !emitted.contains(r#"JsonPropertyName("displayName")"#),
        "the container-cased name must not be used when the field carries its own serde_rename:\n{emitted}"
    );
    assert!(
        !emitted.contains(r#"JsonPropertyName("display_name")"#),
        "the raw Rust field name must not leak onto the wire when either rule applies:\n{emitted}"
    );
    assert!(
        emitted.contains(r#")] string DisplayName"#),
        "the public C# property identifier must stay derived from the raw field name, unaffected \
         by either wire-naming rule:\n{emitted}"
    );
}

#[test]
fn struct_variant_field_without_any_rename_keeps_its_raw_name() {
    let enum_def = tagged_union_enum(
        vec![struct_variant("Explicit", vec![named_field("resource_url", None)])],
        None,
    );

    let emitted = gen_tagged_union(&enum_def, "Sample.Namespace");

    assert!(
        emitted.contains(r#"JsonPropertyName("resource_url")"#),
        "a field with neither serde_rename nor an enum rename_all_fields must keep its raw name:\n{emitted}"
    );
}
