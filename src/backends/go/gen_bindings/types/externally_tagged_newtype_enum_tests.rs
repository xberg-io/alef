use super::enums::{GoEnumRepresentation, gen_enum_type, go_enum_representation};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn pii_category_enum() -> EnumDef {
    EnumDef {
        name: "PiiCategory".to_string(),
        rust_path: "xberg::PiiCategory".to_string(),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Email".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn externally_tagged_unit_and_string_enum_preserves_object_variant() {
    let enum_def = pii_category_enum();

    assert_eq!(go_enum_representation(&enum_def), GoEnumRepresentation::RawMessage);

    let generated = gen_enum_type(&enum_def, &[]);
    assert!(
        generated.contains("type PiiCategory json.RawMessage"),
        "the Go type must accept both serde wire shapes: {generated}"
    );
}
