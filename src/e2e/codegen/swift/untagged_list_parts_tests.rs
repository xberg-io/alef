//! Regression coverage for Swift assertions that read the list variant of a display-text union
//! (`choices[0].message.content.parts[0].text`).
//!
//! The Swift getter returns the plain text of the union as a `RustString`, so the Swift value has
//! no `parts()` accessor and no other public member holds the list. The assertions render a skip.
//!
//! These tests drive the real entry point, `render_test_method`, against an IR with the shape of a chat reply, so the classification
//! comes from the same maps production builds.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

fn chat_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ChatReply".to_string(),
            fields: vec![field(
                "choices",
                TypeRef::Vec(Box::new(TypeRef::Named("Choice".to_string()))),
                false,
            )],
            is_opaque: true,
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "Choice".to_string(),
            fields: vec![field("message", TypeRef::Named("ReplyMessage".to_string()), false)],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "ReplyMessage".to_string(),
            // `Option<ReplyContent>` arrives as `ty: Named` with `optional: true`.
            fields: vec![field("content", TypeRef::Named("ReplyContent".to_string()), true)],
            has_serde: true,
            ..TypeDef::default()
        },
    ];
    let enums = vec![
        EnumDef {
            name: "ReplyContent".to_string(),
            has_serde: true,
            serde_untagged: true,
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", TypeRef::String, false)],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Parts".to_string(),
                    is_tuple: true,
                    fields: vec![field(
                        "_0",
                        TypeRef::Vec(Box::new(TypeRef::Named("ReplyPart".to_string()))),
                        false,
                    )],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        },
        EnumDef {
            name: "ReplyPart".to_string(),
            has_serde: true,
            serde_tag: Some("type".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Text".to_string(),
                fields: vec![field("text", TypeRef::String, false)],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        },
    ];
    let functions = vec![FunctionDef {
        name: "chat".to_string(),
        return_type: TypeRef::Named("ChatReply".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, enums, functions)
}

fn render(assertions: Vec<Assertion>) -> String {
    let (type_defs, enums, functions) = chat_ir();
    let call_config = CallConfig {
        function: "chat".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("chat".to_string(), call_config.clone());
    // The consumer's own config: the content union reads as display text and is optional.
    e2e_config
        .fields_display_as_text
        .insert("choices[0].message.content".to_string());
    e2e_config
        .fields_optional
        .insert("choices[0].message.content".to_string());
    let fixture = Fixture {
        id: "parts_reply".to_string(),
        description: "reply content is a list of text parts".to_string(),
        call: Some("chat".to_string()),
        assertions,
        ..Fixture::default()
    };
    let map = super::values::build_swift_first_class_map(&type_defs, &enums, &e2e_config, &call_config);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        // The union is not a configured text type, so the bridge has no typed accessor for it and
        // the skip is the only thing left to render. A configured text type is read through the
        // accessor instead (`reply_parts_accessor_tests`).
        untagged_union_text_types: Vec::new(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "chat",
        "result",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        &type_defs,
        &enums,
        &functions,
        &[],
    );
    out
}

fn assertion(assertion_type: &str, field_path: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field_path.to_string()),
        value: Some(value),
        ..Assertion::default()
    }
}

#[test]
fn should_skip_a_path_into_the_list_variant_of_a_display_text_union() {
    let out = render(vec![
        assertion("count_equals", "choices[0].message.content.parts", serde_json::json!(2)),
        assertion(
            "equals",
            "choices[0].message.content.parts[0].text",
            serde_json::json!("hello"),
        ),
    ]);
    assert!(
        !out.contains(".parts()"),
        "the union getter is a RustString with no `parts()` member, got:\n{out}"
    );
    assert_eq!(
        out.matches("flattens to text (xberg-io/alef#691)").count(),
        2,
        "each path into the list variant must render a skip, got:\n{out}"
    );
}

#[test]
fn should_still_read_the_display_text_of_the_union_itself() {
    let out = render(vec![assertion(
        "equals",
        "choices[0].message.content",
        serde_json::json!("hello world"),
    )]);
    assert!(
        out.contains("content()") && !out.contains("alef#691"),
        "the union read as text must keep its assertion, got:\n{out}"
    );
}
