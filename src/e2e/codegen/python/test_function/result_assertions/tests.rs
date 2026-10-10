//! Unit tests for `result_assertions.rs`.
//!
//! Split out into its own file to keep `result_assertions.rs` under the repo's 1,000-line
//! file-modularization cap, mirroring `python/assertions.rs`'s own `assertions/tests.rs` split.

use super::*;

fn assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        skip: None,
        assertion_type: assertion_type.to_string(),
        field: field.map(str::to_string),
        value,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn minimal_fixture() -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "widget_smoke".to_string(),
        description: "Create a widget".to_string(),
        input: serde_json::Value::Null,
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: Vec::new(),
        call: None,
        skip: None,
        env: None,
        setup: Vec::new(),
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        mock_response: None,
        source: String::new(),
        category: None,
        tags: Vec::new(),
    }
}

/// Builds a [`PythonTypedDictMap`] classifying `typeddict_types` as `TypedDict`, with the given
/// `(owner, field, target)` traversal edges — mirrors the helper in
/// `field_access::python_renderer`'s own tests.
fn typeddict_map(
    typeddict_types: &[&str],
    field_types: &[(&str, &str, &str)],
) -> crate::e2e::field_access::PythonTypedDictMap {
    let mut map = crate::e2e::field_access::PythonTypedDictMap {
        typeddict_types: typeddict_types.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    for (owner, field, target) in field_types {
        map.field_types
            .entry(owner.to_string())
            .or_default()
            .insert(field.to_string(), target.to_string());
    }
    map
}

#[test]
fn streaming_virtual_assertion_renders_collected_chunks_access() {
    let mut out = String::new();
    let assertion = assertion("count_min", Some("chunks"), Some(serde_json::Value::from(1)));

    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "chunks",
        "chunks",
        None,
        &crate::e2e::field_access::PythonTypedDictMap::default(),
    );

    assert!(out.contains("assert len(chunks) >= 1"), "got: {out}");
}

#[test]
fn not_empty_for_python_streaming_rejects_empty_chunks_but_accepts_zero() {
    let mut out = String::new();
    let assertion = assertion("not_empty", Some("chunks"), None);

    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "chunks",
        "chunks",
        None,
        &crate::e2e::field_access::PythonTypedDictMap::default(),
    );

    // Bare `assert chunks` fails on a legitimate 0, 0.0 or False.
    assert_eq!(
        out.trim(),
        "assert chunks is not None and (not hasattr(chunks, \"__len__\") or len(chunks) > 0)"
    );
}

/// A streaming accessor over a `TypedDict`-classified chunk type subscripts every hop instead
/// of the pre-existing dotted access — the exact defect this task fixes:
/// `stream_content`/`stream_complete`/`tool_calls`/`finish_reason`/`usage` are hand-rolled in
/// `streaming_assertions::accessors` and did not go through `FieldResolver`/`PathSegment`, so
/// 0.75.0's TypedDict fix (which only covers ordinary, non-streaming field paths) left them
/// blind to `[workspace.dto] python_output = "typed-dict"`.
#[test]
fn finish_reason_streaming_accessor_over_a_typeddict_chunk_subscripts() {
    let map = typeddict_map(&["Chunk", "Choice"], &[("Chunk", "choices", "Choice")]);
    let mut out = String::new();
    let assertion = assertion("equals", Some("finish_reason"), Some(serde_json::json!("stop")));

    emit_streaming_virtual_assertion(&mut out, &assertion, "finish_reason", "chunks", Some("Chunk"), &map);

    assert_eq!(
        out.trim(),
        "assert (str(chunks[-1][\"choices\"][0][\"finish_reason\"]) if chunks and \
         chunks[-1][\"choices\"] else None).strip() == \"stop\".strip()"
    );
}

/// CONTROL for the same fixture: a chunk type NOT classified `TypedDict` keeps the pre-existing
/// dotted access, proving the new subscript rendering is conditional on the map, not blanket.
#[test]
fn finish_reason_streaming_accessor_over_a_non_typeddict_chunk_stays_dotted() {
    let map = typeddict_map(&[], &[("Chunk", "choices", "Choice")]);
    let mut out = String::new();
    let assertion = assertion("equals", Some("finish_reason"), Some(serde_json::json!("stop")));

    emit_streaming_virtual_assertion(&mut out, &assertion, "finish_reason", "chunks", Some("Chunk"), &map);

    assert_eq!(
        out.trim(),
        "assert (str(chunks[-1].choices[0].finish_reason) if chunks and chunks[-1].choices else \
         None).strip() == \"stop\".strip()"
    );
}

/// A streaming path descending from a `TypedDict` chunk into a nested field whose OWN type
/// stays native (not itself classified `TypedDict`) switches back to attribute access at that
/// link — mirrors `field_access::python_renderer`'s per-segment (not per-root) dispatch.
#[test]
fn tool_calls_deep_path_switches_back_to_attribute_access_past_a_non_typeddict_link() {
    let map = typeddict_map(
        &["Chunk", "Choice", "Delta", "ToolCall"],
        &[
            ("Chunk", "choices", "Choice"),
            ("Choice", "delta", "Delta"),
            ("Delta", "tool_calls", "ToolCall"),
            // `ToolCall.function` resolves to a type NOT in `typeddict_types` -- e.g. it stays a
            // native `#[pyclass]` because it is reexported. `function`'s own fields must render
            // as attribute access even though every ancestor on the path is subscripted.
            ("ToolCall", "function", "FunctionCall"),
        ],
    );
    let mut out = String::new();
    let assertion = assertion(
        "equals",
        Some("tool_calls[0].function.name"),
        Some(serde_json::json!("lookup")),
    );

    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "tool_calls[0].function.name",
        "chunks",
        Some("Chunk"),
        &map,
    );

    assert_eq!(
        out.trim(),
        "assert ([t for c in chunks for ch in (c[\"choices\"] or []) for t in \
         (ch[\"delta\"][\"tool_calls\"] or [])])[0][\"function\"].name.strip() == \"lookup\".strip()"
    );
}

/// CONTROL: the identical non-streaming (`tool_calls[0].id`) deep path renders byte-identical
/// output before and after this task's change, when no `TypedDict` map is supplied — proving the
/// fix is additive and does not disturb the pre-existing default.
#[test]
fn tool_calls_deep_path_without_a_typeddict_map_stays_dotted() {
    let mut out = String::new();
    let assertion = assertion("equals", Some("tool_calls[0].id"), Some(serde_json::json!("abc")));

    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "tool_calls[0].id",
        "chunks",
        None,
        &crate::e2e::field_access::PythonTypedDictMap::default(),
    );

    assert_eq!(
        out.trim(),
        "assert ([t for c in chunks for ch in (c.choices or []) for t in (ch.delta.tool_calls or \
         [])])[0].id.strip() == \"abc\".strip()"
    );
}

#[test]
fn call_statement_omits_binding_when_the_result_is_unused() {
    let rendered = crate::e2e::template_env::render(
        "python/call_statement.py.jinja",
        crate::alef_context! { result_binding => Option::<&str>::None, call_expr => "await process(value)" },
    );

    assert_eq!(rendered, "    await process(value)\n");
    assert!(!rendered.contains("result ="));
    assert!(!rendered.contains("_ ="));
}

#[test]
fn should_bind_result_when_force_bind_result_is_set_with_no_assertions() {
    let fixture = minimal_fixture();
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "await widget_client.create()",
        "result",
        &field_resolver,
        false,
        false,
        false,
        true,
        None,
    );

    assert!(
        out.contains("result = await widget_client.create()"),
        "expected the call result to be bound so a caller can print it, got: {out}"
    );
}

#[test]
fn should_discard_result_when_force_bind_result_is_unset_and_unused() {
    let fixture = minimal_fixture();
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "await widget_client.create()",
        "result",
        &field_resolver,
        false,
        false,
        false,
        false,
        None,
    );

    assert!(!out.contains("result ="), "unused result must not be bound, got: {out}");
}

#[test]
fn has_real_assertion_is_false_for_comment_only_body() {
    let body = "    # skipped: field 'foo' not available on result type\n";
    assert!(
        !has_real_assertion(body),
        "comment-only body must not count as asserting"
    );
}

#[test]
fn has_real_assertion_is_true_when_a_real_statement_is_present() {
    let body = "    # skipped: field 'foo' not available on result type\n    assert result.ok\n";
    assert!(has_real_assertion(body), "a real assert line must count as asserting");
}

#[test]
fn vacuous_fallback_is_a_noop_without_declared_assertions() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, false, "result", false);
    assert!(
        body.is_empty(),
        "a fixture with no declared assertions is an intentional smoke test and must stay untouched"
    );
}

#[test]
fn vacuous_fallback_emits_a_real_assertion_when_body_is_empty() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, true, "result", false);
    assert_eq!(body, "    assert result is not None\n");
}

#[test]
fn vacuous_fallback_emits_a_real_assertion_over_comment_only_body() {
    let mut body = "    # skipped: field 'chunks' not available on result type\n".to_string();
    apply_vacuous_assertion_fallback(&mut body, true, "result", false);
    assert!(
        body.contains("assert result is not None"),
        "a comment-only body must still get a real fallback assertion, got: {body}"
    );
}

#[test]
fn vacuous_fallback_leaves_a_real_assertion_untouched() {
    let mut body = "    assert result.count == 1\n".to_string();
    let original = body.clone();
    apply_vacuous_assertion_fallback(&mut body, true, "result", false);
    assert_eq!(
        body, original,
        "a fixture with a real assertion must not get an extra fallback line"
    );
}

/// Regression test for the void `not_error` defect: before this fix, a `returns_void`
/// fixture whose only declared assertion was `not_error` fell into the fallback and emitted
/// `assert result is not None` — but PyO3 maps a void call's `Ok(())` to Python `None`, so
/// that assertion FAILED on every successful call, not just an unsuccessful one.
#[test]
fn vacuous_fallback_emits_nothing_for_a_void_call() {
    let mut body = String::new();
    apply_vacuous_assertion_fallback(&mut body, true, "result", true);
    assert!(
        body.is_empty(),
        "a void call's result is always None; asserting not-None would fail every successful \
         call, got: {body}"
    );
}

/// Regression test for the not_error-only vacuous-test defect: a fixture whose
/// only declared assertion is `not_error` must bind the call result and emit a
/// real assertion, not silently discard the result with no assertion at all.
#[test]
fn not_error_only_fixture_binds_result_and_emits_real_assertion() {
    let mut fixture = minimal_fixture();
    fixture.assertions = vec![assertion("not_error", None, None)];
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "await widget_client.create()",
        "result",
        &field_resolver,
        false,
        false,
        false,
        false,
        None,
    );

    assert!(
        out.contains("result = await widget_client.create()"),
        "a not_error-only fixture must bind the result, got: {out}"
    );
    assert!(
        out.contains("assert result is not None"),
        "a not_error-only fixture must emit a real assertion instead of a vacuous body, got: {out}"
    );
}

/// Regression test for the void `not_error` defect: before this fix, a `returns_void`
/// fixture whose only declared assertion was `not_error` bound the result and asserted
/// `assert result is not None` — but PyO3 maps a void call's `Ok(())` to Python `None`, so
/// this assertion FAILED every successful call. The correct rendering is a bare, unbound
/// call statement: an uncaught exception already fails a pytest test on its own.
#[test]
fn void_not_error_fixture_emits_a_bare_unbound_call_not_a_guaranteed_failure() {
    let mut fixture = minimal_fixture();
    fixture.assertions = vec![assertion("not_error", None, None)];
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig {
        returns_void: true,
        ..Default::default()
    };
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "await widget_client.prefetch()",
        "result",
        &field_resolver,
        false,
        false,
        false,
        false,
        None,
    );

    assert!(
        !out.contains("assert result is not None"),
        "a void call's result is always None; asserting not-None would fail every successful \
         call, got: {out}"
    );
    assert!(
        out.contains("await widget_client.prefetch()") && !out.contains("result ="),
        "a void not_error-only fixture must emit a bare, unbound call statement, got: {out}"
    );
}

#[test]
#[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'chunks'")]
fn python_streaming_virtual_unsupported_type_fails_loudly() {
    let mut out = String::new();
    let assertion = assertion("bogus_type", Some("chunks"), None);
    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "chunks",
        "chunks",
        None,
        &crate::e2e::field_access::PythonTypedDictMap::default(),
    );
}

#[test]
fn python_streaming_virtual_supported_type_renders_assertion() {
    let mut out = String::new();
    let assertion = assertion("greater_than", Some("chunks"), Some(serde_json::Value::from(2)));
    emit_streaming_virtual_assertion(
        &mut out,
        &assertion,
        "chunks",
        "chunks",
        None,
        &crate::e2e::field_access::PythonTypedDictMap::default(),
    );
    assert_eq!(out.trim(), "assert chunks > 2");
}

/// Regression test for alef task #81, hole 3: the streaming branch of
/// `emit_result_and_assertions` used to render nothing at all — not even a
/// skip comment — for a fixture whose only declared assertion was a
/// non-streaming-virtual field. That left a vacuously-passing streaming test
/// with an entirely empty body. It must now get the same real fallback
/// assertion the non-streaming branch has always gotten.
#[test]
fn streaming_fixture_whose_only_assertion_is_non_virtual_gets_a_vacuous_fallback() {
    let mut fixture = minimal_fixture();
    fixture.assertions = vec![assertion(
        "equals",
        Some("not_a_streaming_field"),
        Some(serde_json::json!("x")),
    )];
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "chat_stream(request)",
        "result",
        &field_resolver,
        false,
        true,
        false,
        false,
        None,
    );

    assert!(
        out.contains("not_a_streaming_field' not available on streaming result type"),
        "the dropped field must still be named in a skip comment, got: {out}"
    );
    assert!(
        out.contains("assert chunks is not None"),
        "a streaming fixture with a declared but unusable assertion must still get a real \
         fallback assertion instead of an entirely empty body, got: {out}"
    );
}

/// Positive control for the same fix: a streaming fixture whose assertion IS a
/// real streaming-virtual field must render only the real assertion — no skip
/// comment, and the vacuous-fallback must not fire (a real assertion is present).
#[test]
fn streaming_fixture_with_a_real_streaming_assertion_is_not_touched_by_the_fallback() {
    let mut fixture = minimal_fixture();
    fixture.assertions = vec![assertion("count_min", Some("chunks"), Some(serde_json::json!(1)))];
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "chat_stream(request)",
        "result",
        &field_resolver,
        false,
        true,
        false,
        false,
        None,
    );

    assert!(out.contains("assert len(chunks) >= 1"), "got: {out}");
    assert!(
        !out.contains("not available"),
        "a real assertion must not trigger the fallback, got: {out}"
    );
}

/// Regression test for alef task #81: the non-streaming branch's "skipped: field
/// not available" comment must survive as the exact marker text the shared
/// `fail_on_unavailable_field_markers` mechanism (src/e2e/codegen/mod.rs) matches
/// on, so that arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns it into a
/// generation-time failure instead of a silently-passing comment. This test does
/// not set the env var (tests must stay independent of shared process state); the
/// arming behaviour itself is proven in `mod.rs`'s
/// `unavailable_field_marker_tests` against the same marker text asserted here.
#[test]
fn non_streaming_skip_comment_carries_the_marker_the_strict_mode_matches_on() {
    let mut fixture = minimal_fixture();
    fixture.assertions = vec![assertion(
        "equals",
        Some("nonexistent_field"),
        Some(serde_json::json!("x")),
    )];
    let e2e_config = E2eConfig::default();
    let call_config = crate::e2e::config::CallConfig::default();
    let result_fields: std::collections::HashSet<String> = ["content".to_string()].into_iter().collect();
    let field_resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &result_fields,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();

    emit_result_and_assertions(
        &mut out,
        &fixture,
        &e2e_config,
        &call_config,
        "widget_client.create()",
        "result",
        &field_resolver,
        false,
        false,
        false,
        false,
        None,
    );

    assert!(
        out.contains("field 'nonexistent_field' not available on result type"),
        "got: {out}"
    );
}
