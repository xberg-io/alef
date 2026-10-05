//! Regression coverage for `null` values on optional DTO fields in the Go documentation snippet.
//!
//! A fixture writes `"content": null` for an absent optional field. Lowering it against a field
//! whose type is a raw-message enum produced `pkg.Enum(nil)`, which `go_named_scalar_expression`
//! refuses as a `cannot convert` error, so a fixture that renders fine with the field simply
//! absent became an undocumented coverage gap the moment its enclosing array was rendered. ~keep

use super::array_field_snippet_tests::{field, render, target_type};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

/// `type SampleInput json.RawMessage` -- every variant is a tuple of non-`Named` fields and one is
/// a `Vec`, which is `GoEnumRepresentation::RawMessage`.
fn raw_message_enum() -> EnumDef {
    let variant = |name: &str, ty: TypeRef| EnumVariant {
        name: name.into(),
        fields: vec![field("_0", ty, false)],
        ..EnumVariant::default()
    };
    EnumDef {
        name: "SampleInput".into(),
        rust_path: "samplelib::SampleInput".into(),
        variants: vec![
            variant("Single", TypeRef::String),
            variant("Multiple", TypeRef::Vec(Box::new(TypeRef::String))),
        ],
        serde_rename_all: Some("snake_case".into()),
        ..EnumDef::default()
    }
}

fn optional_input(name: &str) -> FieldDef {
    field(name, TypeRef::Named("SampleInput".into()), true)
}

#[test]
fn a_null_optional_enum_field_is_omitted_instead_of_refused() {
    let rendered = render(
        serde_json::json!({"model": "m", "input": null}),
        vec![field("model", TypeRef::String, false), optional_input("input")],
        Vec::new(),
        &[raw_message_enum()],
        false,
    );

    assert!(rendered.contains("Model: `m`,"), "{rendered}");
    assert!(!rendered.contains("Input"), "{rendered}");
}

/// The liter-llm shape: the `null` sits on a field of an array element, one level below the
/// request, where the refusal was reported against the whole fixture.
#[test]
fn a_null_optional_field_inside_an_array_element_is_omitted() {
    let holder = TypeDef {
        name: "SampleHolder".into(),
        rust_path: "samplelib::SampleHolder".into(),
        fields: vec![field("name", TypeRef::String, false), optional_input("input")],
        ..TypeDef::default()
    };
    let rendered = render(
        serde_json::json!({"holders": [{"name": "one", "input": null}]}),
        vec![field(
            "holders",
            TypeRef::Vec(Box::new(TypeRef::Named("SampleHolder".into()))),
            false,
        )],
        vec![holder, target_type()],
        &[raw_message_enum()],
        false,
    );

    assert!(rendered.contains("Name: `one`"), "{rendered}");
    assert!(!rendered.contains("Input"), "{rendered}");
}

/// Negative control: a present value on the same optional enum field is still lowered, so the
/// null check cannot be satisfied by dropping the field unconditionally.
#[test]
fn a_present_optional_enum_field_is_still_lowered() {
    let rendered = render(
        serde_json::json!({"model": "m", "input": "hello"}),
        vec![field("model", TypeRef::String, false), optional_input("input")],
        Vec::new(),
        &[raw_message_enum()],
        false,
    );

    assert!(
        rendered.contains("Input: ptr(pkg.SampleInput(`\"hello\"`))"),
        "{rendered}"
    );
}
