use super::super::{CStreamingAdapterMetadata, render_streaming_test_function};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::{Assertion, Fixture};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn named(name: &str) -> TypeRef {
    TypeRef::Named(name.to_string())
}

/// `StreamChunk { choices: Vec<StreamChoice> }`, `StreamChoice { delta: StreamDelta }` and
/// `StreamDelta { content: Option<String> }`: the shape every chat-completion chunk has.
fn chunk_types(delta_content: TypeRef) -> Vec<TypeDef> {
    chunk_types_with(delta_content, true)
}

fn chunk_types_with(delta_content: TypeRef, finish_and_tools: bool) -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "StreamChunk".to_string(),
            fields: vec![field("choices", TypeRef::Vec(Box::new(named("StreamChoice"))))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "StreamChoice".to_string(),
            fields: [field("delta", named("StreamDelta"))]
                .into_iter()
                .chain(finish_and_tools.then(|| field("finish_reason", optional_text())))
                .collect(),
            ..TypeDef::default()
        },
        TypeDef {
            name: "StreamDelta".to_string(),
            fields: [field("content", delta_content)]
                .into_iter()
                .chain(finish_and_tools.then(|| field("tool_calls", TypeRef::Vec(Box::new(named("StreamToolCall"))))))
                .collect(),
            ..TypeDef::default()
        },
    ]
}

fn optional_text() -> TypeRef {
    TypeRef::Optional(Box::new(TypeRef::String))
}

fn content_assertion(kind: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: kind.into(),
        field: Some("stream_content".into()),
        value,
        ..Assertion::default()
    }
}

fn field_assertion(field: &str, kind: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: kind.into(),
        field: Some(field.into()),
        value,
        ..Assertion::default()
    }
}

fn driver(assertion: Assertion, defs: &[TypeDef], documentation_snippet: bool) -> String {
    let fixture = Fixture {
        id: "stream_with_usage".to_string(),
        description: "stream with usage".to_string(),
        assertions: vec![assertion],
        ..Fixture::default()
    };
    let adapter = CStreamingAdapterMetadata {
        owner_type: "Client".to_string(),
        item_type: "StreamChunk".to_string(),
        request_type: "ChatRequest".to_string(),
        adapter_name: "chat_stream".to_string(),
    };
    let mut out = String::new();
    render_streaming_test_function(
        &mut out,
        &fixture,
        "mylib",
        "chunk",
        &[],
        "client_new",
        &adapter,
        false,
        &[],
        None,
        documentation_snippet,
        defs,
    );
    out
}

/// The defect: the loop freed each chunk without reading a field from it, so a `stream_content`
/// assertion was always a skip. The loop now reads the first choice's delta text through the ABI
/// accessors, appends it to the buffer, and the assertion compares the joined text.
#[test]
fn the_stream_text_is_read_from_each_chunk_delta_and_asserted() {
    let body = driver(
        content_assertion("equals", Some(serde_json::json!("Hi there!"))),
        &chunk_types(optional_text()),
        false,
    );
    for expected in [
        "char *chunk_choices = mylib_stream_chunk_choices(chunk);",
        "alef_json_array_get_index(chunk_choices, 0);",
        "MYLIBAlefHandle first_choice = mylib_stream_choice_from_json(first_choice_json);",
        "MYLIBAlefHandle delta = mylib_stream_choice_delta(first_choice);",
        "char *delta_content = mylib_stream_delta_content(delta);",
        "mylib_free_string(delta_content);",
        "mylib_stream_delta_free(delta);",
        "mylib_stream_choice_free(first_choice);",
        "mylib_free_string(chunk_choices);",
        "assert((strcmp(stream_content, \"Hi there!\") == 0) && \"equals assertion failed for stream_content\");",
    ] {
        assert!(body.contains(expected), "missing `{expected}`. Emitted:\n{body}");
    }
    assert!(
        !body.contains("skipped"),
        "the assertion must not skip. Emitted:\n{body}"
    );
    let read = body.find("mylib_stream_chunk_choices(chunk)").expect("choices read");
    let free = body.find("mylib_stream_chunk_free(chunk)").expect("chunk free");
    assert!(
        read < free,
        "the text must be read before the chunk is freed. Emitted:\n{body}"
    );
}

#[test]
fn contains_and_not_empty_assert_on_the_joined_text() {
    let defs = chunk_types(optional_text());
    let contains = driver(
        content_assertion("contains", Some(serde_json::json!("there"))),
        &defs,
        false,
    );
    assert!(
        contains.contains("assert((strstr(stream_content, \"there\") != NULL) && "),
        "Emitted:\n{contains}"
    );
    let not_empty = driver(content_assertion("not_empty", None), &defs, false);
    assert!(
        not_empty.contains("assert((stream_content_len > 0) && "),
        "Emitted:\n{not_empty}"
    );
}

/// Negative controls: every case below keeps the registered skip and adds no delta plumbing, so
/// the positive test above is not satisfied by accident.
#[test]
fn text_the_ir_cannot_resolve_stays_a_registered_skip() {
    let assertion = || content_assertion("equals", Some(serde_json::json!("Hi there!")));
    let counter_content = chunk_types(TypeRef::Primitive(crate::core::ir::PrimitiveType::U32));
    let mut excluded = chunk_types(optional_text());
    excluded[1].binding_excluded = true;
    for (label, body) in [
        ("no IR", driver(assertion(), &[], false)),
        ("non-text content", driver(assertion(), &counter_content, false)),
        ("excluded choice type", driver(assertion(), &excluded, false)),
        (
            "unsupported assertion type",
            driver(
                content_assertion("regex", Some(serde_json::json!("Hi"))),
                &chunk_types(optional_text()),
                false,
            ),
        ),
    ] {
        assert!(body.contains("/* skipped: "), "{label}. Emitted:\n{body}");
        if label != "unsupported assertion type" {
            assert!(!body.contains("chunk_choices"), "{label}. Emitted:\n{body}");
        }
    }
}

/// A documentation snippet does not include the test runner header that holds the JSON helper
/// the delta read calls, so the snippet keeps no delta plumbing.
#[test]
fn a_documentation_snippet_does_not_read_the_stream_text() {
    let body = driver(
        content_assertion("equals", Some(serde_json::json!("Hi there!"))),
        &chunk_types(optional_text()),
        true,
    );
    assert!(!body.contains("alef_json_array_get_index"), "Emitted:\n{body}");
}

/// The defect: `finish_reason`, `stream_complete`, `tool_calls` and the first call's name were
/// skipped because the loop read nothing from the chunk. The loop now keeps the last non-null
/// finish reason and counts the tool calls, and each assertion compares that state.
#[test]
fn finish_reason_and_tool_calls_are_read_and_asserted() {
    let defs = chunk_types(optional_text());
    let cases = [
        (
            field_assertion("finish_reason", "equals", Some(serde_json::json!("tool_calls"))),
            "assert((stream_finish_reason != NULL && strcmp(stream_finish_reason, \"tool_calls\") == 0) && ",
        ),
        (
            field_assertion("stream_complete", "is_true", None),
            "assert((stream_finish_reason != NULL) && ",
        ),
        (
            field_assertion("tool_calls", "not_empty", None),
            "assert((stream_tool_calls_count > 0) && ",
        ),
        (
            field_assertion(
                "tool_calls[0].function.name",
                "equals",
                Some(serde_json::json!("get_weather")),
            ),
            "assert((stream_tool_call_name != NULL && strcmp(stream_tool_call_name, \"get_weather\") == 0) && ",
        ),
    ];
    for (assertion, expected) in cases {
        let field = assertion.field.clone().unwrap_or_default();
        let body = driver(assertion, &defs, false);
        assert!(
            body.contains(expected),
            "{field}: missing `{expected}`. Emitted:\n{body}"
        );
        assert!(!body.contains("skipped"), "{field}. Emitted:\n{body}");
        assert!(
            body.contains("alef_json_get_string(first_choice_json, \"finish_reason\")")
                || field.starts_with("tool_calls")
        );
        assert!(body.contains("free(stream_finish_reason);") || field.starts_with("tool_calls"));
    }
}

/// Negative control: a choice with no `finish_reason` and a delta with no `tool_calls` keep the
/// registered skip and read nothing.
#[test]
fn a_chunk_without_finish_reason_or_tool_calls_keeps_the_skip() {
    let defs = chunk_types_with(optional_text(), false);
    for assertion in [
        field_assertion("finish_reason", "equals", Some(serde_json::json!("stop"))),
        field_assertion("stream_complete", "is_true", None),
        field_assertion("tool_calls", "not_empty", None),
    ] {
        let body = driver(assertion, &defs, false);
        assert!(body.contains("/* skipped: "), "Emitted:\n{body}");
        assert!(!body.contains("chunk_choices"), "Emitted:\n{body}");
    }
}
