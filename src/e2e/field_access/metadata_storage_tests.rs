use super::*;
use std::collections::{HashMap, HashSet};

pub(crate) fn assert_shared_maps(first: &FieldResolver, second: &FieldResolver) {
    assert!(std::ptr::eq(
        &first.ir_enum_map.field_types,
        &second.ir_enum_map.field_types
    ));
    assert!(std::ptr::eq(
        &first.ir_collection_map.field_types,
        &second.ir_collection_map.field_types
    ));
    assert!(std::ptr::eq(
        &first.ir_result_field_map.optional_fields,
        &second.ir_result_field_map.optional_fields
    ));
}

pub(crate) fn assert_shared_custom_maps(first: &FieldResolver, second: &FieldResolver) {
    assert_eq!(first.php_getter_map.field_types.len(), 1);
    assert_eq!(first.swift_first_class_map.field_types.len(), 1);
    assert!(std::ptr::eq(
        &first.php_getter_map.field_types,
        &second.php_getter_map.field_types
    ));
    assert!(std::ptr::eq(
        &first.swift_first_class_map.field_types,
        &second.swift_first_class_map.field_types
    ));
}

pub(crate) fn assert_shared_variant_maps(first: &FieldResolver, second: &FieldResolver) {
    assert_eq!(first.variant_accessors.narrowing.len(), 1);
    assert!(std::ptr::eq(
        &first.variant_accessors.narrowing,
        &second.variant_accessors.narrowing
    ));
    assert!(std::ptr::eq(
        &first.variant_accessors.payload,
        &second.variant_accessors.payload
    ));
}

pub(crate) fn assert_variant_map_contents(resolver: &FieldResolver, narrowing: &str, payload: Option<&str>) {
    assert_eq!(
        resolver.variant_accessors.narrowing_for("Format", "Html"),
        Some(narrowing)
    );
    assert_eq!(resolver.variant_accessors.payload_for("Format", "Html"), payload);
    assert_eq!(resolver.variant_accessors.narrowing.len(), 1);
    assert_eq!(resolver.variant_accessors.payload.len(), usize::from(payload.is_some()));
}

fn variant_sharing_control(narrowing: &str, payload: Option<&str>) {
    let key = ("Format".to_owned(), "Html".to_owned());
    let map = VariantAccessorMap {
        narrowing: [(key.clone(), narrowing.to_owned())].into(),
        payload: payload
            .map(|payload| [(key, payload.to_owned())].into())
            .unwrap_or_default(),
    };
    let original = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_variant_accessors(map);
    let copied = original.clone().with_variant_accessors(VariantAccessorMap {
        narrowing: original.variant_accessors.narrowing.clone(),
        payload: original.variant_accessors.payload.clone(),
    });
    assert_eq!(
        copied.variant_accessors.narrowing_for("Format", "Html"),
        Some(narrowing)
    );
    assert_eq!(copied.variant_accessors.payload_for("Format", "Html"), payload);
    assert!(std::panic::catch_unwind(|| assert_shared_variant_maps(&original, &copied)).is_err());
    let enriched = original.clone().with_anchored_optional_paths(["title"]);
    assert_eq!(
        enriched.variant_accessors.narrowing_for("Format", "Html"),
        Some(narrowing)
    );
    assert_eq!(enriched.variant_accessors.payload_for("Format", "Html"), payload);
    assert_eq!(enriched.variant_accessors.payload.len(), usize::from(payload.is_some()));
    assert_shared_variant_maps(&original, &enriched);
}

#[test]
fn variant_cache_shares_csharp_narrowing_storage_during_optional_enrichment() {
    variant_sharing_control("AsHtml", None);
}

#[test]
fn variant_cache_shares_dart_narrowing_and_payload_storage_during_optional_enrichment() {
    variant_sharing_control("Format_Html", Some("field0"));
}

pub(crate) fn clone_cached_maps_into_owned_storage(original: &FieldResolver) -> FieldResolver {
    original
        .clone()
        .with_ir_enum_map((*original.ir_enum_map).clone(), original.ir_enum_map.root_type.clone())
        .with_ir_collection_map(
            (*original.ir_collection_map).clone(),
            original.ir_collection_map.root_type.clone(),
        )
        .with_ir_result_fields(
            (*original.ir_result_field_map).clone(),
            original.ir_result_field_map.root_type.clone(),
        )
}

fn resolver() -> FieldResolver {
    let mut enums = IrEnumMap::default();
    enums.field_types.insert("Root".into(), HashMap::new());
    let mut collections = IrCollectionMap::default();
    collections.field_types.insert("Root".into(), HashMap::new());
    let mut fields = IrResultFieldMap::default();
    fields
        .optional_fields
        .insert("Root".into(), HashSet::from(["title".into()]));
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["configured".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(
        HashSet::from(["title".into()]),
        HashSet::new(),
        HashSet::from(["ir_optional".into()]),
    )
    .with_ir_enum_map(enums, Some("Root".into()))
    .with_ir_collection_map(collections, Some("Root".into()))
    .with_ir_result_fields(fields, Some("Root".into()))
}

#[test]
fn optional_enrichment_keeps_the_same_immutable_ir_maps() {
    let original = resolver();
    let enriched = original.clone().with_anchored_optional_paths(["title"]);
    assert_eq!(original.ir_enum_map.field_types.len(), 1);
    assert_eq!(original.ir_collection_map.field_types.len(), 1);
    assert_eq!(original.ir_result_field_map.optional_fields.len(), 1);
    assert!(std::ptr::eq(
        &original.ir_enum_map.field_types,
        &enriched.ir_enum_map.field_types
    ));
    assert!(std::ptr::eq(
        &original.ir_collection_map.field_types,
        &enriched.ir_collection_map.field_types
    ));
    assert!(std::ptr::eq(
        &original.ir_result_field_map.optional_fields,
        &enriched.ir_result_field_map.optional_fields
    ));
    assert!(!original.optional_fields.contains("title"));
    assert!(enriched.optional_fields.contains("title"));
}

#[test]
fn optional_enrichment_shares_ir_names_without_mutating_the_source_overlay() {
    let original = resolver();
    let enriched = original.clone().with_anchored_optional_paths(["title"]);
    let before = original
        .optional_fields
        .iter()
        .find(|name| name.as_str() == "ir_optional")
        .unwrap();
    let after = enriched
        .optional_fields
        .iter()
        .find(|name| name.as_str() == "ir_optional")
        .unwrap();
    assert!(
        std::ptr::eq(before, after),
        "IR optional names must not be deep-cloned per snippet"
    );
    assert!(original.is_optional("ir_optional"));
    assert!(enriched.is_optional("configured"));
    assert!(!original.optional_fields.contains("title"));
    assert!(enriched.optional_fields.contains("title"));
    assert!(std::ptr::eq(
        original.ir_reachable_fields.iter().next().unwrap(),
        enriched.ir_reachable_fields.iter().next().unwrap()
    ));
}
