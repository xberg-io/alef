//! Regression coverage for the Swift e2e generator's enum-field classification.
//!
//! `render_test_method` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` / `[e2e.call.overrides.swift] enum_fields` config
//! (`assertions.rs`'s `field_is_enum`, and the wildcard mirrors at `render_wildcard_assertion`'s
//! `elem_is_enum` and `accessors.rs::swift_traversal_contains_assert`'s `elem_is_enum`). A
//! consumer whose `alef.toml` never declared that entry got no `.rawValue` on a first-class
//! Codable struct's enum property — `XCTAssertEqual(result.kind, "keyValue")` compares a
//! `DataNodeKind` against a `String`, which does not compile.
//!
//! `test_method.rs` now wires the same IR-derived classification the rust/csharp/gleam e2e
//! generators use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's
//! declared Rust return type via `resolve_declared_result_type`). These tests drive the real
//! entry point, `render_test_method`, with no `fields_enum`/`enum_fields` config at all — the
//! classification must come from the IR alone. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::field_access::SwiftFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

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

/// A payload-carrying `#[serde(untagged)]` union. `gen_bindings::enums::emit_enum` reaches
/// `swift_enum_raw_decl` — the only branch that gives the Swift enum a `: String` raw value, and
/// therefore a `.rawValue` — solely when every variant is fieldless; this shape gets
/// `swift_enum_decl` with associated values instead.
fn stage_output_union() -> EnumDef {
    EnumDef {
        name: "StageOutput".to_string(),
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
        has_serde: true,
        ..EnumDef::default()
    }
}

/// `process` returns `ProcessResult { kind: DataNodeKind }`, `other` returns
/// `OtherResult { kind: String }` (same leaf name, unrelated non-enum type — proves the
/// classification is anchored per-call rather than matching on the leaf name alone),
/// `process_optional` returns `OptionalResult { kind: Option<DataNodeKind> }`, and
/// `process_union` returns `UnionResult { kind: StageOutput }` — the payload-carrying shape,
/// carried in the same surface as the unit-only enum so one IR exercises both branches.
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
        TypeDef {
            name: "UnionResult".to_string(),
            fields: vec![kind_field(TypeRef::Named("StageOutput".to_string()), false)],
            ..TypeDef::default()
        },
    ];
    let enums = vec![data_node_kind_enum(), stage_output_union()];
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
        FunctionDef {
            name: "process_union".to_string(),
            return_type: TypeRef::Named("UnionResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, enums, functions)
}

/// A `SwiftFirstClassMap` that treats every table type as a first-class Codable struct, so
/// `kind` renders via property access (`result.kind`) rather than a swift-bridge method call
/// (`result.kind()`). The compile-breaking half of this defect only shows up on the property
/// path: swift-bridge already bridges every enum getter to a `RustString`, so method-call
/// `.toString()` is emitted for both an enum and a plain-string leaf — but a first-class
/// Codable enum property needs `.rawValue` to compare against the fixture's wire-format string,
/// and a plain `String` property must NOT get `.rawValue` (it has none). ~keep
fn first_class_map() -> SwiftFirstClassMap {
    SwiftFirstClassMap {
        first_class_types: ["ProcessResult", "OtherResult", "OptionalResult", "UnionResult"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        field_types: HashMap::new(),
        vec_field_names: HashSet::new(),
        json_bridged_field_names: HashSet::new(),
        json_bridged_by_type: HashMap::new(),
        getter_optionality: HashMap::new(),
        root_type: None,
        stringy_fields_by_type: HashMap::new(),
    }
}

/// `render_test_method` (`swift/test_method.rs`) resolves the per-fixture Swift first-class
/// root type from the call's `result_type` override on ANY of `c`/`csharp`/`java`/`kotlin`/`go`/
/// `php` (`values::swift_call_result_type`) — Swift has no override axis of its own for this,
/// so it reuses whichever backend's config already names the IR type. This is a different
/// config axis from `enum_fields`/`fields_enum`, so setting it does not smuggle in the
/// classification under test.
fn e2e_config_for(call: &str, result_type: &str, extra: impl FnOnce(&mut CallConfig)) -> E2eConfig {
    let mut call_config = CallConfig {
        function: call.to_string(),
        ..CallConfig::default()
    };
    call_config.overrides.insert(
        "csharp".to_string(),
        CallOverride {
            result_type: Some(result_type.to_string()),
            ..CallOverride::default()
        },
    );
    extra(&mut call_config);
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    swift_first_class_map: &SwiftFirstClassMap,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> String {
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        fixture,
        e2e_config,
        "",
        "",
        &[],
        false,
        None,
        swift_first_class_map,
        "Sample",
        &config,
        type_defs,
        enums,
        functions,
        &[],
    );
    out
}

struct Case {
    name: &'static str,
    call: &'static str,
    result_type: &'static str,
    expect_raw_value: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no fields_enum config gets .rawValue via the IR",
        call: "process",
        result_type: "ProcessResult",
        expect_raw_value: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        call: "other",
        result_type: "OtherResult",
        expect_raw_value: false,
    },
    Case {
        name: "an Option<Enum> field is classified as enum via the IR",
        call: "process_optional",
        result_type: "OptionalResult",
        expect_raw_value: true,
    },
    Case {
        name: "a payload-carrying union field does not get .rawValue (associated values have none)",
        call: "process_union",
        result_type: "UnionResult",
        expect_raw_value: false,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums, functions) = table_ir();
    let map = first_class_map();
    for case in CASES {
        let e2e_config = e2e_config_for(case.call, case.result_type, |_| {});
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &map, &type_defs, &enums, &functions);
        let has_raw_value = out.contains(".rawValue");
        assert_eq!(
            has_raw_value, case.expect_raw_value,
            "{}: expected .rawValue = {}, got:\n{out}",
            case.name, case.expect_raw_value
        );
    }
}

/// An explicit per-call `enum_fields` entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `other.kind` is `String` in the IR, so only the
/// config entry can make this classify as enum.
#[test]
fn an_explicit_enum_fields_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let map = first_class_map();
    let e2e_config = e2e_config_for("other", "OtherResult", |call| {
        call.overrides.insert(
            "swift".to_string(),
            CallOverride {
                enum_fields: [("kind".to_string(), "DataNodeKind".to_string())].into_iter().collect(),
                ..CallOverride::default()
            },
        );
    });
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &map, &type_defs, &enums, &functions);
    assert!(
        out.contains(".rawValue"),
        "explicit enum_fields config must still classify the field as enum, got:\n{out}"
    );
}

/// The exact assertion a unit-only enum property must lower to.
const UNIT_ENUM_ASSERTION: &str = "        XCTAssertEqual(result.kind.rawValue, \"key_value\")";

/// The exact line a payload-carrying union field must render instead of any assertion.
const UNION_SKIP_LINE: &str = "        // skipped: enum field 'kind' is a payload-carrying union \
                               with no scalar wire accessor in this binding";

/// The control: a unit-only enum must still lower to its exact `.rawValue` comparison.
///
/// ~keep Without this, a "fix" that refuses every enum-typed field would satisfy the union test
/// below. Reverting the payload-union gate leaves this green — that is the point of a control.
#[test]
fn a_unit_only_enum_property_still_lowers_to_its_exact_raw_value_comparison() {
    let (type_defs, enums, functions) = table_ir();
    let map = first_class_map();
    let out = render(
        &fixture_calling("process"),
        &e2e_config_for("process", "ProcessResult", |_| {}),
        &map,
        &type_defs,
        &enums,
        &functions,
    );
    assert!(
        out.contains(UNIT_ENUM_ASSERTION),
        "expected exactly `{UNIT_ENUM_ASSERTION}`, got:\n{out}"
    );
}

/// The compile-shape discriminator: one IR carrying BOTH a unit-only enum and a payload-carrying
/// union must lower only the first through `.rawValue`, and must render a registered refusal for
/// the second rather than any comparison at all.
///
/// `emit_enum` gives a Swift enum its `: String` raw value only on the all-fieldless-variants
/// branch, so `StageOutput` is declared with associated values and `result.kind.rawValue` is a
/// "value of type 'StageOutput' has no member 'rawValue'" compile error. Withholding the accessor
/// on its own does not fix that: every string-shaped arm reads the same lowered expression, so the
/// field lands on `XCTAssertEqual(result.kind, "key_value")` — "cannot convert value of type
/// 'StageOutput' to expected argument type 'String'", still a compile failure, just a different
/// one. Both halves are pinned: the exact skip line, and the total absence of the fixture literal
/// `"key_value"` — the only thing that could carry a comparison into the emitted file. ~keep
///
/// Reverting the `payload_union_skip_line` gate in `swift/assertions.rs` fails this on the missing
/// skip line AND on the reappearing `XCTAssertEqual(result.kind, "key_value")`.
#[test]
fn a_payload_carrying_union_field_renders_a_registered_refusal_not_a_type_mismatch() {
    let (type_defs, enums, functions) = table_ir();
    let map = first_class_map();

    let union_out = render(
        &fixture_calling("process_union"),
        &e2e_config_for("process_union", "UnionResult", |_| {}),
        &map,
        &type_defs,
        &enums,
        &functions,
    );
    assert!(
        union_out.contains(UNION_SKIP_LINE),
        "expected exactly `{UNION_SKIP_LINE}`, got:\n{union_out}"
    );
    assert!(
        !union_out.contains(".rawValue"),
        "a payload-carrying union is a Swift enum with associated values and no rawValue, so the \
         assertion must not reach for one, got:\n{union_out}"
    );
    assert!(
        !union_out.contains("\"key_value\""),
        "the fixture literal must not be compared against anything: every string arm would lower \
         the union leaf into a Swift type mismatch, got:\n{union_out}"
    );
}

/// The refusal must be a *registered* wording, or `ALEF_E2E_STRICT_FIELD_AVAILABILITY` walks past
/// it and a suite that dropped this assertion looks identical to one that ran it. ~keep
#[test]
fn the_union_refusal_is_recognised_by_the_field_skip_funnel() {
    use crate::e2e::codegen::field_skip::FieldSkip;
    assert_eq!(
        FieldSkip::extract_classified(UNION_SKIP_LINE),
        Some(("kind", FieldSkip::PayloadUnionHasNoScalarWireAccessor))
    );
}

fn render_configured_text_assertion(native: bool, kind: &str) -> String {
    let (mut types, enums, functions) = table_ir();
    types.iter_mut().find(|ty| ty.name == "UnionResult").unwrap().fields[0].optional = true;
    let config = e2e_config_for("process_union", "UnionResult", |call| {
        call.fields_display_as_text.insert("kind".into());
    });
    let map = if native {
        first_class_map()
    } else {
        super::values::build_swift_first_class_map(&types, &enums, &config, &config.calls["process_union"])
    };
    let mut fixture = fixture_calling("process_union");
    fixture.assertions[0].assertion_type = kind.into();
    render(&fixture, &config, &map, &types, &enums, &functions)
}

#[test]
fn configured_native_content_checks_text_emptiness_not_presence() {
    for (kind, assertion) in [("not_empty", "XCTAssertFalse"), ("is_empty", "XCTAssertTrue")] {
        let out = render_configured_text_assertion(true, kind);
        assert!(out.contains("result.kind?.text()"), "{out}");
        assert!(out.contains(assertion) && out.contains(".isEmpty"), "{out}");
        assert!(!out.contains("skipped:") && !out.contains("!= nil"), "{out}");
    }
}

#[test]
fn configured_opaque_content_decodes_json_before_checking_text() {
    for (kind, assertion) in [("not_empty", "XCTAssertFalse"), ("is_empty", "XCTAssertTrue")] {
        let out = render_configured_text_assertion(false, kind);
        assert!(out.contains("JSONDecoder().decode(Sample.StageOutput?.self"), "{out}");
        assert!(out.contains("result.kind()?.toString()"), "{out}");
        assert!(out.contains(assertion) && out.contains(".isEmpty"), "{out}");
        assert!(!out.contains("skipped:") && !out.contains("!= nil"), "{out}");
    }
}
