//! Regression coverage for a real downstream crate's nondeterminism: a wildcard fixture's
//! element half (`imports[].source`) must resolve against the bound block variable's OWN type
//! (`ImportInfo`), never re-anchored against the call's result type the way the container half
//! is. This is the exact shape CI caught: `ImportInfo` declares `source` and has no `imports`
//! field, so `NoMethodError: undefined method 'imports' for an instance of
//! SomeBinding::ImportInfo` fired when the element half was rendered through
//! `FieldResolver::accessor` instead of `FieldResolver::element_accessor`.
//!
//! `accessor()` re-anchors a path against the call's result type via
//! `result_relative_path`/`envelope_projected_path`: since the root (`ProcessResult`) does not
//! declare `source` directly, but `result_fields` names `imports` and the IR confirms
//! `imports.source` reaches a declaring type (`ImportInfo`), the envelope rescue prefixed the
//! ALREADY-ELEMENT-BOUND `e` back through the container path, emitting `e.imports[0].source` —
//! addressing a field that does not exist on the element. `element_accessor()` takes the element
//! path literally (only the alias map applies), correctly emitting `e.source`.

use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

/// `ProcessResult { imports: Vec<ImportInfo> }`, `ImportInfo { source: String }` — the tslp
/// shape: `ImportInfo` declares no `imports` field, so a re-anchored element accessor cannot
/// compile.
fn process_result_import_info_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![FieldDef {
                name: "imports".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("ImportInfo".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ImportInfo".to_string(),
            fields: vec![FieldDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn import_info_resolver() -> FieldResolver {
    let type_defs = process_result_import_info_type_defs();
    let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "ruby");
    let collection_map = FieldResolver::ir_collection_fields(&type_defs);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let result_fields: HashSet<String> = ["imports".to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_field_map, Some("ProcessResult".to_string()))
    .with_ir_collection_map(collection_map, Some("ProcessResult".to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

fn render_imports_source_any(assertion_type: &str) -> String {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some("imports[].source".to_string()),
        value: Some(serde_json::json!("os")),
        ..Default::default()
    };
    let mut out = String::new();
    super::assertions::render_assertion(
        &mut out,
        &assertion,
        "result",
        &import_info_resolver(),
        false,
        &E2eConfig::default(),
        &HashSet::new(),
        &HashMap::new(),
    );
    out
}

/// THE CANARY, keyed on the real tslp regression: the element half must resolve directly on the
/// bound block variable (`e.source`), never re-anchored back through the container's own IR
/// path (`e.imports[0].source`, which addresses a field `ImportInfo` does not declare).
#[test]
fn wildcard_element_half_resolves_against_the_element_type_not_the_container_path() {
    let out = render_imports_source_any("contains");
    assert!(
        out.contains("{ |e| e.source.to_s.include?("),
        "expected the element accessor to read straight off the block variable, got:\n{out}"
    );
    assert!(
        !out.contains("e.imports"),
        "element accessor must not re-walk the container path off the block variable, got:\n{out}"
    );
}
