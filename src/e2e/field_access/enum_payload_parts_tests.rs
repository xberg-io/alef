//! A fixture path that steps through a data-carrying enum field and then into a list of
//! tagged parts: `choices[0].message.content.parts` and `choices[0].message.content.parts[0].text`.
//!
//! Types are neutral: `Reply { choices: Vec<Choice> }`, `Choice { message: Message }`,
//! `Message { content: Option<Body> }`, `Body::{Text(String), Parts(Vec<Piece>)}`,
//! `Piece::{Text { text }, Note { note }}` (internally tagged).

use std::collections::{HashMap, HashSet};

use super::FieldResolver;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn named(name: &str) -> TypeRef {
    TypeRef::Named(name.to_string())
}

fn type_defs() -> Vec<TypeDef> {
    let def = |name: &str, fields: Vec<FieldDef>| TypeDef {
        name: name.to_string(),
        fields,
        ..TypeDef::default()
    };
    vec![
        def("Reply", vec![field("choices", TypeRef::Vec(Box::new(named("Choice"))))]),
        def("Choice", vec![field("message", named("Message"))]),
        def("Message", vec![field("content", TypeRef::Optional(Box::new(named("Body"))))]),
    ]
}

fn enums() -> Vec<EnumDef> {
    vec![
        EnumDef {
            name: "Body".to_string(),
            serde_untagged: true,
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", TypeRef::String)],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Parts".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", TypeRef::Vec(Box::new(named("Piece"))))],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        },
        EnumDef {
            name: "Piece".to_string(),
            serde_tag: Some("type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![field("text", TypeRef::String)],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Note".to_string(),
                    fields: vec![field("note", TypeRef::String)],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        },
    ]
}

fn resolver() -> FieldResolver {
    let optional: HashSet<String> = ["choices[0].message.content".to_string()].into_iter().collect();
    let array: HashSet<String> = ["choices".to_string()].into_iter().collect();
    FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &array, &HashSet::new())
        .with_ir_enum_map(
            FieldResolver::ir_enum_fields(&type_defs(), &enums()),
            Some("Reply".to_string()),
        )
}

#[test]
fn rust_should_match_the_variant_instead_of_naming_it_as_a_field() {
    let rendered = resolver().accessor("choices[0].message.content.parts", "rust", "result");
    assert!(
        rendered.contains("Body::Parts(") && !rendered.contains(".parts"),
        "`parts` is a variant of `Body`, not a struct field (E0609); got: {rendered}"
    );
}

#[test]
fn rust_should_match_the_variant_that_declares_a_field_of_an_element() {
    let rendered = resolver().accessor("choices[0].message.content.parts[0].text", "rust", "result");
    assert!(
        rendered.contains("Piece::Text {") && rendered.contains("Body::Parts("),
        "`text` is a field of `Piece::Text`; got: {rendered}"
    );
}

#[test]
fn python_should_subscript_below_a_variant_getter_that_returns_json() {
    let rendered = resolver().accessor("choices[0].message.content.parts[0].text", "python", "result");
    assert!(
        rendered.contains("parts[0][\"text\"]"),
        "the `parts` getter returns json.loads output, so a part is a dict; got: {rendered}"
    );
}

/// The per-language rendering of both paths, printed so a measuring job can read the lines.
#[test]
fn measure_every_language() {
    let resolver = resolver();
    for language in [
        "rust", "python", "node", "wasm", "go", "java", "csharp", "ruby", "php", "elixir", "kotlin", "swift", "dart",
        "zig", "gleam", "r", "c",
    ] {
        for path in [
            "choices[0].message.content.parts",
            "choices[0].message.content.parts[0].text",
        ] {
            println!("MEASURE {language} {path} => {}", resolver.accessor(path, language, "result"));
        }
    }
}
