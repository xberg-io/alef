//! Regression coverage for cfg-gated struct field filtering in constructor params and getter declarations.
//!
//! Fields whose `#[cfg(...)]` condition is not satisfied by the configured feature set must be dropped from
//! the extern block entirely, otherwise swift-bridge will panic when trying to reference a type that does not
//! exist, such as a feature-gated confidence score type.

use super::{constructor_fields, has_constructor_extern, is_unbridgeable_getter};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn test_type_with_mixed_cfg_fields() -> TypeDef {
    TypeDef {
        name: "ExtractionResult".to_string(),
        rust_path: "sample_crate::ExtractionResult".to_string(),
        fields: vec![
            FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                optional: false,
                cfg: None,
                ..Default::default()
            },
            FieldDef {
                name: "extraction_confidence".to_string(),
                ty: TypeRef::Named("ExtractionConfidence".to_string()),
                optional: true,
                cfg: Some("feature = \"heuristics\"".to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn constructor_fields_filters_cfg_gated_fields() {
    let ty = test_type_with_mixed_cfg_fields();
    let exclude_fields = HashSet::new();
    let empty_features = HashSet::new();
    let with_heuristics: HashSet<&str> = ["heuristics"].into_iter().collect();

    let fields_without = constructor_fields(&ty, &exclude_fields, &empty_features);
    assert_eq!(
        fields_without.len(),
        1,
        "without heuristics feature, only uncfg-gated field should appear"
    );
    assert_eq!(
        fields_without[0].name, "text",
        "uncfg-gated 'text' field should be present"
    );

    let fields_with = constructor_fields(&ty, &exclude_fields, &with_heuristics);
    assert_eq!(
        fields_with.len(),
        2,
        "with heuristics feature, both fields should appear"
    );
    assert_eq!(fields_with[0].name, "text");
    assert_eq!(fields_with[1].name, "extraction_confidence");
    assert!(
        !has_constructor_extern(&ty, &exclude_fields, &empty_features),
        "non-primitive serde DTOs still require Default-based construction"
    );
}

#[test]
fn is_unbridgeable_getter_returns_true_for_cfg_gated_fields() {
    let ty = test_type_with_mixed_cfg_fields();
    let exclude_fields = HashSet::new();
    let empty_features = HashSet::new();
    let with_heuristics: HashSet<&str> = ["heuristics"].into_iter().collect();
    let mut type_paths = HashMap::new();
    type_paths.insert(
        "ExtractionConfidence".to_string(),
        "sample_crate::ExtractionConfidence".to_string(),
    );
    let no_serde_names = HashSet::new();

    let cfg_field = &ty.fields[1];
    assert_eq!(cfg_field.name, "extraction_confidence");

    let unbridgeable_without = is_unbridgeable_getter(
        &ty,
        cfg_field,
        &exclude_fields,
        &type_paths,
        &no_serde_names,
        &empty_features,
    );
    assert!(
        unbridgeable_without,
        "cfg-gated field should be unbridgeable when feature is not configured"
    );

    let unbridgeable_with = is_unbridgeable_getter(
        &ty,
        cfg_field,
        &exclude_fields,
        &type_paths,
        &no_serde_names,
        &with_heuristics,
    );
    assert!(
        !unbridgeable_with,
        "cfg-gated field should be bridgeable when feature is configured and type path exists"
    );

    let uncfg_field = &ty.fields[0];
    let unbridgeable_uncfg = is_unbridgeable_getter(
        &ty,
        uncfg_field,
        &exclude_fields,
        &type_paths,
        &no_serde_names,
        &empty_features,
    );
    assert!(
        !unbridgeable_uncfg,
        "uncfg-gated String field should always be bridgeable"
    );
}
