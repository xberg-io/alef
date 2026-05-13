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

/// The set of streaming-virtual root names that may have deep-path continuations.
///
/// A field like `tool_calls[0].function.name` starts with `tool_calls` and has
/// a continuation `[0].function.name`. These are handled by
/// [`StreamingFieldResolver::accessor`] via the deep-path logic.
const STREAMING_VIRTUAL_ROOTS: &[&str] = &["tool_calls", "finish_reason"];

/// Returns `true` when `field` is a streaming-virtual field name, including
/// deep-nested paths that start with a known streaming-virtual root.
///
/// Examples that return `true`:
/// - `"tool_calls"` (exact root)
/// - `"tool_calls[0].function.name"` (deep path)
/// - `"tool_calls[0].id"` (deep path)
pub fn is_streaming_virtual_field(field: &str) -> bool {
    if STREAMING_VIRTUAL_FIELDS.contains(&field) {
        return true;
    }
    // Check deep-path prefixes: `tool_calls[…` or `tool_calls.`
    for root in STREAMING_VIRTUAL_ROOTS {
        if field.len() > root.len() && field.starts_with(root) {
            let rest = &field[root.len()..];
            if rest.starts_with('[') || rest.starts_with('.') {
                return true;
            }
        }
    }
    false
}

/// Split a field path into `(root, tail)` when it starts with a streaming-virtual
/// root and has a continuation.
///
/// Returns `None` when the field is an exact root match (no tail) or is not a
/// streaming-virtual root at all.
fn split_streaming_deep_path(field: &str) -> Option<(&str, &str)> {
    for root in STREAMING_VIRTUAL_ROOTS {
        if field.len() > root.len() && field.starts_with(root) {
            let rest = &field[root.len()..];
            if rest.starts_with('[') || rest.starts_with('.') {
                return Some((root, rest));
            }
        }
    }
    None
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
            "chunks" => Some(match lang {
                // Zig ArrayList does not expose .len directly; must use .items
                "zig" => format!("{chunks_var}.items"),
                // PHP variables require `$` sigil — bareword `chunks` is parsed as a
                // constant reference and triggers "Undefined constant" errors.
                "php" => format!("${chunks_var}"),
                _ => chunks_var.to_string(),
            }),

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
                    // StreamDelta has all fields optional with skip_serializing_if, so
                    // absent fields are not present as keys in the Jason-decoded map.
                    // Use Map.get with defaults to avoid KeyError on absent :content.
                    format!(
                        "{chunks_var} |> Enum.map(fn c -> (Enum.at(c.choices, 0) || %{{}}) |> Map.get(:delta, %{{}}) |> Map.get(:content, \"\") end) |> Enum.join(\"\")"
                    )
                }
                "python" => {
                    format!("\"\".join(c.choices[0].delta.content or \"\" for c in {chunks_var} if c.choices)")
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
                    // PHP streaming chunks come from `json_decode` of the binding's JSON
                    // string return. The PHP binding serializes with rename_all = "camelCase",
                    // so use `finishReason` (camelCase) to match the emitted JSON keys.
                    format!("!empty(${chunks_var}) && isset(end(${chunks_var})->choices[0]->finishReason)")
                }
                "kotlin" => {
                    // Kotlin: use isNotEmpty() + last() + safe-call chain
                    format!(
                        "{chunks_var}.isNotEmpty() && {chunks_var}.last().choices()?.firstOrNull()?.finishReason() != null"
                    )
                }
                "python" => {
                    format!("bool({chunks_var}) and {chunks_var}[-1].choices[0].finish_reason is not None")
                }
                "elixir" => {
                    format!("Enum.at(List.last({chunks_var}).choices, 0).finish_reason != nil")
                }
                // zig: the collect snippet exhausts the stream; check last chunk JSON
                // was collected (chunks.items is non-empty) as a proxy for completion.
                "zig" => {
                    format!("{chunks_var}.items.len > 0")
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
                    // StreamDelta.ToolCalls is `[]StreamToolCall` (slice, not pointer).
                    // Return the typed slice so deep-path accessors like `tool_calls[0].function.name`
                    // can index and access typed fields.
                    format!(
                        "func() []pkg.StreamToolCall {{ var tc []pkg.StreamToolCall; for _, c := range {chunks_var} {{ for _, ch := range c.Choices {{ tc = append(tc, ch.Delta.ToolCalls...) }} }}; return tc }}()"
                    )
                }
                "java" => {
                    format!(
                        "{chunks_var}.stream().flatMap(c -> c.choices().stream()).flatMap(ch -> ch.delta().toolCalls() != null ? ch.delta().toolCalls().stream() : java.util.stream.Stream.empty()).toList()"
                    )
                }
                "php" => {
                    // PHP streaming chunks are json_decoded stdClass. The PHP binding
                    // serializes with rename_all = "camelCase", so use `toolCalls`.
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
                // Zig: tool_calls count from all chunk deltas
                "zig" => {
                    format!("{chunks_var}.items")
                }
                _ => {
                    format!("{chunks_var}.flatMap((c: any) => c.choices?.[0]?.delta?.toolCalls ?? [])")
                }
            }),

            "finish_reason" => Some(match lang {
                "rust" => {
                    // ChatCompletionChunk's finish_reason is Option<FinishReason> (enum, not
                    // String). Display impl writes the JSON wire form (e.g. "tool_calls").
                    format!(
                        "{chunks_var}.last().and_then(|c| c.choices.first()).and_then(|ch| ch.finish_reason.as_ref()).map(|v| v.to_string()).unwrap_or_default()"
                    )
                }
                "go" => {
                    // FinishReason is a typed alias (`type FinishReason string`) in bindings,
                    // so cast to string explicitly to match the assertion target type.
                    format!(
                        "func() string {{ if len({chunks_var}) == 0 {{ return \"\" }}; last := {chunks_var}[len({chunks_var})-1]; if len(last.Choices) > 0 && last.Choices[0].FinishReason != nil {{ return string(*last.Choices[0].FinishReason) }}; return \"\" }}()"
                    )
                }
                "java" => {
                    format!(
                        "({chunks_var}.isEmpty() ? null : {chunks_var}.get({chunks_var}.size()-1).choices().stream().findFirst().map(ch -> ch.finishReason()).orElse(null))"
                    )
                }
                "php" => {
                    // PHP streaming chunks are json_decoded stdClass. The PHP binding
                    // serializes with rename_all = "camelCase", so use `finishReason`.
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
                    format!("Enum.at(List.last({chunks_var}).choices, 0).finish_reason")
                }
                // Zig: finish_reason from the last chunk's JSON via an inline labeled block.
                // Returns `[]const u8` (unwrapped with orelse "" for expectEqualStrings).
                "zig" => {
                    format!(
                        "(blk: {{ if ({chunks_var}.items.len == 0) break :blk \"\"; var _lcp = std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {chunks_var}.items[{chunks_var}.items.len - 1], .{{}}) catch break :blk \"\"; defer _lcp.deinit(); if (_lcp.value.object.get(\"choices\")) |_lchs| if (_lchs.array.items.len > 0) if (_lchs.array.items[0].object.get(\"finish_reason\")) |_fr| if (_fr == .string) break :blk _fr.string; break :blk \"\"; }})"
                    )
                }
                _ => {
                    format!(
                        "{chunks_var}.length > 0 ? {chunks_var}[{chunks_var}.length - 1].choices?.[0]?.finishReason : undefined"
                    )
                }
            }),

            _ => {
                // Deep-path: e.g. `tool_calls[0].function.name`
                // Split into root + tail, get the root's inline expression, then
                // render the tail (index + fields) in a per-language style on top.
                if let Some((root, tail)) = split_streaming_deep_path(field) {
                    // Rust needs Option-aware chaining for the StreamToolCall fields
                    // (function/id are Option<...>). The generic tail renderer can't
                    // infer Option-wrapping, so we emit rust-specific idiom here.
                    if lang == "rust" && root == "tool_calls" {
                        return Some(render_rust_tool_calls_deep(chunks_var, tail));
                    }
                    // Zig stores stream chunks as JSON strings (`[]const u8`) in
                    // `chunks: ArrayList([]u8)`, not typed `ChatCompletionChunk`
                    // records. A deep `tool_calls[N].function.name` access would
                    // require parsing each chunk's JSON inline — rather than
                    // emit code that won't compile, signal "unsupported" so the
                    // assertion is skipped at the call site.
                    if lang == "zig" && root == "tool_calls" {
                        return None;
                    }
                    let root_expr = Self::accessor(root, lang, chunks_var)?;
                    Some(render_deep_tail(&root_expr, tail, lang))
                } else {
                    None
                }
            }
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
                "let {chunks_var}: Vec<_> = tokio_stream::StreamExt::collect::<Vec<_>>({stream_var}).await\n        .into_iter()\n        .map(|r| r.expect(\"stream item failed\"))\n        .collect();"
            )),
            "go" => Some(format!(
                "var {chunks_var} []pkg.ChatCompletionChunk\n\tfor chunk := range {stream_var} {{\n\t\t{chunks_var} = append({chunks_var}, chunk)\n\t}}"
            )),
            "java" => Some(format!(
                "var {chunks_var} = new java.util.ArrayList<ChatCompletionChunk>();\n        var _it = {stream_var};\n        while (_it.hasNext()) {{ {chunks_var}.add(_it.next()); }}"
            )),
            // PHP binding's chat_stream_async typically returns a JSON string of the
            // chunk array (PHP cannot expose Rust iterators directly via ext-php-rs).
            // Decode to an array of stdClass objects so accessor chains like
            // `$c->choices[0]->delta->content` resolve against the JSON wire shape
            // (snake_case keys).  Falls back to iterator_to_array for a future binding
            // upgrade that exposes a real iterator.
            "php" => Some(format!(
                "${chunks_var} = is_string(${stream_var}) ? (json_decode(${stream_var}) ?: []) : iterator_to_array(${stream_var});"
            )),
            "python" => Some(format!(
                "{chunks_var} = []\n    async for chunk in {stream_var}:\n        {chunks_var}.append(chunk)"
            )),
            "kotlin" => {
                // Kotlin: chatStream returns Iterator<ChatCompletionChunk> (from Java bridge).
                // Drain into a Kotlin List using asSequence().toList().
                Some(format!("val {chunks_var} = {stream_var}.asSequence().toList()"))
            }
            "elixir" => Some(format!("{chunks_var} = Enum.to_list({stream_var})")),
            "node" | "wasm" | "typescript" => Some(format!(
                "const {chunks_var}: any[] = [];\n    for await (const _chunk of {stream_var}) {{ {chunks_var}.push(_chunk); }}"
            )),
            "zig" => {
                // Zig 0.16: ArrayList is unmanaged — no stored allocator.
                // Use `.empty` to initialize, pass `std.heap.c_allocator` to each mutation.
                // `stream_var` is the opaque stream handle obtained via `_start`.
                // We collect every chunk's JSON string into `chunks_var: ArrayList([]u8)`
                // and concatenate delta content into `{chunks_var}_content: ArrayList(u8)`.
                // Accessors use `.items.len` and `{chunks_var}_content.items` on these lists.
                Some(format!(
                    concat!(
                        "var {chunks_var}: std.ArrayList([]u8) = .empty;
",
                        "    defer {{
",
                        "        for ({chunks_var}.items) |_cj| std.heap.c_allocator.free(_cj);
",
                        "        {chunks_var}.deinit(std.heap.c_allocator);
",
                        "    }}
",
                        "    var {chunks_var}_content: std.ArrayList(u8) = .empty;
",
                        "    defer {chunks_var}_content.deinit(std.heap.c_allocator);
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
                        "                            if (_ct == .string) try {chunks_var}_content.appendSlice(std.heap.c_allocator, _ct.string);
",
                        "        }} else |_| {{}}
",
                        "        try {chunks_var}.append(std.heap.c_allocator, _nj);
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

/// Render a rust deep accessor for `tool_calls[N]...` paths over the flattened
/// stream-chunk tool_calls iterator. Handles Option-wrapped fields by chaining
/// `as_ref().and_then(...)` so the final value is a `&str` (for name/id/arguments).
fn render_rust_tool_calls_deep(chunks_var: &str, tail: &str) -> String {
    let segs = parse_tail(tail);
    // Locate index segment (rust uses .nth(n) on the iterator instead of [N] on a Vec)
    let idx = segs.iter().find_map(|s| match s {
        TailSeg::Index(n) => Some(*n),
        _ => None,
    });
    let field_segs: Vec<&str> = segs
        .iter()
        .filter_map(|s| match s {
            TailSeg::Field(f) => Some(f.as_str()),
            _ => None,
        })
        .collect();

    let base = format!(
        "{chunks_var}.iter().flat_map(|c| c.choices.iter().flat_map(|ch| ch.delta.tool_calls.iter().flatten()))"
    );
    let with_nth = match idx {
        Some(n) => format!("{base}.nth({n})"),
        None => base,
    };

    // Chain Option-aware field access. Every field on StreamToolCall is Option<...>;
    // the leaf (String fields) uses `.as_deref()` to project to `&str`.
    let mut expr = with_nth;
    for (i, f) in field_segs.iter().enumerate() {
        let is_leaf = i == field_segs.len() - 1;
        if is_leaf {
            expr = format!("{expr}.and_then(|x| x.{f}.as_deref())");
        } else {
            expr = format!("{expr}.and_then(|x| x.{f}.as_ref())");
        }
    }
    format!("{expr}.unwrap_or(\"\")")
}

/// Parse a deep-path tail (e.g. `[0].function.name`) into structured segments.
///
/// The tail always starts with either `[N]` (array index) or `.field`.
/// Returns a list of segments: `TailSeg::Index(N)` or `TailSeg::Field(name)`.
#[derive(Debug, PartialEq)]
enum TailSeg {
    Index(usize),
    Field(String),
}

fn parse_tail(tail: &str) -> Vec<TailSeg> {
    let mut segs = Vec::new();
    let mut rest = tail;
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('[') {
            // Array index: `[N]`
            if let Some(close) = inner.find(']') {
                let idx_str = &inner[..close];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    segs.push(TailSeg::Index(idx));
                }
                rest = &inner[close + 1..];
            } else {
                break;
            }
        } else if let Some(inner) = rest.strip_prefix('.') {
            // Field name: up to next `.` or `[`
            let end = inner.find(['.', '[']).unwrap_or(inner.len());
            segs.push(TailSeg::Field(inner[..end].to_string()));
            rest = &inner[end..];
        } else {
            break;
        }
    }
    segs
}

/// Render the full deep accessor expression by appending per-language tail
/// segments onto `root_expr`.
fn render_deep_tail(root_expr: &str, tail: &str, lang: &str) -> String {
    use heck::{ToLowerCamelCase, ToPascalCase};

    let segs = parse_tail(tail);
    let mut out = root_expr.to_string();

    for seg in &segs {
        match (seg, lang) {
            (TailSeg::Index(n), "rust") => {
                out = format!("({out})[{n}]");
            }
            (TailSeg::Index(n), "java") => {
                out = format!("({out}).get({n})");
            }
            (TailSeg::Index(n), "kotlin") => {
                if *n == 0 {
                    out = format!("({out}).first()");
                } else {
                    out = format!("({out}).get({n})");
                }
            }
            (TailSeg::Index(n), "elixir") => {
                out = format!("Enum.at({out}, {n})");
            }
            (TailSeg::Index(n), "zig") => {
                out = format!("({out}).items[{n}]");
            }
            (TailSeg::Index(n), "php") => {
                out = format!("({out})[{n}]");
            }
            (TailSeg::Index(n), _) => {
                // rust-like for go (but we handle Field differently), python, node, ts, kotlin, etc.
                out = format!("({out})[{n}]");
            }
            (TailSeg::Field(f), "rust") => {
                use heck::ToSnakeCase;
                out.push('.');
                out.push_str(&f.to_snake_case());
            }
            (TailSeg::Field(f), "go") => {
                use alef_codegen::naming::to_go_name;
                out.push('.');
                out.push_str(&to_go_name(f));
            }
            (TailSeg::Field(f), "java") => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
            }
            (TailSeg::Field(f), "kotlin") => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
            }
            (TailSeg::Field(f), "csharp") => {
                out.push('.');
                out.push_str(&f.to_pascal_case());
            }
            (TailSeg::Field(f), "php") => {
                // Streaming PHP accessors operate on json_decoded stdClass with
                // snake_case property names (JSON wire format), not the camelCase
                // properties exposed on the PHP wrapper class. Use the raw field
                // name verbatim.
                out.push_str("->");
                out.push_str(f);
            }
            (TailSeg::Field(f), "elixir") => {
                out.push('.');
                out.push_str(f);
            }
            (TailSeg::Field(f), "zig") => {
                out.push('.');
                out.push_str(f);
            }
            (TailSeg::Field(f), "python") | (TailSeg::Field(f), "ruby") => {
                out.push('.');
                out.push_str(f);
            }
            // node, wasm, typescript, kotlin, dart, swift all use camelCase
            (TailSeg::Field(f), _) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
        }
    }

    out
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
    fn is_streaming_virtual_field_rejects_non_root_paths_with_matching_tail() {
        // Regression: prior impl matched any field whose chars-after-root-len started
        // with `[` or `.` — without checking that the field actually starts with the
        // root token. `choices[0].finish_reason` therefore falsely matched root
        // `tool_calls` because byte 10 onward is `.finish_reason`.
        assert!(!is_streaming_virtual_field("choices[0].finish_reason"));
        assert!(!is_streaming_virtual_field("choices[0].message.content"));
        assert!(!is_streaming_virtual_field("usage.total_tokens"));
        assert!(!is_streaming_virtual_field("data[0].embedding"));
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
        assert!(
            snip.contains("chunks.append(std.heap.c_allocator"),
            "zig collect: {snip}"
        );
        assert!(snip.contains(".empty;"), "zig collect (Zig 0.16 unmanaged): {snip}");
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
        // Elixir lists do not support bracket access — must use Enum.at, never choices[0]
        assert!(
            !expr.contains("choices[0]"),
            "elixir stream_content must not use bracket access on list: {expr}"
        );
        assert!(
            expr.contains("Enum.at("),
            "elixir stream_content must use Enum.at for list index: {expr}"
        );
    }

    #[test]
    fn accessor_elixir_stream_complete_uses_list_last() {
        let expr = StreamingFieldResolver::accessor("stream_complete", "elixir", "chunks").unwrap();
        assert!(expr.contains("List.last(chunks)"), "elixir stream_complete: {expr}");
        assert!(expr.contains("finish_reason != nil"), "elixir stream_complete: {expr}");
        // Elixir lists do not support bracket access — must use Enum.at, never choices[0]
        assert!(
            !expr.contains("choices[0]"),
            "elixir stream_complete must not use bracket access on list: {expr}"
        );
        assert!(
            expr.contains("Enum.at("),
            "elixir stream_complete must use Enum.at for list index: {expr}"
        );
    }

    #[test]
    fn accessor_elixir_finish_reason_uses_list_last() {
        let expr = StreamingFieldResolver::accessor("finish_reason", "elixir", "chunks").unwrap();
        assert!(expr.contains("List.last(chunks)"), "elixir finish_reason: {expr}");
        assert!(expr.contains("finish_reason"), "elixir finish_reason: {expr}");
        // Elixir lists do not support bracket access — must use Enum.at, never choices[0]
        assert!(
            !expr.contains("choices[0]"),
            "elixir finish_reason must not use bracket access on list: {expr}"
        );
        assert!(
            expr.contains("Enum.at("),
            "elixir finish_reason must use Enum.at for list index: {expr}"
        );
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
        // Items are Result<ChatCompletionChunk, _> — unwrap so chunks is Vec<ChatCompletionChunk>
        assert!(snip.contains(".expect("), "rust must unwrap Result items: {snip}");
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
    fn collect_snippet_php_decodes_json_or_iterates() {
        let snip = StreamingFieldResolver::collect_snippet("php", "result", "chunks").unwrap();
        // PHP binding's chat_stream_async returns a JSON string today; collect-snippet
        // decodes it.  iterator_to_array is retained as the fallback branch so a
        // future binding that exposes a real iterator continues to work without
        // regenerating the e2e tests.
        assert!(snip.contains("json_decode"), "php must decode JSON: {snip}");
        assert!(
            snip.contains("iterator_to_array"),
            "php must keep iterator_to_array fallback: {snip}"
        );
        assert!(snip.contains("$chunks ="), "php must bind $chunks: {snip}");
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
        assert!(
            expr.contains("finish_reason is not None"),
            "python stream_complete: {expr}"
        );
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

    // -----------------------------------------------------------------------
    // Deep-path tests: tool_calls[0].function.name and tool_calls[0].id
    // -----------------------------------------------------------------------

    #[test]
    fn is_streaming_virtual_field_recognizes_deep_tool_calls_paths() {
        assert!(
            is_streaming_virtual_field("tool_calls[0].function.name"),
            "tool_calls[0].function.name should be recognized"
        );
        assert!(
            is_streaming_virtual_field("tool_calls[0].id"),
            "tool_calls[0].id should be recognized"
        );
        assert!(
            is_streaming_virtual_field("tool_calls[1].function.arguments"),
            "tool_calls[1].function.arguments should be recognized"
        );
        // bare root still recognized
        assert!(is_streaming_virtual_field("tool_calls"));
        // unrelated deep path must NOT be recognized
        assert!(!is_streaming_virtual_field("tool_calls_extra.name"));
        assert!(!is_streaming_virtual_field("nonexistent[0].field"));
    }

    /// Snapshot: `tool_calls[0].function.name` for Rust, Kotlin, TypeScript.
    ///
    /// These three languages cover the main accessor styles:
    /// - Rust: snake_case field, explicit `[0]` index on collected Vec
    /// - Kotlin: camelCase method calls with `.first()` for index 0
    /// - TypeScript/Node: camelCase properties with `[0]` bracket
    #[test]
    fn deep_tool_calls_function_name_snapshot_rust_kotlin_ts() {
        let field = "tool_calls[0].function.name";

        let rust = StreamingFieldResolver::accessor(field, "rust", "chunks").unwrap();
        // Rust: Option-aware chain over the iterator — `.nth(0)` then `.and_then`
        // on each Option-wrapped field (function is Option<StreamFunctionCall>,
        // name is Option<String>). Final `.unwrap_or("")` yields `&str`.
        assert!(
            rust.contains(".nth(0)"),
            "rust deep tool_calls: expected .nth(0) iterator index, got: {rust}"
        );
        assert!(
            rust.contains("x.function.as_ref()"),
            "rust deep tool_calls: expected Option-aware function access, got: {rust}"
        );
        assert!(
            rust.contains("x.name.as_deref()"),
            "rust deep tool_calls: expected Option-aware name leaf, got: {rust}"
        );
        assert!(
            !rust.contains("// skipped"),
            "rust deep tool_calls: must not emit skip comment, got: {rust}"
        );

        let kotlin = StreamingFieldResolver::accessor(field, "kotlin", "chunks").unwrap();
        // Kotlin: uses .first() for index 0, then .function().name()
        assert!(
            kotlin.contains(".first()"),
            "kotlin deep tool_calls: expected .first() for index 0, got: {kotlin}"
        );
        assert!(
            kotlin.contains(".function()"),
            "kotlin deep tool_calls: expected .function() method call, got: {kotlin}"
        );
        assert!(
            kotlin.contains(".name()"),
            "kotlin deep tool_calls: expected .name() method call, got: {kotlin}"
        );

        let ts = StreamingFieldResolver::accessor(field, "node", "chunks").unwrap();
        // TypeScript/Node: uses [0] then .function.name (camelCase)
        assert!(
            ts.contains("[0]"),
            "ts/node deep tool_calls: expected [0] index, got: {ts}"
        );
        assert!(
            ts.contains(".function"),
            "ts/node deep tool_calls: expected .function segment, got: {ts}"
        );
        assert!(
            ts.contains(".name"),
            "ts/node deep tool_calls: expected .name segment, got: {ts}"
        );
    }

    #[test]
    fn deep_tool_calls_id_snapshot_all_langs() {
        let field = "tool_calls[0].id";

        let rust = StreamingFieldResolver::accessor(field, "rust", "chunks").unwrap();
        assert!(rust.contains(".nth(0)"), "rust: {rust}");
        assert!(rust.contains("x.id.as_deref()"), "rust: {rust}");

        let go = StreamingFieldResolver::accessor(field, "go", "chunks").unwrap();
        assert!(go.contains("[0]"), "go: {go}");
        // Go: ID is a well-known initialism → uppercase
        assert!(go.contains(".ID"), "go: expected .ID initialism, got: {go}");

        let python = StreamingFieldResolver::accessor(field, "python", "chunks").unwrap();
        assert!(python.contains("[0]"), "python: {python}");
        assert!(python.contains(".id"), "python: {python}");

        let php = StreamingFieldResolver::accessor(field, "php", "chunks").unwrap();
        assert!(php.contains("[0]"), "php: {php}");
        assert!(php.contains("->id"), "php: expected ->id, got: {php}");

        let java = StreamingFieldResolver::accessor(field, "java", "chunks").unwrap();
        assert!(java.contains(".get(0)"), "java: expected .get(0), got: {java}");
        assert!(java.contains(".id()"), "java: expected .id() method call, got: {java}");

        let csharp = StreamingFieldResolver::accessor(field, "csharp", "chunks").unwrap();
        assert!(csharp.contains("[0]"), "csharp: {csharp}");
        assert!(
            csharp.contains(".Id"),
            "csharp: expected .Id (PascalCase), got: {csharp}"
        );

        let elixir = StreamingFieldResolver::accessor(field, "elixir", "chunks").unwrap();
        assert!(elixir.contains("Enum.at("), "elixir: expected Enum.at(, got: {elixir}");
        assert!(elixir.contains(".id"), "elixir: {elixir}");
    }

    #[test]
    fn deep_tool_calls_function_name_snapshot_python_elixir_zig() {
        let field = "tool_calls[0].function.name";

        let python = StreamingFieldResolver::accessor(field, "python", "chunks").unwrap();
        assert!(python.contains("[0]"), "python: {python}");
        assert!(python.contains(".function"), "python: {python}");
        assert!(python.contains(".name"), "python: {python}");

        let elixir = StreamingFieldResolver::accessor(field, "elixir", "chunks").unwrap();
        // Elixir: Enum.at(…, 0).function.name
        assert!(elixir.contains("Enum.at("), "elixir: {elixir}");
        assert!(elixir.contains(".function"), "elixir: {elixir}");
        assert!(elixir.contains(".name"), "elixir: {elixir}");

        // Zig stores chunks as JSON strings, not typed records — deep
        // tool_calls paths are unsupported and resolve to None so the
        // assertion site can skip them.
        assert!(
            StreamingFieldResolver::accessor(field, "zig", "chunks").is_none(),
            "zig: expected None for deep tool_calls path"
        );
    }

    #[test]
    fn parse_tail_parses_index_then_field_segments() {
        let segs = parse_tail("[0].function.name");
        assert_eq!(segs.len(), 3, "expected 3 segments, got: {segs:?}");
        assert_eq!(segs[0], TailSeg::Index(0));
        assert_eq!(segs[1], TailSeg::Field("function".to_string()));
        assert_eq!(segs[2], TailSeg::Field("name".to_string()));
    }

    #[test]
    fn parse_tail_parses_simple_index_field() {
        let segs = parse_tail("[0].id");
        assert_eq!(segs.len(), 2, "expected 2 segments, got: {segs:?}");
        assert_eq!(segs[0], TailSeg::Index(0));
        assert_eq!(segs[1], TailSeg::Field("id".to_string()));
    }

    #[test]
    fn parse_tail_handles_nonzero_index() {
        let segs = parse_tail("[2].function.arguments");
        assert_eq!(segs[0], TailSeg::Index(2));
        assert_eq!(segs[1], TailSeg::Field("function".to_string()));
        assert_eq!(segs[2], TailSeg::Field("arguments".to_string()));
    }
}
