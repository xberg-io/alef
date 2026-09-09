use super::*;
use crate::core::ir::{FunctionDef, TypeRef};

fn fixture(field: &str) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample", "description": "Show the returned value", "input": {},
        "docs": {"topic": "sample"},
        "assertions": [{"type": "not_empty", "field": field}]
    }))
    .expect("fixture")
}

fn config(streaming: bool) -> E2eConfig {
    serde_json::from_value(serde_json::json!({
        "call": {"function": "sample", "result_var": "result", "streaming": streaming},
        "fields_optional": ["usage"]
    }))
    .expect("configuration")
}

#[test]
fn stream_usage_presentation_reads_a_chunk_and_guards_nullable_usage() {
    for (language, expression, guard) in [
        ("go", "resultChunk.Usage.TotalTokens", "resultChunk.Usage != nil"),
        (
            "java",
            "resultChunk.usage().totalTokens()",
            "resultChunk.usage() != null",
        ),
        ("dart", "resultChunk.usage?.totalTokens", ""),
    ] {
        let operations = resolve(&fixture("usage.total_tokens"), &config(true), language, &[], &[], &[]);
        assert_eq!(operations.len(), 1, "{language}");
        assert_eq!(operations[0].expression, expression, "{language}");
        assert_eq!(operations[0].guard_condition, guard, "{language}");
    }
}

#[test]
fn ordinary_usage_remains_rooted_at_the_response() {
    let operations = resolve(&fixture("usage.total_tokens"), &config(false), "go", &[], &[], &[]);
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].expression, "result.Usage.TotalTokens");
}

#[test]
fn native_bytes_do_not_invent_a_content_member_without_a_config_override() {
    let function = FunctionDef {
        name: "sample".into(),
        return_type: TypeRef::Bytes,
        ..FunctionDef::default()
    };
    let operations = resolve(&fixture("content"), &config(false), "go", &[], &[], &[function]);
    assert!(
        operations.is_empty(),
        "bytes are the result, not a result.content wrapper: {operations:?}"
    );
}
