//! Shared streaming-virtual-fields module for e2e test codegen.
//!
//! Chat-stream fixtures assert on "virtual" fields that don't exist on the
//! stream result type itself — `chunks`, `chunks.length`, `stream_content`,
//! `stream_complete`, `no_chunks_after_done`, `tool_calls`, `finish_reason`.
//! These fields resolve against the *collected* list of chunks produced by
//! draining the stream.
//!
//! [`StreamingFieldResolver`] provides two entry points:
//! - [`StreamingFieldResolver::accessor`] — the language-specific expression
//!   for a virtual field given a local variable that holds the collected list.
//! - [`StreamingFieldResolver::collect_snippet`] — the language-specific
//!   code snippet that drains a stream variable into the collected list.
//!
//! ## Convention
//!
//! The `chunks_var` parameter is the local variable name that holds the
//! collected list (default: `"chunks"`).  The `stream_var` parameter is the
//! result variable produced by the stream call (default: `"result"`).
//!
//! The set of streaming-virtual field names handled by this module:
//! - `chunks`              → the collected list itself
//! - `chunks.length`       → length/count of the collected list
//! - `stream_content`      → concatenation of all delta content strings
//! - `stream_complete`     → boolean — last chunk has a non-null finish_reason
//! - `no_chunks_after_done` → structural invariant (true by construction for
//!   channel/iterator-based APIs once the channel is closed; emitted as
//!   `assert!(true)` / `assertTrue` for languages without post-DONE chunk plumbing)
//! - `tool_calls`          → flat list of tool_calls from all chunk deltas
//! - `finish_reason`       → finish_reason string from the last chunk

/// The set of field names treated as streaming-virtual fields.
pub const STREAMING_VIRTUAL_FIELDS: &[&str] = &[
    "chunks",
    "chunks.length",
    "stream_content",
    "stream_complete",
    "no_chunks_after_done",
    "tool_calls",
    "finish_reason",
];

/// Returns `true` when `field` is a streaming-virtual field name.
pub fn is_streaming_virtual_field(field: &str) -> bool {
    STREAMING_VIRTUAL_FIELDS.contains(&field)
}

/// Shared streaming-virtual-fields resolver for e2e test codegen.
pub struct StreamingFieldResolver;

impl StreamingFieldResolver {
    /// Returns the language-specific expression for a streaming-virtual field,
    /// given `chunks_var` (the collected-list local name) and `lang`.
    ///
    /// Returns `None` when the field name is not a known streaming-virtual
    /// field or the language has no streaming support.
    pub fn accessor(field: &str, lang: &str, chunks_var: &str) -> Option<String> {
        match field {
            "chunks" => Some(chunks_var.to_string()),

            "chunks.length" => Some(match lang {
                "rust" => format!("{chunks_var}.len()"),
                "go" => format!("len({chunks_var})"),
                "python" => format!("len({chunks_var})"),
                "php" => format!("count(${chunks_var})"),
                "elixir" => format!("length({chunks_var})"),
                // kotlin List.size is a property (not .length)
                "kotlin" => format!("{chunks_var}.size"),
                // zig: chunks_var is ArrayList([]u8); use .items.len
                "zig" => format!("{chunks_var}.items.len"),
                // node/wasm/typescript use .length
                _ => format!("{chunks_var}.length"),
            }),

            "stream_content" => Some(match lang {
                "rust" => {
                    format!(
                        "{chunks_var}.iter().map(|c| c.choices.first().and_then(|ch| ch.delta.content.as_deref()).unwrap_or(\"\")).collect::<String>()"
                    )
                }
                "go" => {
                    // Go: chunks is []pkg.ChatCompletionChunk
                    format!(
                        "func() string {{ var s string; for _, c := range {chunks_var} {{ if len(c.Choices) > 0 && c.Choices[0].Delta.Content != nil {{ s += *c.Choices[0].Delta.Content }} }}; return s }}()"
                    )
                }
                "java" => {
                    format!(
                        "{chunks_var}.stream().map(c -> c.choices().stream().findFirst().map(ch -> ch.delta().content() != null ? ch.delta().content() : \"\").orElse(\"\")).collect(java.util.stream.Collectors.joining())"
                    )
                }
                "php" => {
                    format!("implode('', array_map(fn($c) => $c->choices[0]->delta->content ?? '', ${chunks_var}))")
                }
                "kotlin" => {
                    // Kotlin: chunks is List<ChatCompletionChunk> (Java records via typealias).
                    // choices() / delta() / content() are Java record accessor methods.
                    format!(
                        "{chunks_var}.joinToString(\"\") {{ it.choices()?.firstOrNull()?.delta()?.content() ?: \"\" }}"
                    )
                }
                "elixir" => {
                    format!(
                        "{chunks_var} |> Enum.map(&(&1.choices[0].delta.content || \"\")) |> Enum.join(\"\")"
                    )
                }
                "python" => {
                    format!(
                        "\"\".join(c.choices[0].delta.content or \"\" for c in {chunks_var} if c.choices)"
                    )
                }
                "zig" => {
                    // Zig: `{chunks_var}_content` is a `std.ArrayList(u8)` populated by
                    // the collect snippet. `.items` gives a `[]u8` slice of the content.
                    format!("{chunks_var}_content.items")
                }
                // node/wasm/typescript
                _ => {
                    format!("{chunks_var}.map((c: any) => c.choices?.[0]?.delta?.content ?? '').join('')")
                }
            }),

            "stream_complete" => Some(match lang {
                "rust" => {
                    format!(
                        "{chunks_var}.last().and_then(|c| c.choices.first()).and_then(|ch| ch.finish_reason.as_ref()).is_some()"
                    )
                }
                "go" => {
                    format!(
                        "func() bool {{ if len({chunks_var}) == 0 {{ return false }}; last := {chunks_var}[len({chunks_var})-1]; return len(last.Choices) > 0 && last.Choices[0].FinishReason != nil }}()"
                    )
                }
                "java" => {
                    format!(
                        "!{chunks_var}.isEmpty() && {chunks_var}.get({chunks_var}.size()-1).choices().stream().findFirst().flatMap(ch -> java.util.Optional.ofNullable(ch.finishReason())).isPresent()"
                    )
                }
                "php" => {
                    format!("!empty(${chunks_var}) && isset(end(${chunks_var})->choices[0]->finishReason)")
                }
                "kotlin" => {
                    // Kotlin: use isNotEmpty() + last() + safe-call chain
                    format!(
                        "{chunks_var}.isNotEmpty() && {chunks_var}.last().choices()?.firstOrNull()?.finishReason() != null"
                    )
                }
                "python" => {
                    format!(
                        "bool({chunks_var}) and {chunks_var}[-1].choices[0].finish_reason is not None"
                    )
                }
                "elixir" => {
                    format!("List.last({chunks_var}).choices[0].finish_reason != nil")
                }
                // node/wasm/typescript
                _ => {
                    format!(
                        "{chunks_var}.length > 0 && {chunks_var}[{chunks_var}.length - 1].choices?.[0]?.finishReason != null"
                    )
                }
            }),

            // no_chunks_after_done is a structural invariant: once the stream
            // closes (channel drained / iterator exhausted), no further chunks
            // can arrive.  We assert `true` as a compile-time proof of intent.
            "no_chunks_after_done" => Some(match lang {
                "rust" => "true".to_string(),
                "go" => "true".to_string(),
                "java" => "true".to_string(),
                "php" => "true".to_string(),
                _ => "true".to_string(),
            }),

            "tool_calls" => Some(match lang {
                "rust" => {
                    format!(
                        "{chunks_var}.iter().flat_map(|c| c.choices.iter().flat_map(|ch| ch.delta.tool_calls.iter().flatten())).collect::<Vec<_>>()"
                    )
                }
                "go" => {
                    format!(
                        "func() []interface{{}} {{ var tc []interface{{}}; for _, c := range {chunks_var} {{ for _, ch := range c.Choices {{ if ch.Delta.ToolCalls != nil {{ for _, t := range *ch.Delta.ToolCalls {{ tc = append(tc, t) }} }} }} }}; return tc }}()"
                    )
                }
                "java" => {
                    format!(
                        "{chunks_var}.stream().flatMap(c -> c.choices().stream()).flatMap(ch -> ch.delta().toolCalls() != null ? ch.delta().toolCalls().stream() : java.util.stream.Stream.empty()).toList()"
                    )
                }
                "php" => {
                    format!(
                        "array_merge(...array_map(fn($c) => $c->choices[0]->delta->toolCalls ?? [], ${chunks_var}))"
                    )
                }
                "kotlin" => {
                    // Kotlin: flatten tool_calls from all chunk deltas into one list
                    format!(
                        "{chunks_var}.flatMap {{ c -> c.choices()?.flatMap {{ ch -> ch.delta()?.toolCalls() ?: emptyList() }} ?: emptyList() }}"
                    )
                }
                "python" => {
                    format!(
                        "[t for c in {chunks_var} for ch in (c.choices or []) for t in (ch.delta.tool_calls or [])]"
                    )
                }
                "elixir" => {
                    format!(
                        "{chunks_var} |> Enum.flat_map(fn c -> (List.first(c.choices) || %{{}}).delta |> Map.get(:tool_calls, []) end)"
                    )
                }
                _ => {
                    format!("{chunks_var}.flatMap((c: any) => c.choices?.[0]?.delta?.toolCalls ?? [])")
                }
            }),

            "finish_reason" => Some(match lang {
                "rust" => {
                    format!(
                        "{chunks_var}.last().and_then(|c| c.choices.first()).and_then(|ch| ch.finish_reason.as_deref()).unwrap_or(\"\")"
                    )
                }
                "go" => {
                    format!(
                        "func() string {{ if len({chunks_var}) == 0 {{ return \"\" }}; last := {chunks_var}[len({chunks_var})-1]; if len(last.Choices) > 0 && last.Choices[0].FinishReason != nil {{ return *last.Choices[0].FinishReason }}; return \"\" }}()"
                    )
                }
                "java" => {
                    format!(
                        "({chunks_var}.isEmpty() ? null : {chunks_var}.get({chunks_var}.size()-1).choices().stream().findFirst().map(ch -> ch.finishReason()).orElse(null))"
                    )
                }
                "php" => {
                    format!("(!empty(${chunks_var}) ? (end(${chunks_var})->choices[0]->finishReason ?? null) : null)")
                }
                "kotlin" => {
                    // Returns the string value of the finish_reason enum from the last chunk.
                    // FinishReason.getValue() returns the JSON wire string (e.g. "tool_calls").
                    format!(
                        "(if ({chunks_var}.isEmpty()) null else {chunks_var}.last().choices()?.firstOrNull()?.finishReason()?.getValue())"
                    )
                }
                "python" => {
                    format!(
                        "({chunks_var}[-1].choices[0].finish_reason if {chunks_var} and {chunks_var}[-1].choices else None)"
                    )
                }
                "elixir" => {
                    format!("List.last({chunks_var}).choices[0].finish_reason")
                }
                _ => {
                    format!(
                        "{chunks_var}.length > 0 ? {chunks_var}[{chunks_var}.length - 1].choices?.[0]?.finishReason : undefined"
                    )
                }
            }),

            _ => None,
        }
    }

    /// Returns the language-specific stream-collect-into-list snippet that
    /// produces `chunks_var` from `stream_var`.
    ///
    /// Returns `None` when the language has no streaming collect support or
    /// when the collect snippet cannot be expressed generically.
    pub fn collect_snippet(lang: &str, stream_var: &str, chunks_var: &str) -> Option<String> {
        match lang {
            "rust" => Some(format!(
                "let {chunks_var}: Vec<_> = tokio_stream::StreamExt::collect::<Vec<_>>({stream_var}).await;"
            )),
            "go" => Some(format!(
                "var {chunks_var} []pkg.ChatCompletionChunk\n\tfor chunk := range {stream_var} {{\n\t\t{chunks_var} = append({chunks_var}, chunk)\n\t}}"
            )),
            "java" => Some(format!(
                "var {chunks_var} = new java.util.ArrayList<ChatCompletionChunk>();\n        var _it = {stream_var};\n        while (_it.hasNext()) {{ {chunks_var}.add(_it.next()); }}"
            )),
            "php" => Some(format!("${chunks_var} = iterator_to_array(${stream_var});")),
            "python" => Some(format!(
                "{chunks_var} = []\n    async for chunk in {stream_var}:\n        {chunks_var}.append(chunk)"
            )),
            "kotlin" => {
                // Kotlin: chatStream returns Iterator<ChatCompletionChunk> (from Java bridge).
                // Drain into a Kotlin List using asSequence().toList().
                Some(format!(
                    "val {chunks_var} = {stream_var}.asSequence().toList()"
                ))
            }
            "elixir" => Some(format!("{chunks_var} = Enum.to_list({stream_var})")),
            "node" | "wasm" | "typescript" => Some(format!(
                "const {chunks_var}: any[] = [];\n    for await (const _chunk of {stream_var}) {{ {chunks_var}.push(_chunk); }}"
            )),
            "zig" => {
                // Zig: drain the stream handle (opaque *LITERLLMChatStreamHandle) via
                // the _next/_free FFI NIFs exposed through `liter_llm.c.*`.
                // `stream_var` is the opaque stream handle already obtained via `_start`.
                // We collect every chunk's JSON string into `chunks_var: ArrayList([]u8)`
                // and concatenate delta content into `{chunks_var}_content: ArrayList(u8)`.
                // Accessors use `.items.len` and `{chunks_var}_content.items` on these lists.
                Some(format!(
                    concat!(
                        "var {chunks_var} = std.ArrayList([]u8).init(std.heap.c_allocator);
",
                        "    defer {{
",
                        "        for ({chunks_var}.items) |_cj| std.heap.c_allocator.free(_cj);
",
                        "        {chunks_var}.deinit();
",
                        "    }}
",
                        "    var {chunks_var}_content = std.ArrayList(u8).init(std.heap.c_allocator);
",
                        "    defer {chunks_var}_content.deinit();
",
                        "    while (true) {{
",
                        "        const _nc = liter_llm.c.literllm_default_client_chat_stream_next({stream_var});
",
                        "        if (_nc == null) break;
",
                        "        const _np = liter_llm.c.literllm_chat_completion_chunk_to_json(_nc);
",
                        "        liter_llm.c.literllm_chat_completion_chunk_free(_nc);
",
                        "        if (_np == null) continue;
",
                        "        const _ns = std.mem.span(_np);
",
                        "        const _nj = try std.heap.c_allocator.dupe(u8, _ns);
",
                        "        liter_llm.c.literllm_free_string(_np);
",
                        "        if (std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, _nj, .{{}})) |_cp| {{
",
                        "            defer _cp.deinit();
",
                        "            if (_cp.value.object.get(\"choices\")) |_chs|
",
                        "                if (_chs.array.items.len > 0)
",
                        "                    if (_chs.array.items[0].object.get(\"delta\")) |_dl|
",
                        "                        if (_dl.object.get(\"content\")) |_ct|
",
                        "                            if (_ct == .string) try {chunks_var}_content.appendSlice(_ct.string);
",
                        "        }} else |_| {{}}
",
                        "        try {chunks_var}.append(_nj);
",
                        "    }}"
                    ),
                    chunks_var = chunks_var,
                    stream_var = stream_var,
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_streaming_virtual_field_recognizes_all_fields() {
        for field in STREAMING_VIRTUAL_FIELDS {
            assert!(
                is_streaming_virtual_field(field),
                "field '{field}' not recognized as streaming virtual"
            );
        }
    }

    #[test]
    fn is_streaming_virtual_field_rejects_real_fields() {
        assert!(!is_streaming_virtual_field("content"));
        assert!(!is_streaming_virtual_field("choices"));
        assert!(!is_streaming_virtual_field("model"));
        assert!(!is_streaming_virtual_field(""));
    }

    #[test]
    fn accessor_chunks_returns_var_name() {
        assert_eq!(
            StreamingFieldResolver::accessor("chunks", "rust", "chunks"),
            Some("chunks".to_string())
        );
        assert_eq!(
            StreamingFieldResolver::accessor("chunks", "node", "chunks"),
            Some("chunks".to_string())
        );
    }

    #[test]
    fn accessor_chunks_length_uses_language_idiom() {
        let rust = StreamingFieldResolver::accessor("chunks.length", "rust", "chunks").unwrap();
        assert!(rust.contains(".len()"), "rust: {rust}");

        let go = StreamingFieldResolver::accessor("chunks.length", "go", "chunks").unwrap();
        assert!(go.starts_with("len("), "go: {go}");

        let node = StreamingFieldResolver::accessor("chunks.length", "node", "chunks").unwrap();
        assert!(node.contains(".length"), "node: {node}");

        let php = StreamingFieldResolver::accessor("chunks.length", "php", "chunks").unwrap();
        assert!(php.starts_with("count("), "php: {php}");
    }

    #[test]
    fn accessor_chunks_length_zig_uses_items_len() {
        let zig = StreamingFieldResolver::accessor("chunks.length", "zig", "chunks").unwrap();
        assert_eq!(zig, "chunks.items.len", "zig chunks.length: {zig}");
    }

    #[test]
    fn accessor_stream_content_zig_uses_content_items() {
        let zig = StreamingFieldResolver::accessor("stream_content", "zig", "chunks").unwrap();
        assert_eq!(zig, "chunks_content.items", "zig stream_content: {zig}");
    }

    #[test]
    fn collect_snippet_zig_drains_via_ffi() {
        let snip = StreamingFieldResolver::collect_snippet("zig", "_stream_handle", "chunks").unwrap();
        assert!(snip.contains("std.ArrayList([]u8)"), "zig collect: {snip}");
        assert!(snip.contains("chat_stream_next(_stream_handle)"), "zig collect: {snip}");
        assert!(snip.contains("chunks_content"), "zig collect: {snip}");
        assert!(snip.contains("chunks.append"), "zig collect: {snip}");
    }

    #[test]
    fn accessor_stream_content_rust_uses_iterator() {
        let expr = StreamingFieldResolver::accessor("stream_content", "rust", "chunks").unwrap();
        assert!(expr.contains(".collect::<String>()"), "rust stream_content: {expr}");
    }

    #[test]
    fn accessor_no_chunks_after_done_returns_true() {
        for lang in ["rust", "go", "java", "php", "node", "wasm", "elixir"] {
            let expr = StreamingFieldResolver::accessor("no_chunks_after_done", lang, "chunks").unwrap();
            assert_eq!(expr, "true", "lang {lang}: expected 'true', got '{expr}'");
        }
    }

    #[test]
    fn accessor_elixir_chunks_length_uses_length_function() {
        let expr = StreamingFieldResolver::accessor("chunks.length", "elixir", "chunks").unwrap();
        assert_eq!(expr, "length(chunks)", "elixir chunks.length: {expr}");
    }

    #[test]
    fn accessor_elixir_stream_content_uses_pipe() {
        let expr = StreamingFieldResolver::accessor("stream_content", "elixir", "chunks").unwrap();
        assert!(expr.contains("|> Enum.join"), "elixir stream_content: {expr}");
        assert!(expr.contains("|> Enum.map"), "elixir stream_content: {expr}");
    }

    #[test]
    fn accessor_elixir_stream_complete_uses_list_last() {
        let expr = StreamingFieldResolver::accessor("stream_complete", "elixir", "chunks").unwrap();
        assert!(expr.contains("List.last(chunks)"), "elixir stream_complete: {expr}");
        assert!(expr.contains("finish_reason != nil"), "elixir stream_complete: {expr}");
    }

    #[test]
    fn accessor_elixir_finish_reason_uses_list_last() {
        let expr = StreamingFieldResolver::accessor("finish_reason", "elixir", "chunks").unwrap();
        assert!(expr.contains("List.last(chunks)"), "elixir finish_reason: {expr}");
        assert!(expr.contains("finish_reason"), "elixir finish_reason: {expr}");
    }

    #[test]
    fn collect_snippet_elixir_uses_enum_to_list() {
        let snip = StreamingFieldResolver::collect_snippet("elixir", "result", "chunks").unwrap();
        assert!(snip.contains("Enum.to_list(result)"), "elixir: {snip}");
        assert!(snip.contains("chunks ="), "elixir: {snip}");
    }

    #[test]
    fn collect_snippet_rust_uses_tokio_stream() {
        let snip = StreamingFieldResolver::collect_snippet("rust", "result", "chunks").unwrap();
        assert!(snip.contains("tokio_stream::StreamExt::collect"), "rust: {snip}");
        assert!(snip.contains("let chunks"), "rust: {snip}");
    }

    #[test]
    fn collect_snippet_go_drains_channel() {
        let snip = StreamingFieldResolver::collect_snippet("go", "stream", "chunks").unwrap();
        assert!(snip.contains("for chunk := range stream"), "go: {snip}");
    }

    #[test]
    fn collect_snippet_java_uses_iterator() {
        let snip = StreamingFieldResolver::collect_snippet("java", "result", "chunks").unwrap();
        assert!(snip.contains("hasNext()"), "java: {snip}");
    }

    #[test]
    fn collect_snippet_php_uses_iterator_to_array() {
        let snip = StreamingFieldResolver::collect_snippet("php", "result", "chunks").unwrap();
        assert!(snip.contains("iterator_to_array"), "php: {snip}");
    }

    #[test]
    fn collect_snippet_node_uses_for_await() {
        let snip = StreamingFieldResolver::collect_snippet("node", "result", "chunks").unwrap();
        assert!(snip.contains("for await"), "node: {snip}");
    }

    #[test]
    fn collect_snippet_python_uses_async_for() {
        let snip = StreamingFieldResolver::collect_snippet("python", "result", "chunks").unwrap();
        assert!(snip.contains("async for chunk in result"), "python: {snip}");
        assert!(snip.contains("chunks.append(chunk)"), "python: {snip}");
    }

    #[test]
    fn accessor_stream_content_python_uses_join() {
        let expr = StreamingFieldResolver::accessor("stream_content", "python", "chunks").unwrap();
        assert!(expr.contains("\"\".join("), "python stream_content: {expr}");
        assert!(expr.contains("c.choices"), "python stream_content: {expr}");
    }

    #[test]
    fn accessor_stream_complete_python_uses_finish_reason() {
        let expr = StreamingFieldResolver::accessor("stream_complete", "python", "chunks").unwrap();
        assert!(expr.contains("finish_reason is not None"), "python stream_complete: {expr}");
    }

    #[test]
    fn accessor_finish_reason_python_uses_last_chunk() {
        let expr = StreamingFieldResolver::accessor("finish_reason", "python", "chunks").unwrap();
        assert!(expr.contains("chunks[-1]"), "python finish_reason: {expr}");
    }

    #[test]
    fn accessor_tool_calls_python_uses_list_comprehension() {
        let expr = StreamingFieldResolver::accessor("tool_calls", "python", "chunks").unwrap();
        assert!(expr.contains("for c in chunks"), "python tool_calls: {expr}");
        assert!(expr.contains("tool_calls"), "python tool_calls: {expr}");
    }

    #[test]
    fn accessor_unknown_field_returns_none() {
        assert_eq!(
            StreamingFieldResolver::accessor("nonexistent_field", "rust", "chunks"),
            None
        );
    }
}
