//! Coverage for the indexed-element prefix guard: when an optional array field (e.g.
//! `Option<Vec<T>>`) is indexed by a fixture assertion, the emitted nil/empty guard must
//! target the slice itself, never a value-typed element.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_function::{GoTestFunctionContext, render_test_function};

/// When `segments` is an optional field (Option<Vec<T>>) and a fixture asserts on
/// `segments[0].id`, the prefix guard must be `result.Segments != nil` — NOT
/// `result.Segments[0] != nil`, which is a compile error for a value-typed element.
#[test]
fn test_indexed_element_prefix_guard_uses_array_not_element() {
    let mut optional_fields = std::collections::HashSet::new();
    optional_fields.insert("segments".to_string());
    let mut array_fields = std::collections::HashSet::new();
    array_fields.insert("segments".to_string());

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "transcribe".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_optional: optional_fields,
        fields_array: array_fields,
        ..E2eConfig::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "edge_transcribe_with_timestamps".to_string(),
        category: None,
        description: "Transcription with timestamp segments".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("segments[0].id".to_string()),
                value: Some(serde_json::Value::Number(serde_json::Number::from(0u64))),
                ..Default::default()
            },
        ],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    // Must guard on the slice itself — not on the element. Either the nil check on the
    // optional slice or the non-empty precondition is valid Go here; `len(...) > 0` is
    // not, because it would swallow the assertion instead of failing the test.
    assert!(
        out.contains("result.Segments != nil") || out.contains("len(result.Segments) == 0"),
        "guard must be on Segments (the slice), not an element; got:\n{out}"
    );
    // Must NOT emit the invalid element nil check.
    assert!(
        !out.contains("result.Segments[0] != nil"),
        "must not emit Segments[0] != nil for a value-type element; got:\n{out}"
    );
}
