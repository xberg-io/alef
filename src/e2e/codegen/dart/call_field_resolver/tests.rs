//! Regression coverage for the missing `with_ir_result_fields` wiring in
//! `build_call_field_resolver` — see the module doc comment in `call_field_resolver.rs` for the
//! full mechanism. Split into its own file per this repo's file-modularization convention (one
//! concern, its own test module) rather than growing `call_field_resolver.rs` inline.

use super::{CallFieldResolverInputs, build_call_field_resolver};
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::DartFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashSet;

/// `Envelope { results: Vec<Document> }`, `Document { chunks: Option<Vec<Chunk>> }` — the same
/// envelope-projection shape used by the sibling rust and swift IR-wiring regressions.
fn envelope_document_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![FieldDef {
                name: "results".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn envelope_document_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "get_report".to_string(),
        return_type: TypeRef::Named("Envelope".to_string()),
        ..FunctionDef::default()
    }]
}

/// The confirmed defect: `build_call_field_resolver` must anchor `is_optional`/`is_array` at the
/// call's own declared result type, so a leaf `Option<Vec<T>>` field known only through the IR —
/// no `[e2e].fields_optional`/`fields_array` entry anywhere — still classifies correctly.
///
/// Revert symptom: removing the `.with_ir_result_fields(..)` call this test pins makes both
/// assertions fail — `is_optional`/`is_array` fall back to the empty config-only sets and answer
/// `false` for `results[0].chunks`, which is exactly what made `dart_length_expr` emit a bare
/// `.length` against a nullable `List<Chunk>?` (dart analyzer: "potentially null").
#[test]
fn resolver_classifies_ir_only_optional_collection_leaf_without_config() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "get_report".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_fields: HashSet::from(["results".to_string()]),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let fixture = Fixture {
        id: "get_report_chunks".to_string(),
        description: "dart report with an IR-only-optional chunks collection".to_string(),
        assertions: vec![Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("results[0].chunks".to_string()),
            value: Some(serde_json::json!(2)),
            ..Default::default()
        }],
        ..Fixture::default()
    };
    let resolver = build_call_field_resolver(CallFieldResolverInputs {
        e2e_config: &e2e_config,
        call_config: &e2e_config.call,
        fixture: &fixture,
        lang: "dart",
        dart_first_class_map: &DartFirstClassMap::default(),
        type_defs: &envelope_document_type_defs(),
        enums: &[],
        functions: &envelope_document_functions(),
    });

    assert!(
        resolver.is_optional("results[0].chunks"),
        "is_optional must be true for an IR-only Option<Vec<T>> leaf with no fields_optional entry"
    );
    assert!(
        resolver.is_array("results[0].chunks"),
        "is_array must be true for an IR-only Vec<T> leaf with no fields_array entry"
    );
}
