//! C# streaming e2e test method rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::{build_args_and_setup, json_to_csharp};
use crate::e2e::codegen::inert_example::{self, InertCause};
use crate::e2e::escape::escape_csharp;

pub(super) fn resolve_csharp_streaming_item_type(
    call_config: &crate::e2e::config::CallConfig,
    adapters: &[crate::core::config::extras::AdapterConfig],
    function_name: &str,
) -> Option<String> {
    let function_name_snake = function_name.to_snake_case();
    crate::e2e::codegen::recipe::streaming_item_type(
        call_config,
        adapters,
        &[function_name, function_name_snake.as_str()],
    )
    .map(str::to_string)
}

/// Render a streaming-adapter test method. The C# binding emits
/// `IAsyncEnumerable<T>` (not `Task<T>`), so the test body uses `await foreach`
/// to drive the stream and aggregates
/// per-chunk data into local vars (`chunks`, `streamContent`, `streamComplete`,
/// optional `lastFinishReason`/`toolCallsJson`/`toolCalls0FunctionName`/`totalTokens`).
/// Assertions then run against those locals — never against pseudo-fields on a
/// response object.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_streaming_test_method(
    out: &mut String,
    fixture: &Fixture,
    class_name: &str,
    call_config: &crate::e2e::config::CallConfig,
    cs_overrides: Option<&crate::e2e::config::CallOverride>,
    e2e_config: &E2eConfig,
    enum_fields: &HashMap<String, String>,
    _assert_enum_fields: &HashMap<String, String>,
    nested_types: &HashMap<String, String>,
    exception_class: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    item_type: Option<&str>,
) {
    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let Some(item_type) = item_type else {
        let _ = writeln!(out, "    [Fact]");
        let _ = writeln!(out, "    public void Test_{method_name}()");
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        // skipped: streaming fixture requires adapter item_type for C# e2e codegen"
        );
        let _ = writeln!(out, "    }}");
        return;
    };

    // Streaming methods return IAsyncEnumerable<T> and carry the conventional
    // `Async` suffix to match the binding's generated DefaultClient surface
    // (which appends Async to every async-shaped method, streaming included).
    let effective_function_name = {
        let mut name = cs_overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call_config.function.to_upper_camel_case());
        if !name.ends_with("Async") {
            name.push_str("Async");
        }
        name
    };
    let function_name = effective_function_name.as_str();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("csharp", fixture, call_config, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params("csharp");
    let args = recipe.args;

    let top_level_options_type = e2e_config
        .call
        .overrides
        .get("csharp")
        .and_then(|o| o.options_type.as_deref());
    let effective_options_type = recipe.options_type.or(top_level_options_type);
    let top_level_options_via = e2e_config
        .call
        .overrides
        .get("csharp")
        .and_then(|o| o.options_via.as_deref());
    let effective_options_via = cs_overrides
        .and_then(|o| o.options_via.as_deref())
        .or(top_level_options_via);

    let adapter_lookup_name = call_config.core_lookup_name("csharp");
    let adapter_request_type_cs: Option<String> = adapter_lookup_name
        .as_deref()
        .and_then(|name| adapters.iter().find(|a| a.name == name))
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
    let mut _chat_stream_class_decls: Vec<String> = Vec::new();
    let mut _chat_stream_teardown_lines: Vec<String> = Vec::new();
    // `mock_url_list` args are wrapped in `adapter_request_type_cs`'s declared request type
    // directly inside `build_args_and_setup` (see `setup.rs`'s `mock_url_list` arm), the same
    // way `mock_url` args always were -- this used to be a separate post-processing step here
    // alone, which the docs snippet renderer never got to consult. ~keep
    let (setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        class_name,
        effective_options_type,
        effective_options_via,
        enum_fields,
        nested_types,
        fixture,
        adapter_request_type_cs.as_deref(),
        config,
        type_defs,
        enums,
        target_params,
        &mut _chat_stream_class_decls,
        &mut _chat_stream_teardown_lines,
    );

    let client_factory = cs_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get("csharp")
            .and_then(|o| o.client_factory.as_deref())
    });
    let mut client_factory_setup = String::new();
    if let Some(factory) = client_factory {
        let factory_name = factory.to_upper_camel_case();
        let fixture_id = &fixture.id;
        let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
        let api_key_var_opt = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
        let is_live_smoke = !has_mock && api_key_var_opt.is_some();
        if let Some(api_key_var) = api_key_var_opt.filter(|_| has_mock) {
            client_factory_setup.push_str(&format!(
                "        var apiKey = System.Environment.GetEnvironmentVariable(\"{api_key_var}\");\n"
            ));
            client_factory_setup.push_str(&format!(
                "        var baseUrl = string.IsNullOrEmpty(apiKey)\n            ? (System.Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? string.Empty) + \"/fixtures/{fixture_id}\"\n            : null;\n"
            ));
            client_factory_setup.push_str(&format!(
                "        Console.WriteLine($\"{fixture_id}: \" + (baseUrl == null ? \"using real API ({api_key_var} is set)\" : \"using mock server ({api_key_var} not set)\"));\n"
            ));
            client_factory_setup.push_str(&format!(
                "        var client = {class_name}.{factory_name}(string.IsNullOrEmpty(apiKey) ? \"test-key\" : apiKey, baseUrl, null, null, null);\n"
            ));
        } else if let Some(api_key_var) = api_key_var_opt.filter(|_| is_live_smoke) {
            client_factory_setup.push_str(&format!(
                "        var apiKey = System.Environment.GetEnvironmentVariable(\"{api_key_var}\");\n"
            ));
            client_factory_setup.push_str("        if (string.IsNullOrEmpty(apiKey)) { return; }\n");
            client_factory_setup.push_str(&format!(
                "        var client = {class_name}.{factory_name}(apiKey, null, null, null, null);\n"
            ));
        } else if fixture.has_host_root_route() {
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            client_factory_setup.push_str(&format!(
                "        var _perFixtureUrl = System.Environment.GetEnvironmentVariable(\"{env_key}\");\n"
            ));
            client_factory_setup.push_str(&format!("        var baseUrl = !string.IsNullOrEmpty(_perFixtureUrl) ? _perFixtureUrl : (System.Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? string.Empty) + \"/fixtures/{fixture_id}\";\n"));
            client_factory_setup.push_str(&format!(
                "        var client = {class_name}.{factory_name}(\"test-key\", baseUrl, null, null, null);\n"
            ));
        } else {
            client_factory_setup.push_str(&format!(
                "        var baseUrl = (System.Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") ?? string.Empty) + \"/fixtures/{fixture_id}\";\n"
            ));
            client_factory_setup.push_str(&format!(
                "        var client = {class_name}.{factory_name}(\"test-key\", baseUrl, null, null, null);\n"
            ));
        }
    }

    let call_target = if client_factory.is_some() { "client" } else { class_name };
    let call_expr = format!("{call_target}.{function_name}({args_str})");

    // Detect whether to use streaming-specific aggregators (chat-completion style)
    // or skip streaming accumulation altogether when the item type has no Choices field.
    // For non-chat-completion streams (e.g., CrawlEvent), use call_config's result_fields.
    let is_chat_stream = fixture.assertions.iter().any(|a| {
        if let Some(f) = a.field.as_deref() {
            matches!(
                f,
                "stream_content"
                    | "finish_reason"
                    | "tool_calls"
                    | "tool_calls[0].function.name"
                    | "usage.total_tokens"
            )
        } else {
            false
        }
    });

    let mut body = String::new();
    // ~keep The `[Fact]` attribute is decided at the END of this function rather than written
    // here, because xUnit's only skip is `[Fact(Skip = ..)]` — an attribute — and whether this
    // method has anything left to assert is not known until every assertion has rendered. Both
    // emit points below prepend this line.
    let mut fact_attribute = "    [Fact]".to_string();
    let _ = writeln!(body, "    public async Task Test_{method_name}()");
    let _ = writeln!(body, "    {{");
    let _ = writeln!(body, "        // {description}");
    if !client_factory_setup.is_empty() {
        body.push_str(&client_factory_setup);
    }
    for line in &setup_lines {
        let _ = writeln!(body, "        {line}");
    }

    if expects_error {
        // Wrap the foreach in a lambda so the IAsyncEnumerable is actually
        // consumed (otherwise the producer never runs and no exception is raised).
        let _ = writeln!(
            body,
            "        await Assert.ThrowsAnyAsync<{exception_class}>(async () => {{"
        );
        let _ = writeln!(body, "            await foreach (var _chunk in {call_expr}) {{ }}");
        body.push_str("        });\n");
        body.push_str("    }\n");
        for line in std::iter::once(fact_attribute.as_str()).chain(body.lines()) {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
        return;
    }

    let _ = writeln!(body, "        var chunks = new List<{item_type}>();");
    // Optional chat-stream aggregator vars — emitted only when assertions reference them
    // so we don't pollute non-chat streaming bodies (CrawlEvent etc.) with chat-only
    // pseudo-fields that have no analog on the streamed item type.
    let asserts_finish_reason = is_chat_stream
        && fixture
            .assertions
            .iter()
            .any(|a| a.field.as_deref() == Some("finish_reason"));
    let asserts_tool_calls = is_chat_stream
        && fixture
            .assertions
            .iter()
            .any(|a| a.field.as_deref() == Some("tool_calls"));
    let asserts_tool_call_name = is_chat_stream
        && fixture
            .assertions
            .iter()
            .any(|a| a.field.as_deref() == Some("tool_calls[0].function.name"));
    let asserts_total_tokens = is_chat_stream
        && fixture
            .assertions
            .iter()
            .any(|a| a.field.as_deref() == Some("usage.total_tokens"));
    if is_chat_stream {
        body.push_str("        var streamContent = new System.Text.StringBuilder();\n");
    }
    if asserts_finish_reason {
        body.push_str("        string? lastFinishReason = null;\n");
    }
    if asserts_tool_calls {
        body.push_str("        string? toolCallsJson = null;\n");
    }
    if asserts_tool_call_name {
        body.push_str("        string? toolCalls0FunctionName = null;\n");
    }
    if asserts_total_tokens {
        body.push_str("        long? totalTokens = null;\n");
    }
    // `no_chunks_after_done` is only observable by asking the enumerator for one more element
    // AFTER it has already reported completion -- the one thing `await foreach` hides. Fixtures
    // asserting it therefore get the manual-enumerator shape so the flag carries a real probe
    // result; every other fixture keeps the `await foreach` body byte-for-byte. This field used to
    // be aliased onto `streamComplete`, so two distinct assertions rendered one same check. ~keep
    let asserts_no_chunks_after_done = fixture
        .assertions
        .iter()
        .any(|a| a.field.as_deref() == Some("no_chunks_after_done"));
    if asserts_no_chunks_after_done {
        let _ = writeln!(
            body,
            "        await using var streamEnumerator = {call_expr}.GetAsyncEnumerator();"
        );
        body.push_str("        while (await streamEnumerator.MoveNextAsync())\n");
        body.push_str("        {\n");
        body.push_str("            var chunk = streamEnumerator.Current;\n");
    } else {
        let _ = writeln!(body, "        await foreach (var chunk in {call_expr})");
        body.push_str("        {\n");
    }
    body.push_str("            chunks.Add(chunk);\n");

    if is_chat_stream {
        // Chat-completion style streaming: look for Choices[0].Delta.Content
        body.push_str(
            "            var choice = chunk.Choices != null && chunk.Choices.Count > 0 ? chunk.Choices[0] : null;\n",
        );
        body.push_str("            if (choice != null)\n");
        body.push_str("            {\n");
        body.push_str("                var delta = choice.Delta;\n");
        body.push_str("                if (delta != null && !string.IsNullOrEmpty(delta.Content))\n");
        body.push_str("                {\n");
        body.push_str("                    streamContent.Append(delta.Content);\n");
        body.push_str("                }\n");
        if asserts_finish_reason {
            // FinishReason is a JSON-converter-driven enum on the chat-completion DTOs;
            // serialize it through the converter so we get the snake_case API value
            // (e.g. "tool_calls") that assertions compare against, not the .NET name.
            body.push_str("                if (choice.FinishReason.HasValue)\n");
            body.push_str("                {\n");
            body.push_str(
                "                    lastFinishReason = System.Text.Json.JsonSerializer.Serialize(choice.FinishReason.Value).Trim('\"');\n",
            );
            body.push_str("                }\n");
        }
        if asserts_tool_calls || asserts_tool_call_name {
            body.push_str(
                "                if (delta != null && delta.ToolCalls != null && delta.ToolCalls.Count > 0)\n",
            );
            body.push_str("                {\n");
            if asserts_tool_calls {
                body.push_str(
                    "                    toolCallsJson = System.Text.Json.JsonSerializer.Serialize(delta.ToolCalls);\n",
                );
            }
            if asserts_tool_call_name {
                body.push_str("                    var firstFn = delta.ToolCalls[0].Function;\n");
                body.push_str("                    if (firstFn != null && !string.IsNullOrEmpty(firstFn.Name))\n");
                body.push_str("                    {\n");
                body.push_str("                        toolCalls0FunctionName = firstFn.Name;\n");
                body.push_str("                    }\n");
            }
            body.push_str("                }\n");
        }
        body.push_str("            }\n");
        if asserts_total_tokens {
            // Usage.TotalTokens is ulong on the chat-completion DTOs; widen to long?
            // so the assertion-mapping in `map_chat_stream_field` (Kind::IntTokens) can
            // compare against `long`-valued assertion JSON without losing the null state.
            body.push_str("            if (chunk.Usage != null)\n");
            body.push_str("            {\n");
            body.push_str("                totalTokens = (long)chunk.Usage.TotalTokens;\n");
            body.push_str("            }\n");
        }
    }
    body.push_str("        }\n");
    if asserts_no_chunks_after_done {
        body.push_str("        var noChunksAfterDone = !(await streamEnumerator.MoveNextAsync());\n");
    }
    if is_chat_stream {
        // `stream_complete` means here what `StreamingFieldResolver`'s accessor means for every
        // other language (see `streaming_assertions/model.rs`'s field table): the LAST collected
        // chunk carries a terminal finish_reason. Deriving it from the collected chunks is what
        // lets `Assert.True(streamComplete)` fail at all -- the unconditional `streamComplete =
        // true` that used to sit immediately above the assertion could not. ~keep
        body.push_str("        var streamComplete = chunks.Count > 0\n");
        body.push_str("            && chunks[chunks.Count - 1].Choices != null\n");
        body.push_str("            && chunks[chunks.Count - 1].Choices.Count > 0\n");
        body.push_str("            && chunks[chunks.Count - 1].Choices[0].FinishReason.HasValue;\n");
    }

    // Emit assertions on local aggregator vars or result_fields.
    // ~keep The offset is recorded so the marker scan and the inert verdict below read the
    // assertion region alone. `body` already holds the method signature, the setup lines and the
    // whole `await foreach` drive, so a verdict taken over all of it would answer "this example
    // asserts something" for every fixture — the drive is executable text.
    let assertions_start = body.len();
    for assertion in &fixture.assertions {
        if is_chat_stream {
            emit_chat_stream_assertion(&mut body, assertion);
        } else {
            // Must be the EFFECTIVE set, not the per-call one. A per-call list REPLACES the global
            // rather than merging, so a crate configuring only `[crates.e2e].result_fields` leaves
            // `call_config.result_fields` empty -- and an empty set gates every field out, silently
            // degrading every assertion here to a skip comment while the suite still reports green.
            // This was the only assertion-gating site in e2e codegen reading the raw per-call set. ~keep
            emit_non_chat_stream_assertion(&mut body, assertion, e2e_config.effective_result_fields(call_config));
        }
    }
    // `streamComplete` is asserted only where the fixture itself declares that field --
    // `emit_chat_stream_assertion` above already renders `Assert.<op>(streamComplete)` (or a
    // skip marker) for every declared assertion. A fixture that never mentions `stream_complete`
    // (e.g. `empty_stream`, which declares `count_min chunks >= 0` -- an explicit statement that
    // zero chunks is acceptable) gets no expectation synthesised here; inventing
    // `Assert.True(streamComplete)` would contradict such a fixture rather than check it. ~keep

    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &body[assertions_start..],
        "csharp",
        &fixture.id,
        &fixture.assertions,
    );
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&body[assertions_start..], "csharp", &fixture.id);

    // ~keep A streaming example has no honest fallback subject of its own: `chunks` is bound to a
    // fresh `new List<T>()` immediately above, so any check on it could never fail. The one
    // exception is a fixture that declared `not_error` — "the stream does not throw" IS a real
    // check and the drive already carries it, so refusing there would delete working coverage.
    let declares_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    let verdict = if declares_not_error {
        None
    } else {
        inert_example::inert_verdict(&body[assertions_start..], "csharp", &fixture.id, &fixture.assertions)
    };
    if let Some(refusal) = verdict {
        inert_example::record_refusal(&refusal);
        let markers = body[assertions_start..].to_string();
        let reason = escape_csharp(&refusal.reason());
        let statement = match refusal.cause {
            InertCause::UnresolvedFieldPath => {
                format!(
                    "        string unresolvedAssertion = \"{reason}\";\n        Assert.Null(unresolvedAssertion);\n"
                )
            }
            InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
                fact_attribute = format!("    [Fact(Skip = \"{reason}\")]");
                String::new()
            }
        };
        body.truncate(assertions_start);
        body.push_str(&inert_example::refusal_body(&markers, &statement));
    }

    body.push_str("    }\n");

    for line in std::iter::once(fact_attribute.as_str()).chain(body.lines()) {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit assertions for non-chat-completion streams by checking which fields are
/// supported in result_fields. Skip unsupported assertions as comments.
///
/// This function replaces the hardcoded chat-completion assertions for generic
/// streaming types (like CrawlEvent) that have different field names.
fn emit_non_chat_stream_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_fields: &std::collections::HashSet<String>,
) {
    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }
    let field = assertion.field.as_deref().unwrap_or("");

    // Virtual fields that don't depend on result_fields
    match field {
        // A non-chat stream item type carries no terminal finish_reason, which is the only thing
        // `stream_complete` is defined by. The previous `Assert.True(streamComplete)` here read a
        // local the body assigned `true` unconditionally, so it could never fail; refusing loudly
        // is the honest reading, and the wording feeds `fail_on_unavailable_field_markers`. ~keep
        "stream_complete" => {
            let _ = writeln!(
                out,
                "        // skipped: {}; this stream's item type carries no terminal finish_reason, \
so completion is not observable here",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
            );
            return;
        }
        // Rendered against the post-completion enumerator probe `render_streaming_test_method`
        // emits whenever a fixture asserts this field. It replaces a literal `Assert.True(true)`,
        // which asserted nothing whatever the stream did. ~keep
        "no_chunks_after_done" => {
            match atype {
                "is_true" => {
                    let _ = writeln!(out, "        Assert.True(noChunksAfterDone);");
                }
                "is_false" => {
                    let _ = writeln!(out, "        Assert.False(noChunksAfterDone);");
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "        // skipped: assertion type '{atype}' on field '{field}' not yet supported for streaming"
                    );
                }
            }
            return;
        }
        // ~keep Both `count_min`/`count_equals` arms below used to guard on `as_u64()` with no
        // `else`, so a fixture value that did not narrow to a `u64` rendered NOTHING: no
        // assertion, no skip comment. Routed through the same funnel every other narrowing guard
        // in this file now uses.
        "chunks" | "stream.items" => match atype {
            "count_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(out, "        Assert.True(chunks.Count >= {n});");
                } else {
                    let _ = writeln!(
                        out,
                        "{}",
                        streaming_assertion_value_skip_line("        ", "//", field, atype)
                    );
                }
                return;
            }
            "count_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(out, "        Assert.Equal({n}, chunks.Count);");
                } else {
                    let _ = writeln!(
                        out,
                        "{}",
                        streaming_assertion_value_skip_line("        ", "//", field, atype)
                    );
                }
                return;
            }
            _ => {}
        },
        _ => {}
    }

    // For fields that depend on result_fields, check if they're supported
    if !result_fields.iter().any(|f| field.starts_with(f)) {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        );
        return;
    }

    // Fields in result_fields can be asserted via chunks[i].FieldName
    match atype {
        "not_empty" => {
            let _ = writeln!(out, "        Assert.NotEmpty(chunks);");
        }
        "is_empty" => {
            let _ = writeln!(out, "        Assert.Empty(chunks);");
        }
        _ => {
            let _ = writeln!(
                out,
                "        // skipped: assertion type '{atype}' on field '{field}' not yet supported for streaming"
            );
        }
    }
}

/// Map a streaming fixture assertion to an `Assert` call on the local aggregator
/// variable produced by `render_chat_stream_test_method`. Pseudo-fields like
/// `chunks` / `stream_content` / `stream_complete` resolve to in-method locals.
fn emit_chat_stream_assertion(out: &mut String, assertion: &Assertion) {
    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }
    let field = assertion.field.as_deref().unwrap_or("");

    enum Kind {
        Chunks,
        Bool,
        Str,
        IntTokens,
        Json,
        Unsupported,
    }

    let (expr, kind) = match field {
        "chunks" => ("chunks", Kind::Chunks),
        "stream_content" => ("streamContent.ToString()", Kind::Str),
        "stream_complete" => ("streamComplete", Kind::Bool),
        // Distinct local, distinct fact. Aliasing this onto `streamComplete` collapsed two
        // assertions onto one flag the body assigned `true` unconditionally, so a fixture
        // asserting both got one unfalsifiable check rendered twice. ~keep
        "no_chunks_after_done" => ("noChunksAfterDone", Kind::Bool),
        "finish_reason" => ("lastFinishReason", Kind::Str),
        "tool_calls" => ("toolCallsJson", Kind::Json),
        "tool_calls[0].function.name" => ("toolCalls0FunctionName", Kind::Str),
        "usage.total_tokens" => ("totalTokens", Kind::IntTokens),
        _ => ("", Kind::Unsupported),
    };

    if matches!(kind, Kind::Unsupported) {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        );
        return;
    }

    // ~keep Every `if let Some(...) = ...` guard below used to have no `else`: a fixture value
    // that did not narrow to the expected shape (`as_u64()`, or bare presence for `equals`/
    // `contains`, which accept any JSON value) rendered NOTHING -- no assertion, no skip comment.
    // The catch-all default also used to emit ad hoc text
    // ("streaming assertion '<t>' on field '<f>' not supported") matching neither `FieldSkip`'s
    // nor `AssertionTypeSkip`'s registered wording, invisible to the strict gate even though a
    // line was present. All of these now route through the same funnel
    // `emit_non_chat_stream_assertion` in this file, and every other backend's streaming
    // renderer, already uses.
    match (atype, &kind) {
        ("count_min", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(
                    out,
                    "        Assert.True(chunks.Count >= {n}, \"expected at least {n} chunks\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        ("count_equals", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "        Assert.Equal({n}, chunks.Count);");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        ("equals", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);
                let _ = writeln!(out, "        Assert.Equal({cs_val}, {expr});");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        ("contains", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let cs_val = json_to_csharp(val);
                let _ = writeln!(out, "        Assert.Contains({cs_val}, {expr} ?? string.Empty);");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        ("not_empty", Kind::Str) => {
            let _ = writeln!(
                out,
                "        Assert.False(string.IsNullOrEmpty({expr} ?? string.Empty));"
            );
        }
        ("not_empty", Kind::Json) => {
            let _ = writeln!(out, "        Assert.NotNull({expr});");
        }
        ("is_empty", Kind::Str) => {
            let _ = writeln!(
                out,
                "        Assert.True(string.IsNullOrEmpty({expr} ?? string.Empty));"
            );
        }
        ("is_true", Kind::Bool) => {
            let _ = writeln!(out, "        Assert.True({expr});");
        }
        ("is_false", Kind::Bool) => {
            let _ = writeln!(out, "        Assert.False({expr});");
        }
        ("greater_than_or_equal", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "        Assert.True({expr} >= {n}, \"expected >= {n}\");");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        ("equals", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "        Assert.Equal((long?){n}, {expr});");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_value_skip_line("        ", "//", field, atype)
                );
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "{}",
                streaming_assertion_type_skip_line("        ", "//", field, atype)
            );
        }
    }
}

#[cfg(test)]
mod strict_field_availability_marker_tests {
    use super::emit_non_chat_stream_assertion;
    use crate::e2e::fixture::Assertion;

    /// Regression test for alef task #81: csharp's streaming path
    /// (`emit_non_chat_stream_assertion`) has its own "unsupported field" skip
    /// comment, structurally separate from `csharp/assertions.rs`'s
    /// `render_assertion`. The shared `fail_on_unavailable_field_markers`
    /// mechanism (wired into `render_streaming_test_method` just above) matches
    /// on this exact wording, so arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY`
    /// turns a dropped streaming assertion into a generation-time failure
    /// instead of a silently-passing comment.
    #[test]
    fn unavailable_streaming_field_skip_comment_carries_the_strict_mode_marker() {
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("weird_field".to_string()),
            ..Assertion::default()
        };
        let mut out = String::new();

        emit_non_chat_stream_assertion(&mut out, &assertion, &std::collections::HashSet::new());

        assert!(
            out.contains("streaming assertion on unsupported field 'weird_field'"),
            "got: {out}"
        );
    }
}

#[cfg(test)]
mod emit_chat_stream_assertion_tests {
    use super::emit_chat_stream_assertion;
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
    use crate::e2e::fixture::Assertion;

    /// ~keep Before this change, `count_min` on `chunks` with a fixture `value` that did not
    /// narrow to a `u64` (here a string) rendered NOTHING: the `if let Some(n) = ...` guard had
    /// no `else`. This is the regression test: a line must be emitted at all, and it must be the
    /// funnel's registered wording.
    #[test]
    fn count_min_with_unnarrowable_value_emits_a_line_instead_of_vanishing() {
        let assertion = Assertion {
            assertion_type: "count_min".into(),
            field: Some("chunks".into()),
            value: Some(serde_json::json!("not-a-number")),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out,
            "        // skipped: assertion type 'count_min' has no renderable value for streaming field \
             'chunks'\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
            "the rendered line must round-trip through the assertion-type funnel, got: {out}"
        );
    }

    /// ~keep `equals`/`contains` on `stream_content` (`Kind::Str`) guard only on `Some(val)`, but
    /// with no `else` a fixture that omitted `value` entirely rendered nothing at all.
    #[test]
    fn equals_with_no_declared_value_emits_a_line_instead_of_vanishing() {
        let assertion = Assertion {
            assertion_type: "equals".into(),
            field: Some("stream_content".into()),
            value: None,
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out,
            "        // skipped: assertion type 'equals' has no renderable value for streaming field \
             'stream_content'\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("equals", AssertionTypeSkip::StreamingAssertionValueNotRenderable))
        );
    }

    /// ~keep Before this change the catch-all arm emitted ad hoc text
    /// (`streaming assertion '<t>' on field '<f>' not supported`) that matched neither
    /// `FieldSkip`'s nor `AssertionTypeSkip`'s registered shape. Exact rendered output, not
    /// `contains`, and a round trip through the funnel that would fail if the wording drifted
    /// back to the old ad hoc text or diverged from `emit_non_chat_stream_assertion`'s wording
    /// for the same variant.
    #[test]
    fn unsupported_assertion_type_on_a_supported_field_is_recognised_by_the_funnel() {
        let assertion = Assertion {
            assertion_type: "matches_regex".into(),
            field: Some("chunks".into()),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out,
            "        // skipped: assertion type 'matches_regex' on field 'chunks' not yet supported for streaming\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "the rendered line must round-trip through the assertion-type funnel, got: {out}"
        );
    }

    /// A matched, well-formed assertion must still render a real `Assert...`, not a skip
    /// comment -- the fix must not regress the happy path.
    #[test]
    fn count_min_with_a_narrowable_value_still_renders_a_real_assertion() {
        let assertion = Assertion {
            assertion_type: "count_min".into(),
            field: Some("chunks".into()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out, "        Assert.True(chunks.Count >= 2, \"expected at least 2 chunks\");\n",
            "got: {out}"
        );
    }
}
