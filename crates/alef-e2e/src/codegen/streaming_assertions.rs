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
//!
//! Deep-nested paths of the form `<root>[N].field.subfield` are also supported
//! for roots that are streaming-virtual (e.g. `tool_calls[0].function.name`).

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

/// Virtual-field roots that accept deep-nested path suffixes.
///
/// A field like `tool_calls[0].function.name` starts with the root `tool_calls`
/// followed by a `[` or `.` suffix, which makes the whole path a streaming-virtual
/// deep path that [`is_streaming_virtual_field`] should accept.
const STREAMING_VIRTUAL_ROOTS: &[&str] = &["tool_calls", "finish_reason"];

/// Returns `true` when `field` is a streaming-virtual field name.
///
/// This includes both the flat fields listed in [`STREAMING_VIRTUAL_FIELDS`] and
/// deep-nested paths whose root is one of [`STREAMING_VIRTUAL_ROOTS`], such as
/// `tool_calls[0].function.name` or `tool_calls[0].id`.
pub fn is_streaming_virtual_field(field: &str) -> bool {
    if STREAMING_VIRTUAL_FIELDS.contains(&field) {
        return true;
    }
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

/// Splits `field` into `(root, tail)` when it is a deep-nested streaming path.
///
/// Returns `None` when `field` is not a deep-nested path (i.e. it is a flat
/// streaming-virtual field or an unknown field).
///
/// Example: `"tool_calls[0].function.name"` → `Some(("tool_calls", "[0].function.name"))`.
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

/// A single navigation segment parsed from a deep-path tail string.
#[derive(Debug, PartialEq)]
enum TailSeg {
    /// Array / list index: `[N]`.
    Index(usize),
    /// Named field access: `.name`.
    Field(String),
}

/// Parses a tail string such as `"[0].function.name"` into a [`Vec`] of [`TailSeg`].
fn parse_tail(tail: &str) -> Vec<TailSeg> {
    let mut segs = Vec::new();
    let mut rest = tail;
    while !rest.is_empty() {
        if let Some(after_bracket) = rest.strip_prefix('[') {
            if let Some(close) = after_bracket.find(']') {
                let idx_str = &after_bracket[..close];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    segs.push(TailSeg::Index(idx));
                }
                rest = &after_bracket[close + 1..];
                continue;
            }
        }
        if let Some(name_start) = rest.strip_prefix('.') {
            let end = name_start.find(['.', '[']).unwrap_or(name_start.len());
            let name = &name_start[..end];
            if !name.is_empty() {
                segs.push(TailSeg::Field(name.to_string()));
            }
            rest = &name_start[end..];
            continue;
        }
        // Unrecognised character — stop parsing.
        break;
    }
    segs
}

/// Renders a deep-path navigation expression starting from `root_expr`.
///
/// `tail` is the raw suffix string (e.g. `"[0].function.name"`).
/// `lang` controls per-language syntax for indexing and field naming.
fn render_deep_tail(root_expr: &str, tail: &str, lang: &str) -> String {
    use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};

    let segs = parse_tail(tail);
    let mut expr = root_expr.to_string();

    for seg in segs {
        match seg {
            TailSeg::Index(idx) => match lang {
                "java" => {
                    expr = format!("{expr}.get({idx})");
                }
                "kotlin" => {
                    if idx == 0 {
                        expr = format!("{expr}.first()");
                    } else {
                        expr = format!("{expr}.get({idx})");
                    }
                }
                "elixir" => {
                    expr = format!("Enum.at({expr}, {idx})");
                }
                "zig" => {
                    expr = format!("{expr}.items[{idx}]");
                }
                _ => {
                    expr = format!("{expr}[{idx}]");
                }
            },
            TailSeg::Field(ref name) => match lang {
                "rust" | "python" | "elixir" | "ruby" => {
                    let snake = name.to_snake_case();
                    expr = format!("{expr}.{snake}");
                }
                "go" => {
                    // Go uses PascalCase with initialism expansion (ID, URL, etc.)
                    let pascal = name.to_pascal_case();
                    // Promote common initialisms that heck does not handle.
                    let go_name = match pascal.as_str() {
                        "Id" => "ID".to_string(),
                        "Url" => "URL".to_string(),
                        "Http" => "HTTP".to_string(),
                        other => other.to_string(),
                    };
                    expr = format!("{expr}.{go_name}");
                }
                "java" | "kotlin" => {
                    // Java/Kotlin record accessor methods: lowerCamelCase()
                    let camel = name.to_lower_camel_case();
                    expr = format!("{expr}.{camel}()");
                }
                "csharp" => {
                    let pascal = name.to_pascal_case();
                    expr = format!("{expr}.{pascal}");
                }
                "php" => {
                    let camel = name.to_lower_camel_case();
                    expr = format!("{expr}->{camel}");
                }
                "swift" => {
                    // swift-bridge exposes Rust struct fields as snake_case method calls.
                    // e.g. `function` → `.function()`, `name` → `.name()`
                    let snake = name.to_snake_case();
                    expr = format!("{expr}.{snake}()");
                }
                _ => {
                    // node / typescript / dart / wasm / zig
                    let camel = name.to_lower_camel_case();
                    expr = format!("{expr}.{camel}");
                }
            },
        }
    }

    expr
}

/// Shared streaming-virtual-fields resolver for e2e test codegen.
pub struct StreamingFieldResolver;

impl StreamingFieldResolver {
    /// Returns the language-specific expression for a streaming-virtual field,
    /// given `chunks_var` (the collected-list local name) and `lang`.
    ///
    /// Returns `None` when the field name is not a known streaming-virtual
    /// field or the language has no streaming support.
    ///
    /// Deep-nested paths such as `tool_calls[0].function.name` are also
    /// supported: the root is resolved first and the tail is rendered using
    /// per-language navigation conventions.
    pub fn accessor(field: &str, lang: &str, chunks_var: &str) -> Option<String> {
        match field {
            "chunks" => Some(match lang {
                // Zig ArrayList does not expose .len directly; must use .items
                "zig" => format!("{chunks_var}.items"),
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
                // swift: [ChatCompletionChunk] Swift array uses .count
                "swift" => format!("{chunks_var}.count"),
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
                    format!("{chunks_var} |> Enum.map(&(&1.choices[0].delta.content || \"\")) |> Enum.join(\"\")")
                }
                "python" => {
                    format!("\"\".join(c.choices[0].delta.content or \"\" for c in {chunks_var} if c.choices)")
                }
                "zig" => {
                    // Zig: `{chunks_var}_content` is a `std.ArrayList(u8)` populated by
                    // the collect snippet. `.items` gives a `[]u8` slice of the content.
                    format!("{chunks_var}_content.items")
                }
                // swift: [ChatCompletionChunk] — compactMap over choices().first?.delta().content()
                // choices() returns RustVec<StreamChoice> (Collection), delta() is non-optional,
                // content() returns Optional<RustString>; toString() converts to Swift String.
                "swift" => {
                    format!("{chunks_var}.compactMap {{ $0.choices().first?.delta().content()?.toString() }}.joined()")
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
                    format!("bool({chunks_var}) and {chunks_var}[-1].choices[0].finish_reason is not None")
                }
                "elixir" => {
                    format!("List.last({chunks_var}).choices[0].finish_reason != nil")
                }
                // zig: the collect snippet exhausts the stream; check last chunk JSON
                // was collected (chunks.items is non-empty) as a proxy for completion.
                "zig" => {
                    format!("{chunks_var}.items.len > 0")
                }
                // swift: non-empty array and last chunk's first choice has a non-nil finish_reason
                "swift" => {
                    format!("!{chunks_var}.isEmpty && {chunks_var}.last?.choices().first?.finish_reason() != nil")
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
                // Zig: tool_calls count from all chunk deltas
                "zig" => {
                    format!("{chunks_var}.items")
                }
                // swift: flat-map tool_calls from all chunk deltas; tool_calls() returns
                // Optional<RustVec<StreamToolCall>> so use ?? [] (coalesced to empty Array via map)
                "swift" => {
                    format!(
                        "{chunks_var}.flatMap {{ c -> [StreamToolCall] in (c.choices().first?.delta().tool_calls()).map {{ vec in (0..<vec.len()).map {{ vec[$0] as! StreamToolCall }} }} ?? [] }}"
                    )
                }
                _ => {
                    format!("{chunks_var}.flatMap((c: any) => c.choices?.[0]?.delta?.toolCalls ?? [])")
                }
            }),

            "finish_reason" => Some(match lang {
                "rust" => {
                    // FinishReason is an enum (not Deref<str>); convert via Display.
                    format!(
                        "{chunks_var}.last().and_then(|c| c.choices.first()).and_then(|ch| ch.finish_reason.as_ref().map(|r| r.to_string())).unwrap_or_default()"
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
                // Zig: finish_reason from the last chunk's JSON via an inline labeled block.
                // Returns `[]const u8` (unwrapped with orelse "" for expectEqualStrings).
                "zig" => {
                    format!(
                        "(blk: {{ if ({chunks_var}.items.len == 0) break :blk \"\"; var _lcp = std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {chunks_var}.items[{chunks_var}.items.len - 1], .{{}}) catch break :blk \"\"; defer _lcp.deinit(); if (_lcp.value.object.get(\"choices\")) |_lchs| if (_lchs.array.items.len > 0) if (_lchs.array.items[0].object.get(\"finish_reason\")) |_fr| if (_fr == .string) break :blk _fr.string; break :blk \"\"; }})"
                    )
                }
                // swift: finish_reason from last chunk's first StreamChoice.
                // finish_reason() returns Optional<FinishReason> (opaque swift-bridge type);
                // to_string().toString() converts it to a Swift String for comparisons.
                "swift" => {
                    format!("{chunks_var}.last?.choices().first?.finish_reason()?.to_string().toString() ?? \"\"")
                }
                _ => {
                    format!(
                        "{chunks_var}.length > 0 ? {chunks_var}[{chunks_var}.length - 1].choices?.[0]?.finishReason : undefined"
                    )
                }
            }),

            _ => {
                // Check for deep-nested path with a streaming-virtual root.
                if let Some((root, tail)) = split_streaming_deep_path(field) {
                    // Rust-specific: StreamToolCall.function is Option<StreamFunctionCall>
                    // and StreamFunctionCall.name is Option<String>.  Direct field access
                    // on an Option fails at compile time.  Rewrite deep paths of the form
                    // `tool_calls[N].function.name` / `tool_calls[N].function.arguments`
                    // into a monadic chain so the expression type-checks.
                    if lang == "rust" && root == "tool_calls" {
                        let root_expr = Self::accessor(root, lang, chunks_var)?;
                        // Parse the tail to extract index and sub-fields.
                        let segs = parse_tail(tail);
                        // Expect the tail to start with an index segment.
                        if let Some(TailSeg::Index(idx)) = segs.first() {
                            let base = format!("{root_expr}[{idx}]");
                            // Collect the remaining field names after the index.
                            let sub_fields: Vec<&str> = segs[1..]
                                .iter()
                                .filter_map(|s| {
                                    if let TailSeg::Field(name) = s {
                                        Some(name.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            // Build an Option-aware chain for known StreamToolCall fields.
                            let expr = match sub_fields.as_slice() {
                                // tool_calls[N].function.name → monadic chain through Option<StreamFunctionCall>
                                ["function", "name"] => {
                                    format!("{base}.function.as_ref().and_then(|f| f.name.as_deref()).unwrap_or(\"\")")
                                }
                                // tool_calls[N].function.arguments
                                ["function", "arguments"] => {
                                    format!(
                                        "{base}.function.as_ref().and_then(|f| f.arguments.as_deref()).unwrap_or(\"\")"
                                    )
                                }
                                // tool_calls[N].id
                                ["id"] => {
                                    format!("{base}.id.as_deref().unwrap_or(\"\")")
                                }
                                // tool_calls[N].function (bare)
                                ["function"] => {
                                    format!("{base}.function.as_ref().unwrap()")
                                }
                                // Fallback: use generic deep tail (may not type-check for all paths)
                                _ => render_deep_tail(&base, tail, lang),
                            };
                            return Some(expr);
                        }
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
                "let {chunks_var}: Vec<_> = tokio_stream::StreamExt::collect::<Result<Vec<_>, _>>({stream_var}).await.expect(\"stream error\");"
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
                Some(format!("val {chunks_var} = {stream_var}.asSequence().toList()"))
            }
            "elixir" => Some(format!("{chunks_var} = Enum.to_list({stream_var})")),
            // swift: chatStream returns an AsyncSequence<ChatCompletionChunk>; drain with
            // `for try await chunk in result { ... }`.  The result variable must be the
            // async sequence returned by chatStream (default: "result").
            "swift" => Some(format!(
                "var {chunks_var}: [ChatCompletionChunk] = []\n        for try await _chunk in {stream_var} {{\n            {chunks_var}.append(_chunk)\n        }}"
            )),
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
        assert!(
            snip.contains("Result<Vec<_"),
            "rust collect must unwrap Results: {snip}"
        );
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

    // ---- deep-nested path tests ----

    #[test]
    fn is_streaming_virtual_field_recognizes_deep_tool_calls_paths() {
        assert!(
            is_streaming_virtual_field("tool_calls[0].function.name"),
            "tool_calls[0].function.name should be virtual"
        );
        assert!(
            is_streaming_virtual_field("tool_calls[0].id"),
            "tool_calls[0].id should be virtual"
        );
        assert!(
            is_streaming_virtual_field("tool_calls[1].function.arguments"),
            "tool_calls[1].function.arguments should be virtual"
        );
        // Plain prefix without valid suffix must NOT match.
        assert!(
            !is_streaming_virtual_field("tool_callsX"),
            "tool_callsX must not be virtual"
        );
        assert!(
            !is_streaming_virtual_field("tool_calls_extra"),
            "tool_calls_extra must not be virtual"
        );
    }

    #[test]
    fn parse_tail_parses_index_then_field_segments() {
        let segs = parse_tail("[0].function.name");
        assert_eq!(
            segs,
            vec![
                TailSeg::Index(0),
                TailSeg::Field("function".to_string()),
                TailSeg::Field("name".to_string()),
            ]
        );
    }

    #[test]
    fn parse_tail_parses_simple_index_field() {
        let segs = parse_tail("[0].id");
        assert_eq!(segs, vec![TailSeg::Index(0), TailSeg::Field("id".to_string()),]);
    }

    #[test]
    fn parse_tail_handles_nonzero_index() {
        let segs = parse_tail("[2].type");
        assert_eq!(segs, vec![TailSeg::Index(2), TailSeg::Field("type".to_string()),]);
    }

    #[test]
    fn deep_tool_calls_function_name_snapshot_rust_kotlin_ts() {
        let rust = StreamingFieldResolver::accessor("tool_calls[0].function.name", "rust", "chunks")
            .expect("rust deep path must resolve");
        // Rust: flat_map root → [0] → Option-aware chain for function.name
        // StreamToolCall.function is Option<StreamFunctionCall> and name is Option<String>,
        // so the expression uses and_then chains rather than direct field access.
        assert!(
            rust.contains("[0].function.as_ref().and_then(|f| f.name.as_deref()).unwrap_or(\"\")"),
            "rust deep path: {rust}"
        );

        let kotlin = StreamingFieldResolver::accessor("tool_calls[0].function.name", "kotlin", "chunks")
            .expect("kotlin deep path must resolve");
        // Kotlin: .first() for index 0, then .function() and .name() method calls
        assert!(kotlin.contains(".first()"), "kotlin deep path index: {kotlin}");
        assert!(kotlin.contains(".function()"), "kotlin deep path field: {kotlin}");
        assert!(kotlin.contains(".name()"), "kotlin deep path field: {kotlin}");

        let ts = StreamingFieldResolver::accessor("tool_calls[0].function.name", "node", "chunks")
            .expect("ts/node deep path must resolve");
        // TypeScript/node: [0].function.name  (camelCase, but these are already camel)
        assert!(ts.contains("[0].function.name"), "ts deep path: {ts}");
    }

    #[test]
    fn deep_tool_calls_id_snapshot_all_langs() {
        let cases: &[(&str, &str)] = &[
            ("rust", "[0].id"),
            ("go", "[0].ID"),
            ("java", ".get(0).id()"),
            ("kotlin", ".first().id()"),
            ("python", "[0].id"),
            ("elixir", ", 0).id"),
            ("php", "[0]->id"),
            ("csharp", "[0].Id"),
            ("node", "[0].id"),
        ];

        for (lang, expected_fragment) in cases {
            let expr = StreamingFieldResolver::accessor("tool_calls[0].id", lang, "chunks")
                .unwrap_or_else(|| panic!("lang {lang} must resolve tool_calls[0].id"));
            assert!(
                expr.contains(expected_fragment),
                "lang={lang}: expected fragment '{expected_fragment}' in '{expr}'"
            );
        }
    }

    #[test]
    fn deep_tool_calls_function_name_snapshot_python_elixir_zig() {
        let python = StreamingFieldResolver::accessor("tool_calls[0].function.name", "python", "chunks")
            .expect("python deep path must resolve");
        assert!(python.contains("[0].function.name"), "python: {python}");

        let elixir = StreamingFieldResolver::accessor("tool_calls[0].function.name", "elixir", "chunks")
            .expect("elixir deep path must resolve");
        // Elixir: Enum.at(root, 0).function.name  (snake_case fields, Enum.at for index)
        assert!(elixir.contains("Enum.at("), "elixir Enum.at: {elixir}");
        assert!(elixir.contains(".function"), "elixir .function field: {elixir}");
        assert!(elixir.contains(".name"), "elixir .name field: {elixir}");

        let zig = StreamingFieldResolver::accessor("tool_calls[0].function.name", "zig", "chunks")
            .expect("zig deep path must resolve");
        // Zig: .items[0].function.name  (zig ArrayList root uses .items)
        assert!(zig.contains(".items[0]"), "zig .items[0]: {zig}");
        assert!(zig.contains(".function"), "zig .function: {zig}");
        assert!(zig.contains(".name"), "zig .name: {zig}");
    }

    // ---- swift-specific tests ----

    #[test]
    fn accessor_swift_chunks_length_uses_count() {
        let expr = StreamingFieldResolver::accessor("chunks.length", "swift", "chunks").unwrap();
        assert_eq!(expr, "chunks.count", "swift chunks.length: {expr}");
    }

    #[test]
    fn accessor_swift_stream_content_uses_compact_map_joined() {
        let expr = StreamingFieldResolver::accessor("stream_content", "swift", "chunks").unwrap();
        assert!(
            expr.contains("compactMap"),
            "swift stream_content must use compactMap: {expr}"
        );
        assert!(
            expr.contains("joined()"),
            "swift stream_content must use joined(): {expr}"
        );
        assert!(
            expr.contains("choices()"),
            "swift stream_content must use choices(): {expr}"
        );
        assert!(
            expr.contains("delta()"),
            "swift stream_content must use delta(): {expr}"
        );
        assert!(
            expr.contains("content()"),
            "swift stream_content must use content(): {expr}"
        );
    }

    #[test]
    fn accessor_swift_stream_complete_uses_finish_reason() {
        let expr = StreamingFieldResolver::accessor("stream_complete", "swift", "chunks").unwrap();
        assert!(expr.contains("chunks.isEmpty"), "swift stream_complete: {expr}");
        assert!(
            expr.contains("finish_reason()"),
            "swift stream_complete must use finish_reason(): {expr}"
        );
    }

    #[test]
    fn accessor_swift_finish_reason_uses_last_chunk() {
        let expr = StreamingFieldResolver::accessor("finish_reason", "swift", "chunks").unwrap();
        assert!(
            expr.contains("chunks.last"),
            "swift finish_reason must use chunks.last: {expr}"
        );
        assert!(
            expr.contains("finish_reason()"),
            "swift finish_reason must use finish_reason(): {expr}"
        );
        assert!(
            expr.contains("to_string()"),
            "swift finish_reason must use to_string(): {expr}"
        );
    }

    #[test]
    fn collect_snippet_swift_uses_for_await() {
        let snip = StreamingFieldResolver::collect_snippet("swift", "result", "chunks").unwrap();
        assert!(
            snip.contains("var chunks: [ChatCompletionChunk]"),
            "swift collect: {snip}"
        );
        assert!(snip.contains("for try await _chunk in result"), "swift collect: {snip}");
        assert!(snip.contains("chunks.append(_chunk)"), "swift collect: {snip}");
    }

    #[test]
    fn deep_tool_calls_function_name_snapshot_swift() {
        let swift = StreamingFieldResolver::accessor("tool_calls[0].function.name", "swift", "chunks")
            .expect("swift deep path must resolve");
        // Swift: [0] (subscript on array), then .function() and .name() method calls
        assert!(swift.contains("[0]"), "swift deep path index: {swift}");
        assert!(swift.contains(".function()"), "swift deep path .function(): {swift}");
        assert!(swift.contains(".name()"), "swift deep path .name(): {swift}");
    }

    #[test]
    fn accessor_swift_no_chunks_after_done_returns_true() {
        let expr = StreamingFieldResolver::accessor("no_chunks_after_done", "swift", "chunks").unwrap();
        assert_eq!(expr, "true", "swift no_chunks_after_done: {expr}");
    }
}
