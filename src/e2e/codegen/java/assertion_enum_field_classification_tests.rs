//! Regression coverage for the Java e2e generator's *assertion-side* enum-field classification.
//!
//! `render_test_method` decided whether a result field is enum-typed purely from the
//! hand-maintained `enum_fields` config (`fields_enum` in `alef.toml`). A consumer whose
//! config never listed a field — e.g. a recursive struct's own enum field, reached only
//! through its parent's field path (`data.kind` on a self-referential `Option<Box<DataNode>>`)
//! — got a raw `assertEquals("KeyValue", result.data().kind())` for an honest-to-goodness Java
//! `enum` field. That assertion can never pass: JUnit's `assertEquals(Object, Object)` calls
//! `.equals()`, and a `String` is never `.equals()` an enum constant, regardless of value.
//!
//! Every other backend that classifies enum fields this way (csharp, kotlin, dart, gleam,
//! swift, ...) rescues fields the config never mentioned from `FieldResolver::is_enum`, backed
//! by the IR-derived classification (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`,
//! anchored at the call's declared Rust return type via `resolve_declared_result_type`). Java's
//! `test_method.rs` built its field resolver without ever wiring that IR data in, and
//! `assertions.rs`'s `field_is_enum` never consulted `field_resolver.is_enum` even where it was
//! available — so a field rendered as enum-typed only when a consumer's `alef.toml` happened to
//! list it.
//!
//! This is the same class of defect `csharp/enum_field_classification_tests.rs` documents; these
//! tests drive the real entry point, `render_test_method`, with no `enum_fields` config at all —
//! the classification must come from the IR alone.

use super::test_method::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn data_node_kind_enum() -> EnumDef {
    EnumDef {
        name: "DataNodeKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "KeyValue".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Sequence".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

fn kind_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "kind".to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `ProcessResult.kind` is `DataNodeKind` (a real Java enum). `OtherResult.kind` is `String`
/// (same leaf field name, unrelated non-enum type) — proves the classification is anchored per
/// return type rather than matching on the field name alone.
fn table_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![kind_field(TypeRef::Named("DataNodeKind".to_string()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OtherResult".to_string(),
            fields: vec![kind_field(TypeRef::String)],
            ..TypeDef::default()
        },
    ];
    let enums = vec![data_node_kind_enum()];
    let functions = vec![
        FunctionDef {
            name: "process".to_string(),
            return_type: TypeRef::Named("ProcessResult".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "other".to_string(),
            return_type: TypeRef::Named("OtherResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, enums, functions)
}

fn kind_fixture(id: &str) -> Fixture {
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
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("KeyValue".to_string())),
            ..Assertion::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Render `fixture` through the real `render_test_method` entry point against `function`, with
/// `type_defs`/`enums`/`functions` as the only source of enum knowledge — no `enum_fields`
/// config, matching a consumer `alef.toml` that never declared `fields_enum`.
fn render(function: &str, type_defs: &[TypeDef], enums: &[EnumDef], functions: &[FunctionDef]) -> String {
    let fixture = kind_fixture("kind_smoke");
    let call = CallConfig {
        function: function.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = ResolvedCrateConfig::default();

    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
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

/// The regression this file exists for: with no `enum_fields` config, a real Java enum field
/// must still render as `result.kind().getValue()`, never a bare `result.kind()` compared
/// directly against a `String` literal.
#[test]
fn an_enum_typed_field_with_no_enum_fields_config_is_classified_as_enum_via_the_ir() {
    let (type_defs, enums, functions) = table_ir();
    let out = render("process", &type_defs, &enums, &functions);
    assert!(
        out.contains(".getValue()"),
        "expected the enum field to render through .getValue(), got:\n{out}"
    );
    assert!(
        out.contains("assertEquals(\"KeyValue\", "),
        "expected the equals assertion to compare against the wire value, got:\n{out}"
    );
}

/// A same-named non-enum field on an unrelated return type must not be misclassified as enum —
/// the IR anchor is the call's declared return type, not the leaf field name.
#[test]
fn a_same_named_non_enum_field_on_an_unrelated_type_is_not_misclassified_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let out = render("other", &type_defs, &enums, &functions);
    assert!(
        !out.contains(".getValue()"),
        "a non-enum String field must not be routed through .getValue(), got:\n{out}"
    );
}
