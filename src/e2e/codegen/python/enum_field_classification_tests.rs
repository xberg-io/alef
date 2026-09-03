//! Regression coverage for the Python e2e generator's enum-field classification.
//!
//! `render_assertion` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` config (plus a per-call `assert_enum_fields` override and an
//! accessor-shape heuristic). A consumer whose `alef.toml` never declared `fields_enum` got a
//! bare `assert result.kind == "key_value"` for an honest-to-goodness `DataNodeKind` enum
//! field. Python does not fail to compile on that — it silently compares the PyO3 enum object
//! against a plain string, which is `False` for a real enum even when the wire value matches,
//! so the fixture's assertion asserts the wrong thing rather than refusing to build.
//!
//! `render_test_function` now wires the same IR-derived classification the rust/csharp/gleam/
//! swift/dart e2e generators use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`,
//! anchored at the call's declared Rust return type via `resolve_declared_result_type`). These
//! tests drive the real entry point, `render_test_function`, with no `fields_enum` config at
//! all — the classification must come from the IR alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_function::{RenderTestFunctionContext, render_test_function};

/// The `_alef_e2e_text(...)` serde-wire coercion helper Python wraps enum-typed fields in
/// before comparing against the fixture's wire-format expected value — the generated-code
/// fingerprint of the enum branch.
const ENUM_MARKER: &str = "_alef_e2e_text(";

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

fn kind_field(ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: "kind".to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// `process` returns `ProcessResult { kind: DataNodeKind }`, `other` returns
/// `OtherResult { kind: String }` (same leaf name, unrelated non-enum type — proves the
/// classification is anchored per-call rather than matching on the leaf name alone), and
/// `process_optional` returns `OptionalResult { kind: Option<DataNodeKind> }`.
fn table_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![kind_field(TypeRef::Named("DataNodeKind".to_string()), false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OtherResult".to_string(),
            fields: vec![kind_field(TypeRef::String, false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OptionalResult".to_string(),
            fields: vec![kind_field(
                TypeRef::Optional(Box::new(TypeRef::Named("DataNodeKind".to_string()))),
                true,
            )],
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
        FunctionDef {
            name: "process_optional".to_string(),
            return_type: TypeRef::Named("OptionalResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, enums, functions)
}

fn fixture_calling(call: &str) -> Fixture {
    Fixture {
        id: "kind_smoke".to_string(),
        description: "Kind field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("key_value".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn e2e_config_for(call: &str) -> E2eConfig {
    let call_config = CallConfig {
        function: call.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> String {
    let mut out = String::new();
    let config = ResolvedCrateConfig::default();
    let enum_fields = std::collections::HashMap::new();
    let handle_nested_types = std::collections::HashMap::new();
    let handle_dict_types = std::collections::HashSet::new();
    let convertible_types = ahash::AHashSet::new();
    let options_wrapped_types = std::collections::HashSet::new();
    let context = RenderTestFunctionContext {
        e2e_config,
        config: &config,
        type_defs,
        enums,
        functions,
        errors: &[],
        options_type: None,
        options_via: "kwargs",
        enum_fields: &enum_fields,
        handle_nested_types: &handle_nested_types,
        handle_dict_types: &handle_dict_types,
        force_bind_result: false,
        convertible_types: &convertible_types,
        crate_has_serde: false,
        options_wrapped_types: &options_wrapped_types,
    };
    render_test_function(&mut out, fixture, context);
    out
}

struct Case {
    name: &'static str,
    call: &'static str,
    expect_enum_branch: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no fields_enum config is classified as enum via the IR",
        call: "process",
        expect_enum_branch: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        call: "other",
        expect_enum_branch: false,
    },
    Case {
        name: "an Option<Enum> field is classified as enum via the IR",
        call: "process_optional",
        expect_enum_branch: true,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums, functions) = table_ir();
    for case in CASES {
        let e2e_config = e2e_config_for(case.call);
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let took_enum_branch = out.contains(ENUM_MARKER);
        assert_eq!(
            took_enum_branch, case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name, case.expect_enum_branch
        );
        // The dynamic-language failure mode: without the enum branch, Python does not fail
        // to compile -- it silently emits a raw `==` comparison of the enum object against
        // the fixture's wire-format string, which is `False` for a real PyO3 enum. Assert
        // the raw comparison is emitted ONLY on the non-enum path, so a regression here
        // (misclassifying a real enum field) is caught even though nothing fails to build.
        assert_eq!(
            out.contains("assert result.kind == \"key_value\""),
            !case.expect_enum_branch,
            "{}: a raw `==` comparison must be emitted only for the non-enum field, got:\n{out}",
            case.name
        );
    }
}

/// An explicit `fields_enum` config entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `other.kind` is `String` in the IR, so only the
/// config entry can make this classify as enum.
#[test]
fn an_explicit_fields_enum_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let mut e2e_config = e2e_config_for("other");
    e2e_config.fields_enum.insert("kind".to_string());
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(ENUM_MARKER),
        "explicit fields_enum config must still classify the field as enum, got:\n{out}"
    );
}
