//! Regression coverage for `csharp_object_initializer`'s literal-construction behavior:
//! file-pointer fields, struct-typed collection element resolution, the `List<string>`
//! fallback when an element type cannot be resolved, and JSON-scalar wrapping. Type-name
//! casing behavior for the same function lives in `object_initializer_type_name_tests.rs`.

use super::setup::{csharp_object_initializer, csharp_object_initializer_with_enums};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use std::collections::HashMap;

#[test]
fn output_format_unit_variant_is_constructed_as_a_record() {
    let output_format = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![
            EnumVariant {
                name: "Markdown".into(),
                ..EnumVariant::default()
            },
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
        serde_rename_all: Some("lowercase".into()),
        ..EnumDef::default()
    };
    let rendered = csharp_object_initializer_with_enums(
        serde_json::json!({"output_format": "markdown"})
            .as_object()
            .expect("object"),
        "SampleConfig",
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &[output_format],
        &[],
        "",
    );
    assert_eq!(
        rendered,
        "new SampleConfig { OutputFormat = new OutputFormat.Markdown() }"
    );

    let true_enum = EnumDef {
        name: "OutputFormat".into(),
        variants: vec![EnumVariant {
            name: "Markdown".into(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let plain = csharp_object_initializer_with_enums(
        serde_json::json!({"output_format": "markdown"})
            .as_object()
            .expect("object"),
        "SampleConfig",
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &[true_enum],
        &[],
        "",
    );
    assert_eq!(plain, "new SampleConfig { OutputFormat = OutputFormat.Markdown }");
}

#[test]
fn normalized_enum_type_name_still_constructs_a_unit_record() {
    let uri = EnumDef {
        name: "URI".into(),
        variants: vec![
            EnumVariant {
                name: "File".into(),
                ..EnumVariant::default()
            },
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
        serde_rename_all: Some("lowercase".into()),
        ..EnumDef::default()
    };
    let rendered = csharp_object_initializer_with_enums(
        serde_json::json!({"uri": "file"}).as_object().expect("object"),
        "FetchConfig",
        &HashMap::from([("uri".into(), "Uri".into())]),
        &HashMap::new(),
        &[],
        &[uri],
        &[],
        "",
    );
    assert_eq!(rendered, "new FetchConfig { Uri = new Uri.File() }");
}

#[test]
fn native_initializer_reads_file_pointer_as_bytes() {
    let type_defs = [TypeDef {
        name: "Upload".into(),
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Bytes,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let files = [crate::e2e::fixture::FixtureDocsFileInput {
        field: "/content".into(),
        path: "guide.pdf".into(),
    }];
    let rendered = csharp_object_initializer(
        serde_json::json!({"content": "guide.pdf"}).as_object().expect("object"),
        "Upload",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &files,
        "",
    );
    assert!(
        rendered.contains("Content = System.IO.File.ReadAllBytes(\"guide.pdf\")"),
        "{rendered}"
    );
}

#[test]
fn object_initializer_uses_struct_element_type_for_object_valued_collections() {
    let type_defs = [
        TypeDef {
            name: "ChatCompletionRequest".into(),
            fields: vec![FieldDef {
                name: "messages".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Message".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Message".into(),
            fields: vec![],
            ..TypeDef::default()
        },
    ];
    let rendered = csharp_object_initializer(
        serde_json::json!({"messages": [{"role": "user", "content": "hi"}]})
            .as_object()
            .expect("object"),
        "ChatCompletionRequest",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &[],
        "",
    );
    assert_eq!(
        rendered,
        "new ChatCompletionRequest { Messages = new List<Message>() { JsonSerializer.Deserialize<Message>(\"{\\\"content\\\":\\\"hi\\\",\\\"role\\\":\\\"user\\\"}\", ConfigOptions)! } }",
        "{rendered}"
    );
}

#[test]
fn object_initializer_uses_struct_element_type_for_string_valued_collections() {
    // `RerankDocument` wraps a bare string on the wire (a single-field newtype), so
    // the fixture value is a plain JSON string — not an object — even though the
    // struct's real element type is `RerankDocument`, not `string`.
    let type_defs = [TypeDef {
        name: "RerankRequest".into(),
        fields: vec![FieldDef {
            name: "documents".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("RerankDocument".into()))),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = csharp_object_initializer(
        serde_json::json!({"documents": ["Artificial intelligence is..."]})
            .as_object()
            .expect("object"),
        "RerankRequest",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &[],
        "",
    );
    assert_eq!(
        rendered,
        "new RerankRequest { Documents = new List<RerankDocument>() { JsonSerializer.Deserialize<RerankDocument>(\"\\\"Artificial intelligence is...\\\"\", ConfigOptions)! } }",
        "{rendered}"
    );
}

#[test]
fn object_initializer_falls_back_to_list_string_for_unresolvable_element_type() {
    // No type_defs entry for the owning struct: the element type genuinely cannot
    // be resolved, so the historical `List<string>` fallback is correct here.
    let rendered = csharp_object_initializer(
        serde_json::json!({"tags": ["a", "b"]}).as_object().expect("object"),
        "Unregistered",
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &[],
        "",
    );
    assert_eq!(
        rendered, "new Unregistered { Tags = new List<string>() { \"a\", \"b\" } }",
        "{rendered}"
    );
}

#[test]
fn object_initializer_wraps_json_scalar_fields_in_json_element_deserialize() {
    // `CreateResponseRequest.Input` binds to `JsonElement?` in C# (an untagged
    // union field represented as arbitrary JSON), but a bare scalar fixture value
    // used to be emitted as a plain string literal, which doesn't satisfy the
    // generated `JsonElement?` property.
    let type_defs = [TypeDef {
        name: "CreateResponseRequest".into(),
        fields: vec![FieldDef {
            name: "input".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Json)),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = csharp_object_initializer(
        serde_json::json!({"input": "Say hello"}).as_object().expect("object"),
        "CreateResponseRequest",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &[],
        "",
    );
    assert_eq!(
        rendered,
        "new CreateResponseRequest { Input = JsonSerializer.Deserialize<JsonElement>(\"\\\"Say hello\\\"\", ConfigOptions)! }",
        "{rendered}"
    );
}
