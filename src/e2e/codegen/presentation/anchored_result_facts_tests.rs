//! Field facts a docs snippet reads must be anchored at the *call's own declared result type*,
//! not looked up by bare field name across every type the crate declares.
//!
//! ~keep Two defects share that one root cause, and both ship as non-compiling snippets:
//!
//! * **Optionality.** `FieldResolver::ir_field_sets` only calls a name optional when EVERY
//!   declaration of it across the whole IR is `Option<T>`, and the NAPI binding additionally
//!   widens every field of a `Default`-deriving type to optional (see
//!   `backends::napi::gen_bindings::types::napi_field_is_optional`). Neither fact survives a
//!   bare-name vote, so the TypeScript renderer emitted `result.metadata.title` against a
//!   `.d.ts` declaring `readonly metadata?: PageMetadata` — `TS18048`.
//! * **Availability.** `result_field_oracle_knows` answered `Some(true)` for any name declared
//!   on any type at all, so a non-error fixture asserting on a field that happens to exist
//!   somewhere unrelated derived an accessor the result type has no member for.
//!
//! The asymmetry between the two oracles is load-bearing and is asserted here in both
//! directions: [`FieldResolver::is_valid_for_result`] must keep default-allowing a name it has
//! never heard of (a hand-authored assertion knows the type; the oracle may not), while
//! [`FieldResolver::result_field_oracle_knows`] must reject that same name for an *inferred*
//! accessor (nothing authored it, so silence must not mean yes).

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

/// A docs-tagged fixture that hand-authors neither `shows` nor `presentation`, so every
/// operation is derived from `assertions` — the shape both defects appear in.
fn docs_fixture(assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture must parse")
}

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// `convert` -> `ConversionResult`, the anchor every test here resolves its field facts against.
fn convert_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

fn metadata_type() -> TypeDef {
    TypeDef {
        name: "Metadata".to_string(),
        fields: vec![field("title", TypeRef::String, false)],
        ..TypeDef::default()
    }
}

/// The exact shape a bare-name unanimity vote gets wrong: `metadata` is `Option<Metadata>` on
/// the type the call returns and a plain `Metadata` on an unrelated type, so "every declaration
/// is `Option<T>`" is false and the name never reaches `optional_fields` — even though the only
/// declaration that matters here is optional. ~keep
#[test]
fn an_optional_field_on_the_calls_return_type_is_optional_despite_a_required_twin_elsewhere() {
    let type_defs = vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field(
                "metadata",
                TypeRef::Optional(Box::new(TypeRef::Named("Metadata".to_string()))),
                true,
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "LogRecord".to_string(),
            fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()), false)],
            ..TypeDef::default()
        },
        metadata_type(),
    ];
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "metadata.title", "value": "Hello"}
    ]));

    let operations = resolve(
        &fixture,
        &config(),
        "node",
        &type_defs,
        &[],
        &convert_returning("ConversionResult"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.metadata?.title"],
        "optionality must come from `metadata` on ConversionResult, not from a vote across every \
         type that declares a field called `metadata`"
    );
}

/// The consumer shape, and the one that actually produced `TS18048` on 24 snippets: `metadata`
/// is declared `PageMetadata` — never `Option` — anywhere in the core crate, but its owner
/// derives `Default`, and the NAPI backend widens every field of a `has_default` type to
/// `Option<T>`, so the emitted `.d.ts` says `readonly metadata?: PageMetadata`. The snippet must
/// guard what the binding it is compiled against actually declares. ~keep
#[test]
fn a_required_field_of_a_default_deriving_result_type_is_optional_in_the_node_binding() {
    let type_defs = vec![
        TypeDef {
            name: "ScrapeOutcome".to_string(),
            has_default: true,
            fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()), false)],
            ..TypeDef::default()
        },
        metadata_type(),
    ];
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "metadata.title", "value": "Hello"}
    ]));

    let operations = resolve(
        &fixture,
        &config(),
        "node",
        &type_defs,
        &[],
        &convert_returning("ScrapeOutcome"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.metadata?.title"],
        "the NAPI binding declares every field of a Default-deriving type optional, so the \
         snippet must reach through `metadata` with `?.`"
    );
}

/// Nothing widens a `has_default` type's fields in the wasm binding — wasm-bindgen exposes the
/// declared field type through a getter — so the same IR must NOT gain a guard there. Without
/// this, "make it optional everywhere" would look like a passing fix. ~keep
#[test]
fn a_required_field_of_a_default_deriving_result_type_stays_required_in_the_wasm_binding() {
    let type_defs = vec![
        TypeDef {
            name: "ScrapeOutcome".to_string(),
            has_default: true,
            fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()), false)],
            ..TypeDef::default()
        },
        metadata_type(),
    ];
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "metadata.title", "value": "Hello"}
    ]));

    let operations = resolve(
        &fixture,
        &config(),
        "wasm",
        &type_defs,
        &[],
        &convert_returning("ScrapeOutcome"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.metadata.title"]
    );
}

/// A field the call's result type does not declare must derive no accessor, even though some
/// unrelated type in the same crate declares that exact name. This is the hole a bare-name
/// reachability set leaves open: `summary` is a real field *somewhere*, so the flat oracle waved
/// it through and the snippet emitted `result.summary` on a type with no such member. ~keep
#[test]
fn a_field_declared_only_on_an_unrelated_type_derives_no_accessor() {
    let type_defs = vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field("content", TypeRef::String, false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ReportRecord".to_string(),
            fields: vec![field("summary", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ];
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "summary", "value": "Hello"}
    ]));

    let operations = resolve(
        &fixture,
        &config(),
        "python",
        &type_defs,
        &[],
        &convert_returning("ConversionResult"),
    );

    assert!(
        operations.is_empty(),
        "`summary` is declared on ReportRecord, not on the type `convert` returns: {operations:?}"
    );
}

/// The companion direction, which keeps the fix from being a blanket "reject anything the root
/// type doesn't declare": a field the result type DOES declare must still be shown. Without
/// this, every snippet silently reverts to a bare `print(result)`. ~keep
#[test]
fn a_field_the_result_type_declares_still_derives_its_accessor() {
    let type_defs = vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field("content", TypeRef::String, false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ReportRecord".to_string(),
            fields: vec![field("summary", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ];
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "content", "value": "Hello"}
    ]));

    let operations = resolve(
        &fixture,
        &config(),
        "python",
        &type_defs,
        &[],
        &convert_returning("ConversionResult"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.content"]
    );
}

/// With no `functions` registry the call's result type cannot be resolved, so nothing is
/// anchored and every answer must be exactly the pre-fix one. Proves the anchoring is what
/// changes the verdicts above rather than some unrelated tightening, and protects every
/// IR-less snippet entry point from being silently emptied out. ~keep
#[test]
fn an_unresolvable_result_type_keeps_the_unanchored_answers() {
    let type_defs = vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field(
                "metadata",
                TypeRef::Optional(Box::new(TypeRef::Named("Metadata".to_string()))),
                true,
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "LogRecord".to_string(),
            fields: vec![field("metadata", TypeRef::Named("Metadata".to_string()), false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ReportRecord".to_string(),
            fields: vec![field("summary", TypeRef::String, false)],
            ..TypeDef::default()
        },
        metadata_type(),
    ];

    let guarded = resolve(
        &docs_fixture(serde_json::json!([{"type": "equals", "field": "metadata.title", "value": "Hello"}])),
        &config(),
        "node",
        &type_defs,
        &[],
        &[],
    );
    assert_eq!(
        guarded
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.metadata.title"],
        "no resolvable result type means no anchored optionality"
    );

    let unrelated = resolve(
        &docs_fixture(serde_json::json!([{"type": "equals", "field": "summary", "value": "Hello"}])),
        &config(),
        "python",
        &type_defs,
        &[],
        &[],
    );
    assert_eq!(
        unrelated
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.summary"],
        "no resolvable result type means no anchored rejection either"
    );
}

/// The asymmetry, asserted in both directions on ONE resolver and ONE field name.
///
/// ~keep A *hand-authored* assertion path is written by someone looking at the real type, and
/// the oracle legitimately does not recognize virtual namespace prefixes, streaming
/// pseudo-fields or synthetic groupings — so `is_valid_for_result` must keep default-allowing
/// them or real assertion coverage is silently dropped. A *derived* snippet accessor has no
/// such author, so `result_field_oracle_knows` must answer `Some(false)` for the very same
/// name. Collapsing the two into one rule breaks one side or the other; this test fails if
/// either side moves.
#[test]
fn the_availability_oracles_disagree_on_purpose_for_a_name_the_result_type_lacks() {
    let type_defs = vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field("content", TypeRef::String, false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ReportRecord".to_string(),
            fields: vec![field("summary", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ];
    let e2e_config = config();
    let resolver = build_resolver(
        &e2e_config,
        &e2e_config.call,
        "python",
        &type_defs,
        &[],
        &convert_returning("ConversionResult"),
    );

    for absent in ["summary", "rate_limit.min_duration_ms"] {
        assert!(
            resolver.is_valid_for_result(absent),
            "a hand-authored assertion on `{absent}` must still render"
        );
        assert_eq!(
            resolver.result_field_oracle_knows(absent),
            Some(false),
            "an inferred accessor for `{absent}` must be refused"
        );
    }

    assert!(resolver.is_valid_for_result("content"));
    assert_eq!(resolver.result_field_oracle_knows("content"), Some(true));
}

/// A result field whose NAME collides with a legacy streaming pseudo-field (`chunks`,
/// `stream_content`, `tool_calls`, ...) is still a real member when the call is not a streaming
/// call, and must keep its derived accessor.
///
/// ~keep `shows_on_result` rejected every name in `STREAMING_VIRTUAL_FIELDS` by spelling alone,
/// with no streaming gate — the one caller of `is_streaming_virtual_field` in the tree that had
/// none. Every assertion renderer gates that same list on `resolve_is_streaming`, so the two
/// generators disagreed: e2e kept asserting `len(result.chunks) > 0` while the snippet silently
/// dropped the field, across 52 files in one consumer's suite. Both directions are asserted
/// together so neither half can move alone.
#[test]
fn a_result_field_named_like_a_streaming_pseudo_field_still_derives_its_accessor() {
    let type_defs = vec![
        TypeDef {
            name: "SegmentReport".to_string(),
            fields: vec![
                field(
                    "chunks",
                    TypeRef::Vec(Box::new(TypeRef::Named("Segment".to_string()))),
                    false,
                ),
                field("total", TypeRef::String, false),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Segment".to_string(),
            fields: vec![field("text", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ];
    let assertions = serde_json::json!([
        {"type": "length_greater_than", "field": "chunks", "value": 0},
        {"type": "equals", "field": "total", "value": "3"}
    ]);

    let operations = resolve(
        &docs_fixture(assertions.clone()),
        &config(),
        "python",
        &type_defs,
        &[],
        &convert_returning("SegmentReport"),
    );
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.chunks", "result.total"],
        "`chunks` is declared by the type `convert` returns, so the snippet must show it"
    );

    let streaming: Fixture = serde_json::from_value(serde_json::json!({
        "id": "streaming_fixture",
        "description": "Streaming fixture",
        "input": {"html": "<p>Hello</p>"},
        "assertions": assertions,
        "mock_response": {"status": 200, "stream_chunks": ["a", "b"]},
        "docs": {"topic": "smoke", "stem": "streaming_fixture"}
    }))
    .expect("fixture must parse");
    let streamed = resolve(
        &streaming,
        &config(),
        "python",
        &type_defs,
        &[],
        &convert_returning("SegmentReport"),
    );
    assert_eq!(
        streamed
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result_chunk.total"],
        "on a streaming fixture `chunks` names the collected local list, not a result member"
    );
}
