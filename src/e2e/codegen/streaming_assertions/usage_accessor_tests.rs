//! Tests for the stream-scoped `usage` root: the accessor each backend gets for
//! `usage.total_tokens`, and the predicates that decide when a call may claim the path.
//!
//! A separate file from `tests.rs`, which sits just under the 1,000-line cap.

use super::*;

#[test]
fn accessor_usage_total_tokens_reads_the_last_chunk() {
    // `usage.total_tokens` is a stream-scoped path: on a streaming call it reads the counter off
    // the last chunk. A non-streaming call never reaches the accessor (see
    // `is_stream_scoped_field_is_not_a_virtual_field`).
    let python = StreamingFieldResolver::accessor("usage.total_tokens", "python", "chunks").unwrap();
    assert_eq!(python, "(chunks[-1].usage if chunks else None).total_tokens");
    let node = StreamingFieldResolver::accessor("usage.total_tokens", "node", "chunks").unwrap();
    assert!(
        node.contains("chunks[chunks.length - 1].usage") && node.ends_with(".totalTokens"),
        "node: {node}"
    );
    let rust = StreamingFieldResolver::accessor("usage.total_tokens", "rust", "chunks").unwrap();
    assert!(
        rust.starts_with("chunks.last().and_then(|c| c.usage.as_ref()).expect(") && rust.ends_with(".total_tokens"),
        "rust must unwrap the Option before reading the counter: {rust}"
    );
    let java = StreamingFieldResolver::accessor("usage.total_tokens", "java", "chunks").unwrap();
    assert!(java.ends_with(".totalTokens()"), "java: {java}");
    let php = StreamingFieldResolver::accessor("usage.total_tokens", "php", "chunks").unwrap();
    assert!(
        php.ends_with("->totalTokens"),
        "php chunks are camelCased by the collect snippet: {php}"
    );
    let elixir = StreamingFieldResolver::accessor("usage.total_tokens", "elixir", "chunks").unwrap();
    assert!(elixir.ends_with(".total_tokens"), "elixir: {elixir}");
}

/// Go returns the raw counter. The `int` widening a fixture literal needs is applied by the Go
/// assertion renderer, and only to an assertion that compares the counter with a number.
#[test]
fn accessor_usage_total_tokens_in_go_is_not_widened() {
    let go = StreamingFieldResolver::accessor("usage.total_tokens", "go", "chunks").unwrap();
    assert_eq!(go, "chunks[len(chunks)-1].Usage.TotalTokens");
}

/// Swift and Dart read `usage` as an optional on the last chunk, so every step chains with `?.`.
/// The generic tail renderer would append `.totalTokens` to an optional, which does not compile.
#[test]
fn accessor_usage_total_tokens_chains_optionals_in_swift_and_dart() {
    let swift = StreamingFieldResolver::accessor("usage.total_tokens", "swift", "chunks").unwrap();
    assert_eq!(swift, "chunks.last?.usage?.totalTokens");
    let dart = StreamingFieldResolver::accessor("usage.total_tokens", "dart", "result").unwrap();
    assert_eq!(dart, "result.lastOrNull?.usage?.totalTokens");
}

#[test]
fn optional_chain_tail_camel_cases_each_field_and_chains_indexes() {
    assert_eq!(
        super::renderers::render_optional_chain_tail("root", ".prompt_tokens_details.cached_tokens"),
        "root?.promptTokensDetails?.cachedTokens"
    );
    assert_eq!(
        super::renderers::render_optional_chain_tail("root", "[0].id"),
        "root?[0]?.id"
    );
}

#[test]
fn is_stream_scoped_field_is_not_a_virtual_field() {
    assert!(is_stream_scoped_field("usage"));
    assert!(is_stream_scoped_field("usage.total_tokens"));
    assert!(is_stream_scoped_field("usage[0]"));
    assert!(!is_stream_scoped_field("usages"));
    assert!(!is_stream_scoped_field("tool_calls"));
    // The context-free predicate must keep refusing `usage`: a non-streaming response carries it.
    assert!(!is_streaming_virtual_field("usage.total_tokens"));
    // The streaming-call predicate accepts both families.
    assert!(is_streaming_call_field("usage.total_tokens"));
    assert!(is_streaming_call_field("tool_calls[0].id"));
    assert!(!is_streaming_call_field("choices[0].message.content"));
}

/// The widening a backend applies to a counter read is gated on a number-valued assertion on a
/// path with a tail. A string-valued assertion and the bare `usage` root keep the field type.
#[test]
fn a_counter_assertion_needs_a_stream_scoped_path_with_a_tail_and_a_number() {
    let number = serde_json::json!(18);
    let text = serde_json::json!("gpt-4");
    assert!(is_stream_counter_assertion("usage.total_tokens", Some(&number)));
    assert!(!is_stream_counter_assertion("usage.model", Some(&text)));
    assert!(!is_stream_counter_assertion("usage", Some(&number)));
    assert!(!is_stream_counter_assertion("usage.total_tokens", None));
    assert!(!is_stream_counter_assertion("chunks.length", Some(&number)));
}
