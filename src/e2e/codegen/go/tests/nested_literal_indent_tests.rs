//! Regression coverage for the indentation of composite literals nested inside other composite
//! literals in the Go documentation snippet.
//!
//! A nested literal is rendered relative to its own first column and the enclosing literal has to
//! shift its continuation lines one tab deeper; embedding it verbatim left every nested field at
//! the depth of the field that holds it, which `gofmt` rewrites. The docs pipeline never runs
//! `gofmt` over a published snippet, so the generator has to get this right itself. ~keep

use super::array_field_snippet_tests::{assert_gofmt_stable, field, render, target_type};
use crate::core::ir::{TypeDef, TypeRef};

fn wrapper_type() -> TypeDef {
    TypeDef {
        name: "SampleWrapper".into(),
        rust_path: "samplelib::SampleWrapper".into(),
        fields: vec![
            field("name", TypeRef::String, false),
            field("target", TypeRef::Named("SampleTarget".into()), false),
        ],
        ..TypeDef::default()
    }
}

#[test]
fn a_struct_nested_in_a_struct_is_indented_one_level_deeper() {
    let rendered = render(
        serde_json::json!({"model": "m", "target": {"name": "search"}}),
        vec![
            field("model", TypeRef::String, false),
            field("target", TypeRef::Named("SampleTarget".into()), false),
        ],
        vec![target_type()],
        &[],
        false,
    );

    assert!(
        rendered.contains("\t\tTarget: &pkg.SampleTarget{\n\t\t\tName: `search`,\n\t\t},"),
        "{rendered}"
    );
    assert_gofmt_stable(&rendered);
}

#[test]
fn a_struct_nested_in_an_array_element_is_indented_one_level_deeper() {
    let rendered = render(
        serde_json::json!({"wrappers": [{"name": "one", "target": {"name": "search"}}]}),
        vec![field(
            "wrappers",
            TypeRef::Vec(Box::new(TypeRef::Named("SampleWrapper".into()))),
            false,
        )],
        vec![wrapper_type(), target_type()],
        &[],
        false,
    );

    assert!(rendered.contains("Name: `search`,"), "{rendered}");
    assert_gofmt_stable(&rendered);
}
