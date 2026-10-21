//! Regression coverage for the Java e2e generator's `.getValue()` accessor gate on
//! IR-classified enum fields.
//!
//! ~keep This is a sibling test module, not an addition to `assertions.rs` (already over the
//! repo's 1,000-line file-modularization cap), per that rule's guidance for over-limit files.
//!
//! `field_is_enum` in `assertions.rs` decides whether an assertion may append `.getValue()` to
//! a field access. It broadened to trust `FieldResolver::is_enum` (IR-derived) without checking
//! *which* Rust representation the IR enum has. `getValue()` is real only on the plain Java
//! `enum` the binding backend emits for a fieldless (or non-data-carrying) enum; a data-carrying
//! enum — e.g. a `#[serde(untagged)]` union — is still an "enum" in the IR, but the Java binding
//! backend renders it as a wrapper class with no `getValue()` method, so the emitted assertion
//! did not compile. The 98 Java tests that shipped alongside that broadening all happened to
//! cover only fieldless enums, so none caught it.
//!
//! These tests drive the real entry point, `render_test_method`, against two representative IR
//! shapes and pin the exact accessor chosen for each — not just "does `.getValue()` appear
//! somewhere", but the specific expression the assertion renders.

use super::test_method::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// A fieldless enum — the shape the Java binding backend renders as a plain `enum` with a
/// `getValue()` accessor (`@JsonValue`).
fn stage_status_enum() -> EnumDef {
    EnumDef {
        name: "StageStatus".to_string(),
        variants: vec![
            EnumVariant {
                name: "Queued".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Complete".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// A `#[serde(untagged)]` union with a data-carrying variant — the shape the Java binding
/// backend renders as a wrapper class (`gen_java_untagged_wrapper`) with `value()` / `text()` /
/// `asString()` accessors, and no `getValue()`.
fn stage_output_enum() -> EnumDef {
    EnumDef {
        name: "StageOutput".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "0".to_string(),
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

fn table_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "StatusResult".to_string(),
            fields: vec![FieldDef {
                name: "status".to_string(),
                ty: TypeRef::Named("StageStatus".to_string()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OutputResult".to_string(),
            fields: vec![FieldDef {
                name: "summary".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("StageOutput".to_string()))),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];
    let enums = vec![stage_status_enum(), stage_output_enum()];
    let functions = vec![
        FunctionDef {
            name: "read_status".to_string(),
            return_type: TypeRef::Named("StatusResult".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "read_summary".to_string(),
            return_type: TypeRef::Named("OutputResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, enums, functions)
}

fn equals_fixture(id: &str, field: &str, value: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Render `fixture` through the real `render_test_method` entry point. `fields_display_as_text`
/// mirrors the hand-maintained `alef.toml` config a consumer would carry for a union-typed
/// content field (independent of, and unaffected by, the IR-derived enum-shape fix under test).
fn render(
    fixture: &Fixture,
    function: &str,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
    fields_display_as_text: &[&str],
) -> String {
    let call = CallConfig {
        function: function.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call,
        fields_display_as_text: fields_display_as_text.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let config = ResolvedCrateConfig::default();

    let mut out = String::new();
    render_test_method(
        &mut out,
        fixture,
        "SampleClass",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &[],
        &config,
        type_defs,
        enums,
        functions,
        &[],
    );
    out
}

/// Shape (a): a fieldless IR enum field must still render through `.getValue()` — this is
/// cd866bfdc's original fix, and this test protects it from being clawed back by the
/// wrapper-enum exclusion added alongside it.
#[test]
fn a_fieldless_enum_field_still_renders_through_get_value() {
    let (type_defs, enums, functions) = table_ir();
    let fixture = equals_fixture("status_smoke", "status", "Queued");
    let out = render(&fixture, "read_status", &type_defs, &enums, &functions, &[]);
    assert!(
        out.contains(".getValue()"),
        "expected the fieldless enum field to render through .getValue(), got:\n{out}"
    );
}

/// Shape (b): a data-carrying (`#[serde(untagged)]`) IR enum field must NOT render through
/// `.getValue()` — the Java binding backend never emits that method on the wrapper class it
/// generates for this shape, so the assertion would not compile. Configuring
/// `fields_display_as_text` for the field pins the exact fallback accessor a real consumer
/// observes (`.text()`) rather than only checking `.getValue()`'s absence.
#[test]
fn a_data_carrying_union_enum_field_renders_through_the_wrapper_text_accessor_not_get_value() {
    let (type_defs, enums, functions) = table_ir();
    let fixture = equals_fixture("summary_smoke", "summary", "hello");
    let out = render(&fixture, "read_summary", &type_defs, &enums, &functions, &["summary"]);
    assert!(
        !out.contains(".getValue()"),
        "a data-carrying union enum field must never render through .getValue() (the Java \
         binding backend's wrapper class for this shape declares no such method), got:\n{out}"
    );
    assert!(
        out.contains(".map(v -> v.text()).orElse(\"\")"),
        "expected the union wrapper's .text() accessor, got:\n{out}"
    );
}

/// Same data-carrying union shape as above, but with no `fields_display_as_text` config at
/// all — the minimal reproduction of the shipped regression. `.getValue()` must still never
/// appear, independent of whether a text-accessor fallback is configured.
#[test]
fn a_data_carrying_union_enum_field_never_renders_get_value_even_without_display_as_text_config() {
    let (type_defs, enums, functions) = table_ir();
    let fixture = equals_fixture("summary_smoke_bare", "summary", "hello");
    let out = render(&fixture, "read_summary", &type_defs, &enums, &functions, &[]);
    assert!(
        !out.contains(".getValue()"),
        "a data-carrying union enum field must never render through .getValue(), got:\n{out}"
    );
}

/// A same-shaped fieldless enum on an unrelated return type is unaffected by the wrapper-enum
/// exclusion — the IR anchor is the call's declared return type.
#[test]
fn an_unrelated_fieldless_enum_field_is_unaffected() {
    let (type_defs, enums, functions) = table_ir();
    let fixture = equals_fixture("status_smoke_2", "status", "Complete");
    let out = render(&fixture, "read_status", &type_defs, &enums, &functions, &[]);
    assert!(
        out.contains(".getValue()"),
        "expected the fieldless enum field to render through .getValue(), got:\n{out}"
    );
}
