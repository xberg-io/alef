//! The narrowing map must name exactly the accessors the C# backend generates — no more, no
//! fewer. Fixture names are deliberately neutral per `project-agnostic-codegen`; the shape they
//! stand in for is a tagged union whose variants each wrap one payload struct.

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn payload_field(type_name: &str) -> FieldDef {
    FieldDef {
        name: "_0".to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        ..FieldDef::default()
    }
}

/// A union with one accessible variant, one binding-excluded variant, and one variant carrying
/// two fields — only the first earns an `As<Variant>` property.
fn sample_union() -> EnumDef {
    EnumDef {
        name: "SamplePayload".to_string(),
        variants: vec![
            EnumVariant {
                name: "Alpha".to_string(),
                fields: vec![payload_field("AlphaDetails")],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Beta".to_string(),
                fields: vec![payload_field("BetaDetails")],
                binding_excluded: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Gamma".to_string(),
                fields: vec![payload_field("GammaDetails"), payload_field("GammaExtra")],
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
fn should_name_the_accessor_the_backend_generates_for_an_accessible_variant() {
    let map = super::build_variant_accessor_map(&[sample_union()]);

    assert_eq!(
        map.narrowing_for("SamplePayload", "Alpha"),
        Some("AsAlpha"),
        "an accessible single-payload variant must narrow through the generated As<Variant>"
    );
}

#[test]
fn should_omit_every_variant_the_backend_generates_no_accessor_for() {
    let map = super::build_variant_accessor_map(&[sample_union()]);

    for variant in ["Beta", "Gamma", "Delta"] {
        assert_eq!(
            map.narrowing_for("SamplePayload", variant),
            None,
            "{variant} gets no generated accessor, so offering one would name a member that \
             does not exist"
        );
    }
    assert_eq!(map.narrowing.len(), 1, "only Alpha is accessible");
}

/// The check that matters: the map and the generator must not be able to disagree. Both read
/// `variant_accessor_properties`, and this pins that they stay in agreement for every variant
/// rather than only for the ones the fixtures above happen to cover.
#[test]
fn should_agree_exactly_with_the_backends_own_accessor_set() {
    let union = sample_union();
    let generated: Vec<String> = crate::backends::csharp::gen_bindings::variant_accessor_properties(&union)
        .into_iter()
        .map(|(pascal, _)| format!("As{pascal}"))
        .collect();

    let mut mapped: Vec<String> = super::build_variant_accessor_map(std::slice::from_ref(&union))
        .narrowing
        .into_values()
        .collect();
    mapped.sort();
    let mut generated_sorted = generated;
    generated_sorted.sort();

    assert_eq!(
        mapped, generated_sorted,
        "the resolver's accessor set must equal the generator's, or a snippet can reference a \
         property the binding never emitted"
    );
}

#[test]
fn should_be_empty_for_a_crate_with_no_unions() {
    let map = super::build_variant_accessor_map(&[]);

    assert!(
        map.is_empty(),
        "an empty map is what makes every non-union path render exactly as it did before"
    );
}
