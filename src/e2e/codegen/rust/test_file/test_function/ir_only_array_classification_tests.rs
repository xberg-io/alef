//! Regression coverage for a `count_min`/`greater_than_or_equal`-style assertion against a leaf
//! `Option<Vec<T>>` field whose `Vec`-ness is known ONLY through the IR — no `[e2e].fields_array`
//! config entry names it.
//!
//! ~keep Split out of `optional_segment_len_tests.rs`, which pins the SIBLING bug (the accessor
//! itself failing to unwrap an intermediate `Option<Vec<T>>` segment before indexing/`.len()`).
//! This file pins a DIFFERENT bug in the same neighborhood: when the optional field is the path's
//! own LEAF (no `.length`/`.count`/`.size` suffix, e.g. a `count_min` assertion whose `field` is
//! the collection itself), `render_rust_with_optionals` deliberately leaves the leaf un-unwrapped
//! (`!is_leaf && optional_fields.contains(..)` in `optional_renderers.rs`) and hands the decision
//! to the assertion-type-specific helpers in `assertion_helpers.rs`
//! (`render_count_min_assertion`/`render_gte_assertion`/`render_not_empty_assertion`/
//! `render_is_empty_assertion`), which branch on `field_resolver.is_optional(..) &&
//! field_resolver.is_array(..)`. Before the fix, `is_array` never consulted the IR (only the
//! hand-maintained `fields_array` config), so a field whose collection-ness is known only through
//! the IR read as scalar, and `render_count_min_assertion` fell to its plain branch:
//! `assert!({field_access}.len() >= {n}, ...)` against a still-wrapped `Option<Vec<Chunk>>` — a
//! non-compiling `E0624`/`E0599` in the real generated suite (`method len is private` /
//! `no method named len found for enum Option<T>`, depending on how method resolution reports it).

use super::*;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::{Assertion, Fixture};

/// `Envelope { results: Vec<Document> }`, `Document { chunks: Option<Vec<Chunk>> }` — the exact
/// envelope-projection shape `rust/assertions/chunks_anchoring_tests.rs` already uses for the
/// synthetic `chunks_have_content` handler, reused here for a plain `count_min` assertion.
/// Deliberately NO `[e2e].fields_array` entry anywhere in the built `E2eConfig` below: the whole
/// point is that `chunks`'s `Vec`-ness is knowable only from `type_defs`.
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

fn make_assertion(assertion_type: &str, field: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        skip: None,
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

/// Renders through the production entry point (`render_test_function`), with `results` declared
/// as the consumer's envelope projection via `result_fields` — matching how a real `alef.toml`
/// tells the generator where the payload sits — but with NO `fields_array` entry, matching a
/// config that has never needed to name `chunks` by hand because nothing indexed into it before.
fn render(assertions: Vec<Assertion>) -> String {
    let call = CallConfig {
        function: "get_report".to_string(),
        module: "sample_lib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        result_fields: ["results".to_string()].into_iter().collect(),
        ..CallConfig::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "get_report_chunks".to_string(),
        description: "sample_lib report with an IR-only-optional chunks collection".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"sample_lib\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &envelope_document_type_defs(),
        &[],
        &envelope_document_functions(),
        "sample_lib",
        None,
        None,
        false,
    );
    out
}

/// The confirmed defect: `count_min` against the leaf `results[0].chunks` (no `.length` suffix,
/// no `fields_array` entry) must unwrap the `Option<Vec<Chunk>>` via `.as_ref().is_some_and(..)`,
/// never a bare `.len()` against the still-wrapped value.
///
/// Revert symptom: reverting the `FieldResolver::is_array` IR fallback makes this fail — the
/// output contains `results[0].chunks.len() >= 2` (a `.len()` call directly on `Option<Vec<T>>`,
/// which does not compile) instead of the `.as_ref().is_some_and(..)` form, and
/// `syn::parse_file` still succeeds either way (both are syntactically valid Rust; only the
/// `.len()` form fails to TYPE-check, which `syn` cannot catch) — so the string assertions below
/// are the ones that actually prove the fix, not the parse check.
#[test]
fn count_min_on_ir_only_optional_collection_leaf_unwraps_before_len() {
    let out = render(vec![make_assertion(
        "count_min",
        "results[0].chunks",
        serde_json::Value::Number(serde_json::Number::from(2u64)),
    )]);

    assert!(
        out.contains("results[0].chunks.as_ref().is_some_and(|v| v.len() >= 2)"),
        "must unwrap the Option<Vec<Chunk>> via as_ref().is_some_and before comparing length; got:\n{out}"
    );
    assert!(
        !out.contains("results[0].chunks.len() >= 2"),
        "must not emit a bare .len() against the still-wrapped Option<Vec<Chunk>>; got:\n{out}"
    );

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}

/// Companion coverage for `not_empty` on the same leaf shape, which takes the sibling
/// `is_opt && is_arr` branch in `render_not_empty_assertion` — this one is not a compile failure
/// either way (both shapes are valid Rust), but a SEMANTIC weakening: without `is_arr`,
/// `not_empty` on `Option<Vec<T>>` degrades to "is present" (`Some(..)`, true even for
/// `Some(vec![])`) instead of the fixture's actual claim, "present AND non-empty".
///
/// Revert symptom: reverting the fix makes this fail — the output contains
/// `result.results[0].chunks.is_some()` (checking presence only) instead of
/// `results[0].chunks.as_ref().is_some_and(|v| !v.is_empty())` (checking presence AND
/// non-emptiness), so a fixture asserting `not_empty` on an `Option<Vec<T>>` that resolves to
/// `Some(vec![])` would wrongly pass pre-fix and correctly fail post-fix.
#[test]
fn not_empty_on_ir_only_optional_collection_leaf_unwraps_before_is_empty() {
    let out = render(vec![make_assertion(
        "not_empty",
        "results[0].chunks",
        serde_json::Value::Bool(true),
    )]);

    assert!(
        out.contains("results[0].chunks.as_ref().is_some_and(|v| !v.is_empty())"),
        "must unwrap the Option<Vec<Chunk>> and check non-emptiness, not just presence; got:\n{out}"
    );
    assert!(
        !out.contains("result.results[0].chunks.is_some()"),
        "must not degrade to a bare presence check against the still-wrapped Option<Vec<Chunk>>; got:\n{out}"
    );

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}
