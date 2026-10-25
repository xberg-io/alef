//! A Dart assertion on the chat reply text reads the union through its `text()` accessor.
//!
//! The reply field is a payload-carrying union: plain text or a list of parts. The binding adds a
//! `text()` method to such a type when the field is listed in `fields_display_as_text`. The
//! generator must use that method and must not refuse the assertion.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::DartFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_case::{DartTestCaseContext, render_test_case};

fn reply_union() -> EnumDef {
    EnumDef {
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
    }
}

fn render_reply(assertions: Vec<Assertion>, display_as_text: bool) -> String {
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
    let enums = vec![reply_union()];
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
        assertions,
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let dart_first_class_map = DartFirstClassMap::default();
    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        DartTestCaseContext {
            e2e_config: &e2e_config,
            lang: "dart",
            bridge_class: "Sample",
            dart_first_class_map: &dart_first_class_map,
            adapters: &[],
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            functions: &functions,
            errors: &[],
            native_typed_dtos: false,
            is_snippet: false,
        },
    );
    out
}

fn assertion(kind: &str, value: Option<&str>) -> Assertion {
    Assertion {
        assertion_type: kind.to_string(),
        field: Some("content".to_string()),
        value: value.map(|v| serde_json::Value::String(v.to_string())),
        ..Assertion::default()
    }
}

#[test]
fn an_equals_check_on_the_reply_text_reads_the_text_accessor() {
    let out = render_reply(vec![assertion("equals", Some("hello"))], true);
    assert!(out.contains("content?.text() ?? ''"), "got:\n{out}");
    assert!(!out.contains("skipped"), "got:\n{out}");
}

#[test]
fn a_not_empty_check_on_the_reply_text_reads_the_text_accessor() {
    let out = render_reply(vec![assertion("not_empty", None)], true);
    assert!(
        out.contains("expect((result.content?.text() ?? ''), isNotEmpty);"),
        "got:\n{out}"
    );
    assert!(!out.contains("skipped"), "got:\n{out}");
}

#[test]
fn a_union_field_without_the_text_accessor_is_still_refused() {
    let out = render_reply(vec![assertion("equals", Some("hello"))], false);
    assert!(
        out.contains("skipped: enum field 'content' is a payload-carrying union"),
        "got:\n{out}"
    );
}
