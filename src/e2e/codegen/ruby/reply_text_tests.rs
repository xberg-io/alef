//! A Ruby assertion on the chat reply text reads a union field through its plain text.
//!
//! The binding lowers the reply union to a String (plain text) or an Array of part Hashes. A field
//! in `fields_display_as_text` is asserted on that text, never refused as a serialized Hash.

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn render(assertion: Assertion, display_as_text: bool) -> String {
    let type_defs = vec![TypeDef {
        name: "ChatReply".to_string(),
        fields: vec![FieldDef {
            name: "content".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("ReplyContent".to_string()))),
            optional: true,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let enums = vec![EnumDef {
        name: "ReplyContent".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            is_tuple: true,
            ..EnumVariant::default()
        }],
        serde_untagged: true,
        ..EnumDef::default()
    }];
    let functions = vec![FunctionDef {
        name: "chat".to_string(),
        return_type: TypeRef::Named("ChatReply".to_string()),
        ..FunctionDef::default()
    }];
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(
        "chat".to_string(),
        CallConfig {
            function: "chat".to_string(),
            result_var: "result".to_string(),
            ..CallConfig::default()
        },
    );
    if display_as_text {
        e2e_config.fields_display_as_text.insert("content".to_string());
    }
    let fixture = Fixture {
        id: "reply".to_string(),
        description: "reply".to_string(),
        call: Some("chat".to_string()),
        assertions: vec![assertion],
        ..Fixture::default()
    };
    super::spec_file::render_spec_file(
        "reply",
        &[&fixture],
        "Sample",
        None,
        "sample",
        None,
        &std::collections::HashMap::new(),
        false,
        &e2e_config,
        false,
        false,
        &[],
        &crate::core::config::ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        &enums,
        &functions,
    )
}

fn equals_hello() -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some("content".to_string()),
        value: Some(serde_json::json!("hello")),
        ..Assertion::default()
    }
}

#[test]
fn an_equals_check_on_the_reply_text_reads_the_text_of_the_union() {
    let out = render(equals_hello(), true);
    assert!(out.contains("v.is_a?(Array)"), "got:\n{out}");
    assert!(out.contains(".call(result.content)"), "got:\n{out}");
    assert!(
        out.contains("to eq('hello')") || out.contains("to eq(\"hello\")"),
        "got:\n{out}"
    );
    assert!(!out.contains("skipped"), "got:\n{out}");
}

#[test]
fn a_not_empty_check_on_the_reply_text_reads_the_text_of_the_union() {
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("content".to_string()),
        ..Assertion::default()
    };
    let out = render(assertion, true);
    assert!(out.contains("v.is_a?(Array)"), "got:\n{out}");
    assert!(!out.contains("skipped"), "got:\n{out}");
}

#[test]
fn a_union_field_that_is_not_display_as_text_is_still_refused() {
    let out = render(equals_hello(), false);
    assert!(out.contains("enum serialization differs in Ruby"), "got:\n{out}");
}
