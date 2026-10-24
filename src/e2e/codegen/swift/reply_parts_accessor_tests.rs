//! Coverage for Swift assertions that read the list variant of a display-text union
//! (`choices[0].message.content.parts[0].text`).
//!
//! The union getter returns the plain text of the reply. The assertions read the whole value
//! through the typed accessor that the bridge emits for the same field, then navigate the
//! encoded value with the wire path of the variant.
//!
//! These tests drive the real entry point, `render_test_method`, against an IR with the shape of a
//! chat reply and the consumer's settings (display as text, optional, a configured text type), so
//! the classification comes from the same maps production builds.

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

fn reply_ir(untagged: bool) -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
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
            serde_untagged: untagged,
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

fn render(untagged: bool, assertions: Vec<Assertion>) -> String {
    let (type_defs, enums, functions) = reply_ir(untagged);
    let call_config = CallConfig {
        function: "chat".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("chat".to_string(), call_config.clone());
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
        untagged_union_text_types: vec!["ReplyContent".to_string()],
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
fn should_count_the_parts_through_the_typed_accessor() {
    let out = render(
        true,
        vec![assertion(
            "count_equals",
            "choices[0].message.content.parts",
            serde_json::json!(2),
        )],
    );
    assert!(
        out.contains(".message().contentValue()"),
        "the parts must come from the typed accessor of the union field, got:\n{out}"
    );
    assert!(
        out.contains("JSONEncoder().encode(") && out.contains("as? [Any])?.count"),
        "the count must read the encoded list, got:\n{out}"
    );
    assert!(
        !out.contains("skipped") && !out.contains("content()?.toString()"),
        "the path must render an assertion, not a skip or a read of the joined text, got:\n{out}"
    );
}

#[test]
fn should_read_the_text_of_one_part_through_the_typed_accessor() {
    let out = render(
        true,
        vec![
            assertion(
                "equals",
                "choices[0].message.content.parts[0].text",
                serde_json::json!("hello"),
            ),
            assertion(
                "equals",
                "choices[0].message.content.parts[1].text",
                serde_json::json!(" world"),
            ),
        ],
    );
    assert_eq!(
        out.matches(".message().contentValue()").count(),
        2,
        "each part assertion must read the typed accessor, got:\n{out}"
    );
    assert!(
        out.contains("dropFirst(0)") && out.contains("dropFirst(1)") && out.contains("[\"text\"]"),
        "each assertion must pick its part by index and read the text key, got:\n{out}"
    );
    assert!(
        out.contains("\"hello\"") && out.contains("\" world\""),
        "each assertion must compare its own text, got:\n{out}"
    );
}

/// The skip for display-text union paths is the fallback of the renderer. If it ran first, every
/// parts path would be skipped and the consumer's parts checks would stop asserting.
#[test]
fn should_render_the_renderer_before_the_skip_for_a_parts_path() {
    let out = render(
        true,
        vec![
            assertion("count_equals", "choices[0].message.content.parts", serde_json::json!(2)),
            assertion(
                "equals",
                "choices[0].message.content.parts[0].text",
                serde_json::json!("hello"),
            ),
        ],
    );
    assert!(
        !out.contains("alef#691") && !out.contains("// skipped"),
        "a parts path of a configured text type must not render the skip, got:\n{out}"
    );
    assert_eq!(
        out.matches(".message().contentValue()").count(),
        2,
        "each parts path must render an assertion, got:\n{out}"
    );
}

#[test]
fn should_still_read_the_display_text_of_the_union_itself() {
    let out = render(
        true,
        vec![assertion(
            "equals",
            "choices[0].message.content",
            serde_json::json!("hello world"),
        )],
    );
    assert!(
        out.contains("content()") && !out.contains("contentValue()"),
        "the union read as text must keep reading the text getter, got:\n{out}"
    );
}

#[test]
fn should_leave_a_union_that_is_not_untagged_to_the_existing_path() {
    let out = render(
        false,
        vec![assertion(
            "count_equals",
            "choices[0].message.content.parts",
            serde_json::json!(2),
        )],
    );
    assert!(
        !out.contains("contentValue()"),
        "a union with a wrapper in its wire form does not encode as its payload, got:\n{out}"
    );
    assert!(
        out.contains("flattens to text (xberg-io/alef#691)"),
        "the path the renderer cannot read keeps the skip, got:\n{out}"
    );
}
