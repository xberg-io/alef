//! Regression coverage for a nested `Option<Vec<T>>` segment field reached through an
//! array-projected path (`entries[0].sections`) rather than as a top-level result field.
//!
//! ~keep Before this fix, `FieldResolver::ir_field_sets` only ever proved a BARE field name
//! optional ("sections"), by unanimity across every declaration of that name in the crate. The
//! `render_rust_with_optionals` accessor renderer, though, keys its per-segment unwrap check by
//! the FULL cumulative path it has walked so far ("entries[0].sections") — the exact convention
//! `fields_optional` config entries use. A bare name never matches a cumulative path once it
//! crosses more than one segment, so a genuinely `Option<Vec<T>>` field reached through a
//! `results[]`-style projection rendered unwrapped: `result.entries[0].sections[0]` (`E0608`,
//! cannot index `Option<Vec<T>>`) and `result.entries[0].sections.len()` (no such method on
//! `Option`). `presentation.rs` (the doc-snippet generator) already solved this for its own
//! accessors by calling `FieldResolver::with_anchored_optional_paths`, which walks the IR's own
//! `(owner_type, field_name)` graph instead of matching bare names. `render_test_function` did
//! not call it at all. This file pins the fix: the assertion-resolver construction in
//! `render_test_function` must anchor the fixture's own assertion field paths the same way.

use super::*;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::{Assertion, Fixture};

/// `SampleResult { entries: Vec<Entry> }`, `Entry { sections: Option<Vec<Section>>, tags:
/// Vec<String> }`, `Section { label: String }`. Only `Entry.sections` is optional; `Entry.tags`
/// is the non-optional negative control, declared on the SAME owning type so a fix that
/// unwraps unconditionally (rather than reading `FieldDef::optional`) cannot pass by accident.
fn sample_lib_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SampleResult".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Entry".to_string(),
            fields: vec![
                FieldDef {
                    name: "sections".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Section".to_string()))),
                    optional: true,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "tags".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: false,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Section".to_string(),
            fields: vec![FieldDef {
                name: "label".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn sample_lib_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "get_report".to_string(),
        return_type: TypeRef::Named("SampleResult".to_string()),
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

/// Renders a fixture with the given assertions against `sample_lib_type_defs`/
/// `sample_lib_functions`, exactly through the production entry point
/// (`render_test_function`) — not a hand-built resolver — so this pins the actual wiring the
/// fix touches, not a reimplementation of it.
fn render(assertions: Vec<Assertion>) -> String {
    let call = CallConfig {
        function: "get_report".to_string(),
        module: "sample_lib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        ..CallConfig::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "get_report_sections".to_string(),
        description: "sample_lib report with optional nested sections".to_string(),
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
        &sample_lib_type_defs(),
        &[],
        &sample_lib_functions(),
        "sample_lib",
        None,
        None,
        false,
    );
    out
}

/// The confirmed defect, indexing site: `entries[0].sections[0].label` must unwrap the
/// `Option<Vec<Section>>` before indexing.
#[test]
fn nested_optional_segment_field_unwraps_before_indexing() {
    let out = render(vec![make_assertion(
        "equals",
        "entries[0].sections[0].label",
        serde_json::Value::String("alpha".to_string()),
    )]);

    assert!(
        out.contains("sections.as_ref().unwrap()[0]"),
        "must unwrap the Option<Vec<Section>> before indexing; got:\n{out}"
    );
    // The exact pre-fix defect shape: indexing straight into the Option would not compile
    // (E0608). Guard against a fix that emits SOME unwrap elsewhere but still leaves this
    // exact non-compiling shape in the output alongside it.
    assert!(
        !out.contains("sections[0]"),
        "must not emit the un-unwrapped Option<Vec<T>> index; got:\n{out}"
    );

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}

/// The confirmed defect, length site: `entries[0].sections.length` must unwrap the
/// `Option<Vec<Section>>` before `.len()` — the reported `method \`len\` is private` shape.
#[test]
fn nested_optional_segment_field_unwraps_before_len() {
    let out = render(vec![make_assertion(
        "equals",
        "entries[0].sections.length",
        serde_json::Value::Number(serde_json::Number::from(1u64)),
    )]);

    assert!(
        out.contains("sections.as_ref().unwrap().len()"),
        "must unwrap the Option<Vec<Section>> before .len(); got:\n{out}"
    );
    assert!(
        !out.contains("sections.len()"),
        "must not emit .len() directly against the Option; got:\n{out}"
    );

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}

/// Negative control: `entries[0].tags` is a plain `Vec<String>` (not `Option<Vec<T>>`) on the
/// SAME owning type (`Entry`) as the optional `sections` field above. A fix that unwraps
/// unconditionally — rather than reading `FieldDef::optional` per field — is a different bug;
/// this must keep emitting the plain, unwrapped index and length forms.
#[test]
fn non_optional_sibling_segment_field_stays_unwrapped_plain() {
    let out = render(vec![
        make_assertion(
            "equals",
            "entries[0].tags[0]",
            serde_json::Value::String("beta".to_string()),
        ),
        make_assertion(
            "equals",
            "entries[0].tags.length",
            serde_json::Value::Number(serde_json::Number::from(1u64)),
        ),
    ]);

    assert!(
        out.contains("tags[0]") && !out.contains("tags.as_ref().unwrap()[0]"),
        "a non-optional Vec<T> field must index plainly, with no Option unwrap; got:\n{out}"
    );
    assert!(
        out.contains("tags.len()") && !out.contains("tags.as_ref().unwrap().len()"),
        "a non-optional Vec<T> field must call .len() plainly, with no Option unwrap; got:\n{out}"
    );

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}
