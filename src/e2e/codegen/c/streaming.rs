//! C e2e streaming adapter test generation.

use crate::core::config::{AdapterPattern, ResolvedCrateConfig};
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::transform_json_keys_for_language;
use crate::e2e::escape::escape_c;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::ToSnakeCase;
use std::fmt::Write as FmtWrite;

pub(super) struct CStreamingAdapterMetadata {
    owner_type: String,
    item_type: String,
    request_type: String,
    adapter_name: String,
}

pub(super) fn resolve_c_streaming_adapter(
    config: &ResolvedCrateConfig,
    function_name: &str,
) -> Option<CStreamingAdapterMetadata> {
    config
        .adapters
        .iter()
        .find(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.name == function_name)
        .and_then(|adapter| {
            Some(CStreamingAdapterMetadata {
                owner_type: adapter.owner_type.clone()?,
                item_type: adapter.item_type.clone()?,
                request_type: adapter
                    .request_type
                    .as_deref()
                    .and_then(|path| path.rsplit("::").next())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)?,
                adapter_name: adapter.name.clone(),
            })
        })
}

pub(super) fn resolve_c_client_owner_type(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    function_name: &str,
) -> Option<String> {
    config
        .adapters
        .iter()
        .find(|adapter| {
            matches!(adapter.pattern, AdapterPattern::Streaming | AdapterPattern::AsyncMethod)
                && adapter.name == function_name
        })
        .and_then(|adapter| adapter.owner_type.clone())
        .or_else(|| {
            type_defs.iter().find_map(|type_def| {
                type_def
                    .methods
                    .iter()
                    .any(|method| method.name == function_name)
                    .then(|| type_def.name.clone())
            })
        })
        .or_else(|| {
            let opaque_types: Vec<&crate::core::ir::TypeDef> =
                type_defs.iter().filter(|type_def| type_def.is_opaque).collect();
            (opaque_types.len() == 1).then(|| opaque_types[0].name.clone())
        })
}

pub(super) fn validate_c_snippet_metadata(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
    function_name: &str,
    client_factory: Option<&str>,
    engine_factory: Option<&str>,
    streaming: Option<bool>,
) -> anyhow::Result<()> {
    if engine_factory.is_some() || client_factory.is_none() {
        return Ok(());
    }
    if crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, streaming)
        && resolve_c_streaming_adapter(config, function_name).is_none()
    {
        anyhow::bail!("streaming fixture requires matching [[crates.adapters]] metadata for C snippet generation");
    }
    if resolve_c_client_owner_type(config, type_defs, function_name).is_none() {
        anyhow::bail!("client_factory is configured but C snippet generation could not resolve the client owner type");
    }
    Ok(())
}

pub(super) fn render_c_diagnostic_skip(out: &mut String, reason: &str) {
    let escaped = escape_c(reason);
    let _ = writeln!(out, "    fprintf(stderr, \"skipped: {escaped}\\n\");");
    let _ = writeln!(out, "}}");
}

/// Emit a streaming-adapter test function that drives the FFI iterator handle.
///
/// Calls the adapter-derived `{prefix}_{owner}_{method}_start` function to
/// obtain an opaque handle, loops over the corresponding `_next` function until
/// it returns null,
/// and aggregates per-chunk data into local variables (`chunks_count`,
/// `stream_content`, `last_choices_json`, ...). Fixture
/// assertions on streaming pseudo-fields (`chunks`, `no_chunks_after_done`) are
/// translated to assertions on these locals. Chat-specific field extraction
/// remains best effort and unsupported fields — `stream_content`,
/// `stream_complete`, `finish_reason`, `tool_calls`,
/// `tool_calls[0].function.name`, `usage.total_tokens` — are skipped by
/// `emit_chat_stream_assertion`.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_streaming_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    client_factory: &str,
    streaming: &CStreamingAdapterMetadata,
    expects_error: bool,
    api_key_var: Option<&str>,
    documentation_snippet: bool,
) {
    // cbindgen's `[export] prefix` (shouty-snake), not a bare uppercase — see
    // `c_consumer::export_type_prefix`. ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);
    let owner_snake = streaming.owner_type.to_snake_case();
    let request_type_pascal = &streaming.request_type;
    let request_type_snake = request_type_pascal.to_snake_case();
    let item_type_pascal = &streaming.item_type;
    let item_type_snake = item_type_pascal.to_snake_case();
    let adapter_name = &streaming.adapter_name;
    let stream_start = format!("{prefix}_{owner_snake}_{adapter_name}_start");
    let stream_next = format!("{prefix}_{owner_snake}_{adapter_name}_next");
    let stream_free = format!("{prefix}_{owner_snake}_{adapter_name}_free");

    let mut request_var: Option<String> = None;
    for arg in args {
        if arg.arg_type == "json_object" {
            let var_name = format!("{request_type_snake}_handle");

            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_val = if field.is_empty() || field == "input" {
                Some(&fixture.input)
            } else {
                fixture.input.get(field)
            };

            if let Some(val) = json_val
                && !val.is_null()
            {
                let normalized = transform_json_keys_for_language(val, "snake_case");
                let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                let escaped = escape_c(&json_str);
                let _ = writeln!(
                    out,
                    "    {prefix_upper}AlefHandle {var_name} = \
                         {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                );
                let _ = writeln!(out, "    assert({var_name} != 0 && \"failed to build request\");");
                request_var = Some(var_name);
                break;
            }
        }
    }

    // ~keep This loop only ever considers `json_object` args, which are always materialized
    // as a scalar `AlefHandle` via `_from_json(...)`; when absent, `0` is the handle's
    // "none" sentinel, not the pointer sentinel `NULL`.
    let req_handle = request_var.clone().unwrap_or_else(|| "0".to_string());
    let req_snake = request_var
        .as_ref()
        .and_then(|v| v.strip_suffix("_handle"))
        .unwrap_or(request_type_snake.as_str())
        .to_string();

    let fixture_id = &fixture.id;
    // ~keep A documentation snippet is published verbatim to readers, so neither the
    // mock-server wiring nor the literal `"test-key"` credential below may reach it —
    // mirrors `test_function::render_test_function`'s own `has_mock`, which already ANDs
    // with `!documentation_snippet` and declares the `api_key` local the docs branch
    // reads. Test mode passes `false`, leaving both mock branches byte-for-byte intact.
    let has_mock = fixture.needs_mock_server() && !documentation_snippet;
    if has_mock && api_key_var.is_some() {
        // `api_key` and `base_url_buf` are already declared by the env-fallback
        // block above (the smoke+mock path). Reuse them — don't redeclare
        // `mock_base`/`base_url`, which would be a C compile error.
        // use_mock was captured before api_key was potentially reassigned to "test-key",
        // so it correctly reflects the original env state.
        let _ = writeln!(out, "    const char* _base_url_arg = use_mock ? base_url_buf : NULL;");
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{client_factory}(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else if has_mock {
        let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
        let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
        let _ = writeln!(out, "    char base_url[1024];");
        let _ = writeln!(
            out,
            "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
        );
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{client_factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else if documentation_snippet {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{client_factory}(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle client = {prefix}_{client_factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != 0 && \"failed to create client\");");

    // The streaming opaque handle is a Rust type named `{Prefix}{Owner}{Method}StreamHandle`;
    // cbindgen additionally prepends the configured uppercase type-name `prefix` (e.g. `SAMPLELLM`),
    // exactly as it does for ordinary opaque handle types like `{prefix_upper}{owner_type}`.
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle stream_handle = \
         {stream_start}(client, {req_handle});"
    );

    if expects_error {
        let _ = writeln!(
            out,
            "    assert(stream_handle == 0 && \"expected stream-start to fail\");"
        );
        if request_var.is_some() {
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
        }
        let _ = writeln!(out, "    {prefix}_{owner_snake}_free(client);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(
        out,
        "    assert(stream_handle != 0 && \"expected stream-start to succeed\");"
    );

    let _ = writeln!(out, "    size_t chunks_count = 0;");
    let _ = writeln!(out, "    char* stream_content = (char*)malloc(1);");
    let _ = writeln!(out, "    assert(stream_content != NULL);");
    let _ = writeln!(out, "    stream_content[0] = '\\0';");
    let _ = writeln!(out, "    size_t stream_content_len = 0;");
    // `no_chunks_after_done` is a structural invariant, not tracked state: the `while` loop
    // below calls `stream_next` only until it returns 0/done and `break`s immediately, so no
    // code path can call `stream_next` again afterward -- there is no way for this loop to
    // observe a chunk arriving after done. Every other backend encodes the same fact the same
    // way: `StreamingFieldResolver::accessor`'s `no_chunks_after_done` arm
    // (`streaming_assertions/accessors.rs`) returns the literal `"true"` for every language
    // without post-DONE chunk plumbing, exactly matching this `1` that is never reassigned.
    // This is documented at `streaming_assertions/model.rs`'s field-name table. If a future
    // adapter design exposes post-DONE chunks as an observable event, THAT is where tracking
    // would need to start — not here, where "done" and "no more chunks" are the same event. ~keep
    let _ = writeln!(
        out,
        "    int no_chunks_after_done = 1; /* true by construction: the loop below cannot call stream_next again once it observes done */"
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "    while (1) {{");
    let _ = writeln!(
        out,
        "        {prefix_upper}AlefHandle {result_var} = {stream_next}(stream_handle);"
    );
    out.push_str(&crate::e2e::template_env::render(
        "c/stream_exhausted_branch.jinja",
        crate::alef_context! { result_var => result_var, prefix => prefix },
    ));
    let _ = writeln!(out, "        chunks_count++;");
    let _ = writeln!(out, "        {prefix}_{item_type_snake}_free({result_var});");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    {stream_free}(stream_handle);");
    let _ = writeln!(out);

    for assertion in &fixture.assertions {
        emit_chat_stream_assertion(out, assertion);
    }

    let _ = writeln!(out, "    free(stream_content);");
    if request_var.is_some() {
        let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
    }
    let _ = writeln!(out, "    {prefix}_{owner_snake}_free(client);");
    let _ = writeln!(
        out,
        "    /* suppress unused */ (void)no_chunks_after_done; \
         (void)chunks_count; (void)stream_content_len;"
    );
    let _ = writeln!(out, "}}");
}

/// Emit a single fixture assertion for a streaming test, mapping fixture
/// pseudo-field references (`chunks`, `no_chunks_after_done`, ...)
/// to the local aggregator variables built by [`render_streaming_test_function`].
fn emit_chat_stream_assertion(out: &mut String, assertion: &Assertion) {
    // `mock.*` (alef issue #443) never reaches `assertions::render_assertion` for a streaming
    // fixture -- this function is the split-dispatch path the issue's own intel table named `c`
    // for. Intercept first, exactly like that shared oracle does. ~keep
    if super::assertion_mock_capture::try_render_mock_capture_assertion(out, assertion) {
        return;
    }

    let field = assertion.field.as_deref().unwrap_or("");

    enum Kind {
        IntCount,
        Bool,
        Unsupported,
    }

    let (expr, kind) = match field {
        "chunks" => ("chunks_count", Kind::IntCount),
        "no_chunks_after_done" => ("no_chunks_after_done", Kind::Bool),
        // `stream_complete` joins the unsupported set rather than keeping a local of its own.
        // The driver loop above frees each chunk handle without reading a single field from it,
        // and `finish_reason` is unsupported on this backend for exactly that reason — so nothing
        // here can observe the terminal marker the cross-backend field is defined by
        // (`streaming_assertions/model.rs`'s field table). The local this used to resolve to was
        // set from loop termination alone: a different fact wearing this field's name, and one
        // the loop now asserts directly against the ABI's error code. ~keep
        "stream_complete"
        | "stream_content"
        | "finish_reason"
        | "tool_calls"
        | "tool_calls[0].function.name"
        | "usage.total_tokens" => ("", Kind::Unsupported),
        _ => ("", Kind::Unsupported),
    };

    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }

    if matches!(kind, Kind::Unsupported) {
        let _ = writeln!(
            out,
            "    /* skipped: {} */",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        );
        return;
    }

    // ~keep The `count_min`/`greater_than_or_equal`/`equals` arms below used to render nothing at
    // all when `assertion.value` did not narrow to a `u64` — no assertion AND no skip comment, the
    // exact silent-vanish shape `AssertionTypeSkip::StreamingAssertionValueNotRenderable` exists to
    // name. The catch-all arm at the bottom used to emit ad hoc text
    // ("streaming assertion '<t>' on field '<f>' not supported") that matches neither
    // `FieldSkip`'s nor `AssertionTypeSkip`'s registered wording, so it rendered a comment no
    // strict gate could see. Both are now routed through the same funnel every other backend
    // (dart/elixir/go/java/kotlin/php/swift/typescript/zig) already uses.
    match (atype, &kind) {
        ("count_min", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} >= {n} && \"expected at least {n} chunks\");");
            } else {
                let _ = writeln!(
                    out,
                    "{} */",
                    streaming_assertion_value_skip_line("    ", "/*", field, atype)
                );
            }
        }
        ("is_true", Kind::Bool) => {
            let _ = writeln!(out, "    assert({expr} && \"expected {field} to be true\");");
        }
        ("is_false", Kind::Bool) => {
            let _ = writeln!(out, "    assert(!{expr} && \"expected {field} to be false\");");
        }
        ("greater_than_or_equal", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} >= {n} && \"expected {expr} >= {n}\");");
            } else {
                let _ = writeln!(
                    out,
                    "{} */",
                    streaming_assertion_value_skip_line("    ", "/*", field, atype)
                );
            }
        }
        ("equals", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} == {n} && \"equals assertion failed\");");
            } else {
                let _ = writeln!(
                    out,
                    "{} */",
                    streaming_assertion_value_skip_line("    ", "/*", field, atype)
                );
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "{} */",
                streaming_assertion_type_skip_line("    ", "/*", field, atype)
            );
        }
    }
}

#[cfg(test)]
mod emit_chat_stream_assertion_tests {
    use super::emit_chat_stream_assertion;
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
    use crate::e2e::fixture::Assertion;

    /// The split-dispatch regression alef issue #443 names `c` for: this function never calls
    /// `assertions::render_assertion`, so a streaming fixture asserting `mock.*` reaches ONLY
    /// this dispatcher. Before the interception at the top of `emit_chat_stream_assertion`, a
    /// `mock.*` field fell through to `Kind::Unsupported` (the `_` arm on `field`) and rendered
    /// `FieldSkip::StreamingAssertionOnUnsupportedField` -- alef's own capture gap misreported as
    /// a fixture authoring problem. ~keep
    #[test]
    fn a_mock_capture_assertion_intercepts_before_the_unsupported_field_fallback() {
        let assertion = Assertion {
            assertion_type: "equals".into(),
            field: Some("mock.requests.total".into()),
            value: Some(serde_json::json!(2)),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out,
            "    assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 2 && \"equals assertion failed\");\n",
            "got: {out}"
        );
    }

    /// ~keep Before this change, a `count_min` on `chunks` whose fixture `value` was not a `u64`
    /// (here a string) rendered NOTHING: the `if let Some(n) = ...` guard had no `else`, so the
    /// assertion vanished with no line for any funnel to see -- the exact silent-vanish shape
    /// `AssertionTypeSkip::StreamingAssertionValueNotRenderable` exists to name. This is the
    /// regression the fix closes: a line must be emitted at all, and it must be the funnel's
    /// registered wording, not merely non-empty.
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
            out, "    /* skipped: assertion type 'count_min' has no renderable value for streaming field 'chunks' */\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
            "the rendered line must round-trip through the assertion-type funnel, got: {out}"
        );
    }

    /// ~keep Mirrors the `count_min` case above for `greater_than_or_equal`, the other
    /// `Kind::IntCount` arm that guards on `as_u64()` without an `else`.
    #[test]
    fn greater_than_or_equal_with_unnarrowable_value_emits_a_line_instead_of_vanishing() {
        let assertion = Assertion {
            assertion_type: "greater_than_or_equal".into(),
            field: Some("chunks".into()),
            value: Some(serde_json::json!(1.5)),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);
        assert_eq!(
            out,
            "    /* skipped: assertion type 'greater_than_or_equal' has no renderable value for streaming field \
             'chunks' */\n",
            "got: {out}"
        );
    }

    /// ~keep Before this change the catch-all arm emitted ad hoc text
    /// (`streaming assertion '<t>' on field '<f>' not supported`) that matched neither
    /// `FieldSkip`'s nor `AssertionTypeSkip`'s registered shape, so a strict census walked right
    /// past it even though a line was present. This is the load-bearing assertion: exact rendered
    /// output, not `contains`, and a round trip through the funnel that would fail if the wording
    /// drifted back to the old ad hoc text.
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
            "    /* skipped: assertion type 'matches_regex' on field 'chunks' not yet supported for streaming */\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "the rendered line must round-trip through the assertion-type funnel, got: {out}"
        );
    }

    /// A matched, well-formed assertion must still render a real `assert(...)`, not a skip
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
            out, "    assert(chunks_count >= 2 && \"expected at least 2 chunks\");\n",
            "got: {out}"
        );
    }
}

#[cfg(test)]
mod stream_completion_signal_tests {
    use super::{CStreamingAdapterMetadata, emit_chat_stream_assertion, render_streaming_test_function};
    use crate::e2e::codegen::field_skip::FieldSkip;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn adapter() -> CStreamingAdapterMetadata {
        CStreamingAdapterMetadata {
            owner_type: "Client".to_string(),
            item_type: "StreamChunk".to_string(),
            request_type: "ChatRequest".to_string(),
            adapter_name: "chat_stream".to_string(),
        }
    }

    fn driver_body(assertions: Vec<Assertion>) -> String {
        let fixture = Fixture {
            id: "stream_terminates_cleanly".to_string(),
            description: "streaming call terminates cleanly".to_string(),
            assertions,
            ..Fixture::default()
        };
        let mut out = String::new();
        render_streaming_test_function(
            &mut out,
            &fixture,
            "mylib",
            "chunk",
            &[],
            "client_new",
            &adapter(),
            false,
            None,
            false,
        );
        out
    }

    /// The defect: the driver declared `int stream_complete = 0;` and set it to `1` on any clean
    /// loop exit, then let a fixture's `stream_complete` assertion read it. Nothing in that path
    /// ever inspects a chunk — the loop frees each handle without reading a field — so the local
    /// answered "the iterator ran out without setting an error code" while wearing the name of a
    /// field the cross-backend contract defines as "the last chunk carries a terminal
    /// finish_reason". The real observation stays, as an assert on the ABI's own error code.
    #[test]
    fn the_driver_asserts_the_abi_error_code_instead_of_flagging_an_invented_completion() {
        let body = driver_body(vec![]);

        // Positive first: a driver loop was emitted at all, so the absences below mean something.
        assert!(
            body.contains("mylib_client_chat_stream_next(stream_handle);"),
            "the driver loop must render before its contents are judged. Emitted:\n{body}"
        );
        assert!(
            body.contains("assert(mylib_last_error_code() == 0 && \"the stream ended with an error\");"),
            "loop termination must be asserted directly on the ABI error code. Emitted:\n{body}"
        );
        assert!(
            !body.contains("stream_complete"),
            "no local may carry the `stream_complete` name this backend cannot substantiate. \
             Emitted:\n{body}"
        );
    }

    /// The other half: with the local gone, a fixture that really does declare `stream_complete`
    /// must leave a registered skip the ledger can count, not silently vanish and not resolve to
    /// some other backend's aggregator name.
    #[test]
    fn a_stream_complete_assertion_renders_the_registered_skip() {
        let assertion = Assertion {
            assertion_type: "is_true".into(),
            field: Some("stream_complete".into()),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);

        assert_eq!(
            out,
            format!(
                "    /* skipped: {} */\n",
                FieldSkip::StreamingAssertionOnUnsupportedField.message("stream_complete")
            ),
            "got: {out}"
        );
    }

    /// Negative control for the arm above. `no_chunks_after_done` is backed by a local the driver
    /// really does establish structurally, so it must keep rendering a real `assert(...)` — a fix
    /// that skipped every boolean streaming field would pass the test above and fail this one.
    #[test]
    fn no_chunks_after_done_still_renders_a_real_assertion() {
        let assertion = Assertion {
            assertion_type: "is_true".into(),
            field: Some("no_chunks_after_done".into()),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion);

        assert_eq!(
            out, "    assert(no_chunks_after_done && \"expected no_chunks_after_done to be true\");\n",
            "got: {out}"
        );
    }
}
