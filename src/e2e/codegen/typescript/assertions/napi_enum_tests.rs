//! What the node (napi) binding actually hands JavaScript for an enum-typed struct field.
//!
//! ~keep Same FALSE FAILURE class as `wasm_enum_tests`, and invisible for the same reason: a
//! correctly authored fixture against a correctly generated binding produced an assertion that
//! could never hold. This one shipped — it kept a real downstream crate's Node e2e gate red
//! across releases, and because `publish-node` gates on that gate, npm silently never received a
//! build. The generic path lowered `kind` to `String(result.kind).includes("Function")`; napi
//! hands over `{ type: "Function" }`, which stringifies to `"[object Object]"`.

use super::render_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn variant(name: &str, fields: Vec<FieldDef>, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        serde_rename: serde_rename.map(str::to_string),
        ..EnumVariant::default()
    }
}

fn payload_field() -> FieldDef {
    FieldDef {
        name: "_0".to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

/// `StructureKind { Function, Other(String) }` — the real shape from a downstream crate: unit
/// variants plus one payload variant, serde default. `is_tagged_data_enum` routes it to the
/// tagged-object emitter, so it is `{ type: "Function" }` on the wire.
/// `Format { Markdown, Html }` — all unit variants, so `#[napi(string_enum)]`: a bare string.
fn enums() -> Vec<EnumDef> {
    vec![
        EnumDef {
            name: "StructureKind".to_string(),
            variants: vec![
                variant("Function", vec![], None),
                variant("Other", vec![payload_field()], None),
            ],
            ..EnumDef::default()
        },
        EnumDef {
            name: "Format".to_string(),
            variants: vec![variant("Markdown", vec![], Some("md")), variant("Html", vec![], None)],
            ..EnumDef::default()
        },
    ]
}

fn type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Item".to_string(),
        fields: vec![
            FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::Named("StructureKind".to_string()),
                ..FieldDef::default()
            },
            FieldDef {
                name: "format".to_string(),
                ty: TypeRef::Named("Format".to_string()),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }]
}

fn resolver() -> FieldResolver {
    let defs = type_defs();
    let enum_defs = enums();
    let result_fields: HashSet<String> = ["kind".to_string(), "format".to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&defs, &enum_defs),
        Some("Item".to_string()),
    )
    .with_napi_tagged_object_enums(&enum_defs)
}

fn enum_field_config() -> HashMap<String, String> {
    HashMap::from([
        ("kind".to_string(), "StructureKind".to_string()),
        ("format".to_string(), "Format".to_string()),
    ])
}

fn render_node(field: &str, expected: &str) -> String {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(expected.to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver(),
        false,
        &enum_field_config(),
        "node",
        false,
        false,
        false,
    );
    out
}

#[test]
fn should_read_the_discriminant_property_for_a_tagged_data_enum() {
    let out = render_node("kind", "Function");
    assert_eq!(out, "    expect(result.kind?.[\"type\"]).toBe(\"Function\");\n");
}

/// The exact defect: the object must never be compared as a scalar. `String({type:"Function"})`
/// is `"[object Object]"`, so any whole-object comparison is false for every possible value —
/// which is why this asserts on the emitted text rather than only on the happy path above.
#[test]
fn should_never_compare_the_whole_tagged_object_as_a_scalar() {
    let out = render_node("kind", "Function");
    assert!(
        !out.contains("String(result.kind)"),
        "whole-object scalar comparison survived: {out}"
    );
    assert!(
        !out.contains("expect(result.kind).toBe"),
        "whole-object equality survived: {out}"
    );
}

/// A unit-only enum is a `#[napi(string_enum)]` — a bare string. Reading a discriminant off it
/// would be the mirror-image false failure, so the napi path must decline to handle it.
#[test]
fn should_leave_a_unit_only_string_enum_as_a_scalar_comparison() {
    let out = render_node("format", "md");
    assert!(
        !out.contains("?.[\"type\"]"),
        "string_enum wrongly treated as a tagged object: {out}"
    );
}

/// The shape that actually shipped broken: `structure[].kind` is a WILDCARD path, and the
/// wildcard branch returns before the non-wildcard enum dispatch is reached. Fixing only the
/// plain path would leave this — the real downstream-crate assertion — unchanged, so
/// this test is the one that pins the defect rather than a neighbouring case.
#[test]
fn should_read_the_discriminant_for_a_wildcard_enum_element() {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some("items[].kind".to_string()),
        value: Some(serde_json::Value::String("Function".to_string())),
        ..Default::default()
    };
    let defs = vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![FieldDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        type_defs().remove(0),
    ];
    let enum_defs = enums();
    let result_fields: HashSet<String> = ["items".to_string(), "items[].kind".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &["items".to_string()].into_iter().collect(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&defs, &enum_defs),
        Some("Report".to_string()),
    )
    .with_napi_tagged_object_enums(&enum_defs);

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        &enum_field_config(),
        "node",
        false,
        false,
        false,
    );
    assert!(
        !out.contains("String(e.kind)"),
        "wildcard element still stringified as a scalar -- this is the shipped defect: {out}"
    );
    assert!(
        out.contains("e.kind?.[\"type\"] === \"Function\""),
        "wildcard element did not read the discriminant: {out}"
    );
}
