//! Regression coverage for the C# e2e generator's enum-field classification.
//!
//! `render_test_method` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `[e2e.call.overrides.csharp] enum_fields` config (`fields_enum` in
//! `assertions.rs`). A consumer whose `alef.toml` never declared that entry got a raw
//! `Assert.Equal("KeyValue", result.Data!.Kind)` for an honest-to-goodness `DataNodeKind` enum
//! field — a `CS1503` the field's declared C# type can never satisfy, and one whose reported
//! error message is a red herring (`IAsyncEnumerable<char>?`, xunit's closest-matching but
//! unrelated overload) rather than anything naming the real defect.
//!
//! `csharp.rs` now wires the same IR-derived enum classification the rust e2e generator uses
//! (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's declared Rust
//! return type via `resolve_declared_result_type`) so a field renders as enum-typed whenever the
//! IR says so, config or not. These tests drive the real entry point, `render_test_method`, with
//! no `enum_fields`/`fields_enum` config at all — the classification must come from the IR alone. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig, StreamingConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

/// A `DataNodeKind`-shaped enum: two unit variants, no serde rename overrides.
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

fn equals_kind_assertion(expected: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some("kind".to_string()),
        value: Some(serde_json::Value::String(expected.to_string())),
        ..Assertion::default()
    }
}

fn fixture_calling(call: &str) -> Fixture {
    Fixture {
        id: "kind_smoke".to_string(),
        description: "Kind field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![equals_kind_assertion("KeyValue")],
        ..Fixture::default()
    }
}

/// Render `fixture` through the real `render_test_method` entry point with `type_defs`/`enums`/
/// `functions` as the only source of enum knowledge — no `enum_fields`/`assert_enum_fields`
/// config, matching a consumer `alef.toml` that never declared `fields_enum`.
#[allow(clippy::too_many_arguments)]
fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> String {
    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        fixture,
        "Sample",
        "Process",
        "SampleException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        type_defs,
        enums,
        functions,
        &[],
    );
    out
}

/// Shared IR fixture for the table: `process` returns `ProcessResult { kind: DataNodeKind }`,
/// `other` returns `OtherResult { kind: String }` (same field name, unrelated non-enum type, to
/// prove classification is anchored per-call rather than matching on the leaf name alone), and
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

fn e2e_config_for(call: &str, function: &str, extra: impl FnOnce(&mut CallConfig)) -> E2eConfig {
    let mut call_config = CallConfig {
        function: function.to_string(),
        ..CallConfig::default()
    };
    extra(&mut call_config);
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

/// Table-driven: does the rendered assertion take the enum-comparison branch
/// (`System.Text.Json.JsonSerializer.Serialize`) or the naive literal-vs-object branch?
///
/// `call` doubles as the Rust function name in every case here — `table_ir()`'s
/// [`FunctionDef`]s and `e2e_config_for`'s named calls are both keyed on it.
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
        let e2e_config = e2e_config_for(case.call, case.call, |_| {});
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let took_enum_branch = out.contains("System.Text.Json.JsonSerializer.Serialize");
        assert_eq!(
            took_enum_branch, case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name, case.expect_enum_branch
        );
    }
}

/// Regression: the enum-equals assertion must compare the fixture's wire literal
/// (`"KeyValue"`) directly against the real serialized wire string
/// (`System.Text.Json.JsonSerializer.Serialize(...).Trim('"')`), with no lowercasing
/// applied to either side. A previous version lowercased the expected literal
/// (`"KeyValue"` -> `"keyvalue"`) and ran the actual value through
/// `JsonNamingPolicy.SnakeCaseLower.ConvertName(...)` (`"KeyValue"` -> `"key_value"`) --
/// two different, mutually-inconsistent transforms that only agreed by coincidence for
/// enums whose wire values already were snake_case. `DataNodeKind` (modeled by
/// `data_node_kind_enum()`, no `serde_rename_all`) never matched, so the compiling
/// assertion failed at runtime on every run. ~keep
#[test]
fn enum_equals_assertion_compares_the_real_wire_value_unmodified() {
    let (type_defs, enums, functions) = table_ir();
    let e2e_config = e2e_config_for("process", "process", |_| {});
    let fixture = fixture_calling("process");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(r#"Assert.Equal("KeyValue", "#),
        "expected the fixture's wire literal verbatim (no lowercasing), got:\n{out}"
    );
    assert!(
        !out.contains("\"keyvalue\""),
        "must not lowercase the expected literal into a value the real wire format never produces, got:\n{out}"
    );
    assert!(
        !out.contains("JsonNamingPolicy"),
        "must not guess a naming-policy transform; it must serialize through the enum's own converter, got:\n{out}"
    );
    assert!(
        out.contains(
            "System.Text.Json.JsonSerializer.Serialize(result.Kind, new System.Text.Json.JsonSerializerOptions { \
             Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping }).Trim('\"')"
        ),
        "expected the actual value serialized through the enum's own JsonConverter with \
         HTML-safe escaping disabled (otherwise wire values containing `+ ' < > &` mismatch \
         the plain-string expectation), got:\n{out}"
    );
}

/// An explicit `fields_enum` config entry keeps working unchanged (config wins, same as
/// before this fix) — the IR only rescues fields the config never mentioned.
#[test]
fn an_explicit_fields_enum_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    // "other" has no IR-derived enum field at all (OtherResult.kind is String) — only the
    // explicit config entry can make this classify as enum.
    let e2e_config = e2e_config_for("other", "other", |call| {
        call.overrides.insert(
            "csharp".to_string(),
            crate::e2e::config::CallOverride {
                enum_fields: [("kind".to_string(), "DataNodeKind".to_string())].into_iter().collect(),
                ..crate::e2e::config::CallOverride::default()
            },
        );
    });
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains("System.Text.Json.JsonSerializer.Serialize"),
        "explicit fields_enum config must still classify the field as enum, got:\n{out}"
    );
}

/// The streaming branch (`render_streaming_test_method`) is a structurally separate code path
/// that never receives `field_resolver` — wiring the IR-derived enum map into
/// `render_test_method`'s per-call resolver construction (now built before the streaming
/// early-return) must not perturb which branch a streaming fixture takes, nor emit any of the
/// field-assertion content the non-streaming branch would.
#[test]
fn a_forced_streaming_call_still_routes_to_the_streaming_branch_unaffected_by_enum_wiring() {
    let (type_defs, enums, functions) = table_ir();
    let e2e_config = e2e_config_for("process", "process", |call| {
        call.streaming = Some(StreamingConfig::Enabled(true));
    });
    let fixture = fixture_calling("process");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        !out.contains("System.Text.Json.JsonSerializer.Serialize") && !out.contains("result.Kind"),
        "a forced-streaming call must not emit the non-streaming field-assertion path, got:\n{out}"
    );
}
