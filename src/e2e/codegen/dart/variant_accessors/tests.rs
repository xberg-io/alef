//! The Dart narrowing must spell the cast exactly as the assertion emitter does, and must
//! decline every variant that has no single payload to step into. Neutral fixture names per
//! `project-agnostic-codegen`.

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn tuple_field(name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        ..FieldDef::default()
    }
}

fn sample_union() -> EnumDef {
    EnumDef {
        name: "SamplePayload".to_string(),
        variants: vec![
            EnumVariant {
                name: "Alpha".to_string(),
                fields: vec![tuple_field("_0", "AlphaDetails")],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Beta".to_string(),
                fields: vec![tuple_field("_0", "BetaDetails")],
                binding_excluded: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Gamma".to_string(),
                fields: vec![tuple_field("_0", "G1"), tuple_field("_1", "G2")],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Delta".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn should_cast_to_the_freezed_subclass_and_read_the_tuple_payload() {
    let map = super::build_variant_accessor_map(&[sample_union()]);

    assert_eq!(
        map.narrowing_for("SamplePayload", "Alpha"),
        Some("SamplePayload_Alpha"),
        "the cast target is frb's <Union>_<Variant> subclass"
    );
    assert_eq!(
        map.payload_for("SamplePayload", "Alpha"),
        Some("field0"),
        "a `_0` tuple field is exposed as `field0`, per dart_tuple_field_identifier"
    );
}

#[test]
fn should_decline_variants_with_no_single_payload_or_no_binding() {
    let map = super::build_variant_accessor_map(&[sample_union()]);

    for variant in ["Beta", "Gamma", "Delta"] {
        assert_eq!(
            map.narrowing_for("SamplePayload", variant),
            None,
            "{variant} has no single reachable payload, so there is nothing to narrow into"
        );
    }
    assert_eq!(map.narrowing.len(), 1, "only Alpha is narrowable");
}

#[test]
fn should_be_empty_for_a_crate_with_no_unions() {
    assert!(
        super::build_variant_accessor_map(&[]).is_empty(),
        "an empty map is what leaves every non-union Dart path rendering as it did before"
    );
}
