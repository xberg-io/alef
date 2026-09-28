//! Coverage for [`FieldResolver::is_internally_tagged_variant_segment`] — the IR-derived
//! replacement for a hand-maintained list of one consumer's `FormatMetadata` variant names
//! anchored on the literal field name `format`.
//!
//! The generalisation is the point: every test here uses an enum named nothing like
//! `FormatMetadata`, reached through a field named nothing like `format`, with variant names
//! that appear nowhere in that superseded list. All of them fail against a hardcoded list and
//! pass against a walk of the crate's own IR.

use super::types::TaggedEnumWire;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::{FieldResolver, IrEnumMap, JsonNavStep, SwiftFirstClassMap};
use std::collections::{HashMap, HashSet};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn newtype_variant(name: &str, payload: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        is_tuple: true,
        fields: vec![field("_0", TypeRef::Named(payload.to_string()))],
        ..EnumVariant::default()
    }
}

fn struct_of(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        has_serde: true,
        ..TypeDef::default()
    }
}

/// `Envelope { payload: EventPayload }` where `EventPayload` is internally tagged on `kind` and
/// renames its variants `SCREAMING_SNAKE_CASE`.
///
/// Deliberately shares nothing with the superseded hard-coded knowledge: the field is `payload`,
/// not `format`; the tag is `kind`, not `format_type`; and `HEART_BEAT`/`TEXT_MESSAGE` are not
/// among the 21 `FormatMetadata` variant names. The rename strategy is also NOT `snake_case`, so
/// the wire value and the Rust identifier genuinely differ and a hand-rolled case transform
/// would get one of them wrong. ~keep
fn event_payload_ir(tag: Option<&str>, content: Option<&str>) -> (Vec<TypeDef>, Vec<EnumDef>) {
    let type_defs = vec![
        struct_of(
            "Envelope",
            vec![field("payload", TypeRef::Named("EventPayload".to_string()))],
        ),
        struct_of("HeartBeatPayload", vec![field("uptime_secs", TypeRef::String)]),
        struct_of("TextMessagePayload", vec![field("body", TypeRef::String)]),
    ];
    let enums = vec![EnumDef {
        name: "EventPayload".to_string(),
        has_serde: true,
        serde_tag: tag.map(str::to_string),
        serde_content: content.map(str::to_string),
        serde_rename_all: Some("SCREAMING_SNAKE_CASE".to_string()),
        variants: vec![
            newtype_variant("HeartBeat", "HeartBeatPayload"),
            newtype_variant("TextMessage", "TextMessagePayload"),
        ],
        ..EnumDef::default()
    }];
    (type_defs, enums)
}

fn envelope_resolver(tag: Option<&str>, content: Option<&str>, root_type: Option<&str>) -> FieldResolver {
    let (type_defs, enums) = event_payload_ir(tag, content);
    let empty = HashSet::new();
    FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty).with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs, &enums),
        root_type.map(str::to_string),
    )
}

#[test]
fn should_skip_a_variant_segment_of_an_internally_tagged_enum_under_any_field_name() {
    let resolver = envelope_resolver(Some("kind"), None, Some("Envelope"));

    assert!(
        resolver.is_internally_tagged_variant_segment("payload", "HEART_BEAT"),
        "the wire spelling of EventPayload::HeartBeat names a variant, not a JSON key"
    );
    assert!(
        resolver.is_internally_tagged_variant_segment("payload", "text_message"),
        "the accessor spelling must resolve too — `ir_tagged_union_split` reads a fixture \
         variant segment by upper-camel-casing it"
    );
}

#[test]
fn should_keep_an_ordinary_payload_field_as_a_real_key() {
    let resolver = envelope_resolver(Some("kind"), None, Some("Envelope"));

    assert!(
        !resolver.is_internally_tagged_variant_segment("payload", "uptime_secs"),
        "a field of the flattened payload IS a wire key and must not be skipped"
    );
}

#[test]
fn should_not_skip_when_the_enum_is_adjacently_tagged() {
    let resolver = envelope_resolver(Some("kind"), Some("data"), Some("Envelope"));

    assert!(
        !resolver.is_internally_tagged_variant_segment("payload", "HEART_BEAT"),
        "adjacent tagging nests the payload under the content key, so the variant segment is \
         not flattened away and skipping it would address the wrong node"
    );
}

#[test]
fn should_not_skip_when_the_enum_is_externally_tagged() {
    let resolver = envelope_resolver(None, None, Some("Envelope"));

    assert!(
        !resolver.is_internally_tagged_variant_segment("payload", "HEART_BEAT"),
        "external tagging keeps the variant name as a real JSON key — that is the one shape \
         where the segment must survive verbatim"
    );
}

#[test]
fn should_not_skip_a_variant_name_reached_through_an_unrelated_field() {
    let resolver = envelope_resolver(Some("kind"), None, Some("Envelope"));

    assert!(
        !resolver.is_internally_tagged_variant_segment("uptime_secs", "HEART_BEAT"),
        "the verdict is anchored on the preceding field's TYPE; a name that is a variant \
         somewhere else in the crate must not skip a segment here"
    );
}

#[test]
fn should_not_skip_when_the_call_result_type_could_not_be_anchored() {
    let resolver = envelope_resolver(Some("kind"), None, None);

    assert!(
        !resolver.is_internally_tagged_variant_segment("payload", "HEART_BEAT"),
        "with no anchored root the IR cannot confirm anything, and an unconfirmed segment \
         stays the literal key lookup it was before this existed"
    );
}

#[test]
fn should_not_skip_the_first_segment_of_a_path() {
    let resolver = envelope_resolver(Some("kind"), None, Some("Envelope"));

    assert!(
        !resolver.is_internally_tagged_variant_segment("", "HEART_BEAT"),
        "nothing precedes the first segment, so there is no field whose type could make it a \
         variant"
    );
}

/// End-to-end through the Swift JSON-navigation call site: the skipped segment must not become a
/// [`JsonNavStep::Key`], for an enum the superseded hard-coded list never heard of.
#[test]
fn swift_json_navigation_skips_a_generalised_variant_segment() {
    let (type_defs, enums) = event_payload_ir(Some("kind"), None);
    let swift_first_class_map = SwiftFirstClassMap {
        json_bridged_field_names: HashSet::from(["payload".to_string()]),
        ..SwiftFirstClassMap::default()
    };
    let resolver = FieldResolver::new_with_error_aliases(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
    )
    .with_swift_first_class_map(swift_first_class_map)
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs, &enums),
        Some("Envelope".to_string()),
    );

    let (leaf_field, steps) = resolver
        .swift_json_bridged_navigation("payload.heart_beat.uptime_secs")
        .expect("payload is JSON-bridged, so the path steps past it and must be navigable");

    assert_eq!(leaf_field, "payload");
    assert_eq!(steps, vec![JsonNavStep::Key("uptime_secs".to_string())]);
    assert!(
        !steps.contains(&JsonNavStep::Key("heart_beat".to_string())),
        "the variant segment has no JSON key on an internally tagged wire form, got: {steps:?}"
    );
}

/// CONTROL for the same call site: flip only the enum's representation to adjacent tagging and
/// the very same path must keep its variant segment.
#[test]
fn swift_json_navigation_keeps_the_variant_segment_when_tagging_is_adjacent() {
    let (type_defs, enums) = event_payload_ir(Some("kind"), Some("data"));
    let swift_first_class_map = SwiftFirstClassMap {
        json_bridged_field_names: HashSet::from(["payload".to_string()]),
        ..SwiftFirstClassMap::default()
    };
    let resolver = FieldResolver::new_with_error_aliases(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
    )
    .with_swift_first_class_map(swift_first_class_map)
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs, &enums),
        Some("Envelope".to_string()),
    );

    let (_, steps) = resolver
        .swift_json_bridged_navigation("payload.heart_beat.uptime_secs")
        .expect("payload is JSON-bridged, so the path steps past it and must be navigable");

    assert_eq!(
        steps,
        vec![
            JsonNavStep::Key("heart_beat".to_string()),
            JsonNavStep::Key("uptime_secs".to_string()),
        ]
    );
}

/// The exact shape the superseded list existed for, now answered from the IR: a `snake_case`
/// internally tagged enum whose accessor and wire spellings coincide.
#[test]
fn should_still_skip_the_snake_case_variant_shape_the_hard_coded_list_covered() {
    let type_defs = vec![
        struct_of(
            "ExtractedDocument",
            vec![field("metadata", TypeRef::Named("Metadata".to_string()))],
        ),
        struct_of(
            "Metadata",
            vec![field(
                "format",
                TypeRef::Optional(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
            )],
        ),
        struct_of("ExcelMetadata", vec![field("sheet_count", TypeRef::String)]),
    ];
    let enums = vec![EnumDef {
        name: "FormatMetadata".to_string(),
        has_serde: true,
        serde_tag: Some("format_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![newtype_variant("Excel", "ExcelMetadata")],
        ..EnumDef::default()
    }];
    let empty = HashSet::new();
    let resolver = FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty).with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs, &enums),
        Some("ExtractedDocument".to_string()),
    );

    assert!(resolver.is_internally_tagged_variant_segment("metadata.format", "excel"));
    assert!(
        !resolver.is_internally_tagged_variant_segment("metadata.format", "sheet_count"),
        "the payload's own field is a real wire key"
    );
}

/// A hand-built [`IrEnumMap`] is still a legal input (the type is public and
/// exhaustively constructible), so the predicate must read the two facts it needs off it
/// without depending on `build_ir_enum_map` having run. ~keep
#[test]
fn should_read_the_wire_facts_off_a_hand_built_ir_enum_map() {
    let map = IrEnumMap {
        enum_field_types: HashMap::from([(
            "Envelope".to_string(),
            HashMap::from([("payload".to_string(), "EventPayload".to_string())]),
        )]),
        tagged_enum_wire: HashMap::from([(
            "EventPayload".to_string(),
            TaggedEnumWire {
                tag: "kind".to_string(),
                variants: HashMap::from([("HeartBeat".to_string(), "HEART_BEAT".to_string())]),
                content: None,
            },
        )]),
        ..IrEnumMap::default()
    };
    let empty = HashSet::new();
    let resolver = FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &empty)
        .with_ir_enum_map(map, Some("Envelope".to_string()));

    assert!(resolver.is_internally_tagged_variant_segment("payload", "HEART_BEAT"));
}
