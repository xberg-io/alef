//! Regression coverage for the Swift *snippet* generator's treatment of a JSON-bridged leaf.
//!
//! swift-bridge collapses a JSON-bridged field to one `RustString`, which has no elements and no
//! subscript. The snippet generator asked nothing and emitted the subscript anyway, so a
//! documentation snippet could not compile.
//!
//! The invariant these tests pin is about the SPELLING, not about either generator's verdict:
//! neither generator may emit `accessor()[...]` on a JSON-bridged leaf, because that is the thing
//! that does not compile. The two generators are deliberately allowed to differ on what they do
//! INSTEAD. The e2e generator may decode the leaf with `JSONSerialization` and navigate the
//! decoded value (see `json_bridged_navigation`), since a test only has to run; a documentation
//! snippet must stay idiomatic and readable, so it clamps the path back to the leaf itself and
//! shows that. An earlier version of this file used "does the e2e generator refuse?" as the
//! oracle, which silently encoded the era when neither generator could express the step at all.
//! ~keep
//!
//! These tests drive both real entry points — `render_test_method` and `snippet::render_with_ir` —
//! against one IR, so a fix that teaches only one generator the rule cannot pass.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// The e2e generator's wording for "this step is unspellable in Swift".
const JSON_BRIDGE_SKIP: &str = "swift-bridge JSON-bridges it to RustString";

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// IR whose getter shapes are the ones the Swift backend actually emits: `labels` and `headings`
/// JSON-bridge to a `RustString`, `sections` stays a countable `RustVec`.
fn bridged_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "SectionInfo".to_string(),
            fields: vec![
                field("level", TypeRef::Primitive(PrimitiveType::U32), false),
                field("text", TypeRef::String, false),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "PageMetadata".to_string(),
            fields: vec![
                field("title", TypeRef::String, false),
                // `HashMap<String, String>` -> `fn labels(&self) -> String` (the whole map as JSON).
                field(
                    "labels",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
                // `Option<Vec<SectionInfo>>` -> `fn headings(&self) -> String`.
                field(
                    "headings",
                    TypeRef::Vec(Box::new(TypeRef::Named("SectionInfo".to_string()))),
                    true,
                ),
                // `Vec<SectionInfo>` -> `fn sections(&self) -> RustVec<SectionInfo>`: the negative
                // control. An indiscriminate refusal would take this one down with the others.
                field(
                    "sections",
                    TypeRef::Vec(Box::new(TypeRef::Named("SectionInfo".to_string()))),
                    false,
                ),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![field("metadata", TypeRef::Named("PageMetadata".to_string()), false)],
            has_serde: true,
            ..TypeDef::default()
        },
    ];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, functions)
}

fn e2e_config() -> (E2eConfig, CallConfig) {
    let call_config = CallConfig {
        function: "process".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("process".to_string(), call_config.clone());
    e2e_config.result_fields = ["metadata".to_string()].into_iter().collect();
    (e2e_config, call_config)
}

fn fixture_showing(path: &str) -> Fixture {
    fixture_with_operation(path, serde_json::json!({"op": "show", "path": path, "display": true}))
}

fn fixture_iterating(path: &str, item: &str, fields: &[&str]) -> Fixture {
    fixture_with_operation(
        path,
        serde_json::json!({"op": "iterate", "path": path, "item": item, "fields": fields}),
    )
}

/// One fixture drives both generators: `docs.presentation` is what the snippet renders, and the
/// identically-pathed `assertions` entry is what the e2e test method renders.
fn fixture_with_operation(path: &str, operation: serde_json::Value) -> Fixture {
    Fixture {
        id: "bridged_leaf".to_string(),
        description: "Bridged leaf".to_string(),
        call: Some("process".to_string()),
        docs: serde_json::from_value(serde_json::json!({
            "topic": "guides",
            "presentation": {"operations": [operation]}
        }))
        .expect("docs must parse"),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some(path.to_string()),
            value: Some(serde_json::json!("Example")),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn render_e2e(fixture: &Fixture) -> String {
    let (type_defs, functions) = bridged_ir();
    let (e2e, call_config) = e2e_config();
    let map = super::values::build_swift_first_class_map(&type_defs, &[], &e2e, &call_config);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        fixture,
        &e2e,
        "process",
        "result",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        &type_defs,
        &[],
        &functions,
        &[],
    );
    out
}

fn render_snippet(fixture: &Fixture) -> String {
    let (type_defs, functions) = bridged_ir();
    let (e2e, _) = e2e_config();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    super::snippet::render_with_ir(fixture, &e2e, &config, &type_defs, &[], &functions).expect("snippet renders")
}

/// The reported shape: a string-keyed subscript on a field the binding collapsed to one
/// `RustString`. `labels()["theme"]` is not spellable, and the e2e file rendered from the same IR
/// says so on the line the snippet contradicted.
#[test]
fn should_clamp_a_map_subscript_to_the_bridged_leaf_the_e2e_generator_refuses() {
    let fixture = fixture_showing("metadata.labels[\"theme\"]");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    assert!(
        e2e.contains(JSON_BRIDGE_SKIP),
        "premise: the e2e generator must refuse this step, got:\n{e2e}"
    );
    assert!(
        !snippet.contains("labels()["),
        "the snippet must not subscript a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("print(result.metadata().labels())"),
        "the snippet must fall back to the readable bridged leaf, got:\n{snippet}"
    );
}

/// The same impossibility spelled as an index plus a member read.
#[test]
fn should_clamp_an_indexed_step_into_a_bridged_leaf() {
    let fixture = fixture_showing("metadata.headings[0].text");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    // ~keep Premise restated: the e2e generator now NAVIGATES this step by decoding the leaf,
    // where it used to refuse it outright. What the snippet generator must not do is unchanged.
    assert!(
        e2e.contains("JSONSerialization") && !e2e.contains(JSON_BRIDGE_SKIP),
        "premise: the e2e generator must decode-and-navigate this step, not refuse it, got:\n{e2e}"
    );
    assert!(
        !e2e.contains("headings()[0]") && !e2e.contains("headings()?[0]"),
        "the e2e generator must not subscript a RustString leaf either, got:\n{e2e}"
    );
    assert!(
        !snippet.contains("headings()[0]") && !snippet.contains("headings()?[0]"),
        "the snippet must not index a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("result.metadata().headings()"),
        "the snippet must still show the bridged leaf itself, got:\n{snippet}"
    );
}

/// An `iterate` reads elements off its own leaf, so a bridged leaf has no shorter prefix that
/// still iterates — the operation goes, and the snippet falls back to the whole result.
#[test]
fn should_drop_an_iterate_over_a_json_bridged_leaf() {
    let fixture = fixture_iterating("metadata.headings", "section", &["text"]);
    let snippet = render_snippet(&fixture);

    assert!(
        !snippet.contains("for section in"),
        "the snippet must not iterate a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("print(result)"),
        "dropping the only operation must fall back to showing the whole result, got:\n{snippet}"
    );
}

/// Negative control. `sections` is a genuine `RustVec`, so both generators must keep stepping into
/// it; a fix that refused every subscript would fail here.
#[test]
fn should_leave_a_countable_vec_leaf_indexable_in_both_generators() {
    let fixture = fixture_showing("metadata.sections[0].text");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    assert!(
        !e2e.contains(JSON_BRIDGE_SKIP),
        "premise: a countable RustVec leaf must not be refused, got:\n{e2e}"
    );
    assert!(
        snippet.contains("result.metadata().sections()[0].text()"),
        "the snippet must keep indexing a countable RustVec leaf, got:\n{snippet}"
    );
}

/// The invariant the preceding tests are instances of, stated once over a table: a subscript
/// written directly on a JSON-bridged accessor does not compile, so NEITHER generator may spell
/// one — and a countable `RustVec` leaf must still be indexed by both.
///
/// ~keep The oracle is the leaf's own shape, carried in the table, not "does the e2e generator
/// refuse?". Keying off e2e's verdict encoded the era when a bridged leaf was unreachable for
/// both generators; the e2e generator can now decode and navigate one, which is a different
/// answer to a different question and must not drag the snippet rule along with it.
#[test]
fn should_agree_with_the_e2e_generator_about_every_step_past_a_leaf() {
    // (path, accessor, leaf is a countable RustVec rather than a JSON-bridged RustString)
    let cases = [
        ("metadata.labels[\"theme\"]", "labels()", false),
        ("metadata.headings[0].text", "headings()", false),
        ("metadata.sections[0].text", "sections()", true),
    ];
    for (path, accessor, countable) in cases {
        let fixture = fixture_showing(path);
        let e2e = render_e2e(&fixture);
        let snippet = render_snippet(&fixture);
        let subscripted = |out: &str| out.contains(&format!("{accessor}[")) || out.contains(&format!("{accessor}?["));
        assert_eq!(
            subscripted(&snippet),
            countable,
            "the snippet generator spells `{path}` wrongly: a bridged leaf must never be \
             subscripted and a countable one always must\n--- snippet ---\n{snippet}"
        );
        // ~keep The e2e generator HOISTS a countable vec into a local and subscripts that
        // (`let _vec_sections_x = result.metadata().sections(); _vec_sections_x[0]`), where the
        // snippet generator inlines the subscript onto the accessor call. Both are correct Swift,
        // so the countable side can only require that a subscript appears somewhere; only the
        // bridged side can require it appears on the accessor call nowhere.
        if countable {
            assert!(
                e2e.contains("[0]"),
                "the e2e generator must still index a countable RustVec leaf for `{path}`, \
                 whether inline or through a hoisted local\n--- e2e ---\n{e2e}"
            );
        } else {
            assert!(
                !subscripted(&e2e),
                "the e2e generator must never subscript the JSON-bridged accessor for `{path}`\
                 \n--- e2e ---\n{e2e}"
            );
        }
    }
}
