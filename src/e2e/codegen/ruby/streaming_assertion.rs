//! Ruby chat-stream assertion rendering, split out of `ruby/examples.rs`.
//!
//! ~keep `examples.rs` is already over the repo's 1,000-line cap (see `file-modularization` in
//! CLAUDE.md) and sits at its recorded ratchet ceiling. The narrowing-guard fix below (every
//! `if let Some(...) = ...` arm gaining an `else` that renders the registered skip wording
//! instead of nothing) adds real behavior, not padding, so the touched concern moves to its own
//! file rather than pushing `examples.rs` past its ceiling.

use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Assertion;

use super::values::json_to_ruby;

/// Map a streaming fixture assertion to an `expect` call on the local aggregator
/// variable produced by [`super::examples::render_chat_stream_example`]. Pseudo-fields like
/// `chunks` / `stream_content` / `stream_complete` resolve to the in-block locals,
/// not response accessors.
pub(super) fn emit_chat_stream_assertion(
    out: &mut String,
    assertion: &Assertion,
    _e2e_config: &E2eConfig,
    streaming_item_type: Option<&str>,
    stream_complete_is_derived: bool,
) {
    // `mock.*` request-count virtual fields (alef issue #443) resolve against the mock
    // server's own request log, never the streaming aggregator locals this function
    // otherwise renders against -- intercept before any of that streaming-specific
    // dispatch. This function is Ruby's ONLY assertion path for a streaming fixture
    // (`examples::render_chat_stream_example` never calls `assertions::render_assertion`),
    // so a `mock.*` assertion on a streaming Ruby fixture reaches this interception
    // instead of `assertions.rs`'s own copy, which only ever sees non-streaming fixtures.
    if super::assertion_mock_capture::try_render_mock_capture_assertion(out, assertion) {
        return;
    }

    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }
    let field = assertion.field.as_deref().unwrap_or("");

    // Ruby drives the stream with a block, so by the time the call returns there is no iterator
    // left to ask for one more element -- the only way to observe a chunk arriving after done.
    // `csharp/streaming.rs` can probe its enumerator and does; ruby cannot, and the previous
    // mapping papered over that by aliasing this field onto the `stream_complete` local, so two
    // different assertions rendered one identical check that could not fail either way. ~keep
    if field == "no_chunks_after_done" {
        out.push_str(&format!(
            "    # skipped: {}; a block-driven ruby stream exposes no post-completion probe\n",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        ));
        return;
    }
    if field == "stream_complete" && !stream_complete_is_derived {
        out.push_str(&format!(
            "    # skipped: {}; this stream's chunks carry no terminal finish_reason, \
so completion is not observable here\n",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        ));
        return;
    }

    enum Kind {
        Chunks,
        Bool,
        Str,
        IntTokens,
        Json,
        Unsupported,
    }

    // Use StreamingFieldResolver to compute field expressions from chunks.
    let expr_opt = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
        field,
        "ruby",
        "chunks",
        None,
        streaming_item_type,
    );

    let (expr, kind) = match (field, expr_opt) {
        ("chunks", Some(expr)) => (expr, Kind::Chunks),
        ("chunks.length", Some(expr)) => (expr, Kind::Chunks),
        ("stream_content", Some(expr)) => (expr, Kind::Str),
        ("finish_reason", Some(expr)) => (expr, Kind::Str),
        ("tool_calls", Some(expr)) => (expr, Kind::Json),
        ("tool_calls[0].function.name", Some(expr)) => (expr, Kind::Str),
        ("usage.total_tokens", Some(expr)) => (expr, Kind::IntTokens),
        // Match on the field alone: the resolver answers `Some` here for every language, so a
        // `None` pattern would be unreachable and would silently drop the assertion into the
        // `Unsupported` arm. The spec body binds `stream_complete` to this very resolver
        // expression, so asserting the local and asserting the accessor are the same check;
        // `no_chunks_after_done` is refused above rather than aliased onto it. ~keep
        ("stream_complete", _) => ("stream_complete".to_string(), Kind::Bool),
        _ => ("".to_string(), Kind::Unsupported),
    };

    if matches!(kind, Kind::Unsupported) {
        out.push_str(&format!(
            "    # skipped: {}\n",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
        ));
        return;
    }

    // ~keep Every `if let Some(...) = ...` guard below used to have no `else`: a fixture value
    // that did not narrow to the expected shape (`as_u64()`, or bare presence for `equals`/
    // `contains`, which accept any JSON value) rendered NOTHING -- no assertion, no skip comment.
    // The catch-all default also used to emit ad hoc text
    // ("streaming assertion '<t>' on field '<f>' not supported") matching neither `FieldSkip`'s
    // nor `AssertionTypeSkip`'s registered wording, invisible to the strict gate even though a
    // line was present. All of these now route through the same funnel every other backend's
    // streaming renderer already uses.
    match (atype, &kind) {
        ("count_min", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}.length).to be >= {n}\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        ("count_equals", Kind::Chunks) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}.length).to eq({n})\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        ("equals", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                // Mirror Python's `expr.strip() == expected.strip()` pattern: converters
                // commonly emit a trailing newline that fixture authors don't write into the
                // expected string, so strip both sides for the equality check.
                out.push_str(&format!("    expect({expr}.to_s.strip).to eq({rb_val}.strip)\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        ("contains", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                out.push_str(&format!("    expect({expr}.to_s).to include({rb_val})\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        ("not_empty", Kind::Str) => {
            out.push_str(&format!("    expect({expr}.to_s).not_to be_empty\n"));
        }
        ("not_empty", Kind::Json) => {
            out.push_str(&format!("    expect({expr}).not_to be_nil\n"));
        }
        ("is_empty", Kind::Str) => {
            out.push_str(&format!("    expect({expr}.to_s).to be_empty\n"));
        }
        ("is_true", Kind::Bool) => {
            out.push_str(&format!("    expect({expr}).to be(true)\n"));
        }
        ("is_false", Kind::Bool) => {
            out.push_str(&format!("    expect({expr}).to be(false)\n"));
        }
        ("greater_than_or_equal", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}).to be >= {n}\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        ("equals", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                out.push_str(&format!("    expect({expr}).to eq({n})\n"));
            } else {
                out.push_str(&format!(
                    "{}\n",
                    streaming_assertion_value_skip_line("    ", "#", field, atype)
                ));
            }
        }
        _ => {
            out.push_str(&format!(
                "{}\n",
                streaming_assertion_type_skip_line("    ", "#", field, atype)
            ));
        }
    }
}

#[cfg(test)]
mod emit_chat_stream_assertion_tests {
    use super::emit_chat_stream_assertion;
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
    use crate::e2e::config::E2eConfig;
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
        emit_chat_stream_assertion(&mut out, &assertion, &E2eConfig::default(), None, false);
        assert_eq!(
            out, "    # skipped: assertion type 'count_min' has no renderable value for streaming field 'chunks'\n",
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
        emit_chat_stream_assertion(&mut out, &assertion, &E2eConfig::default(), None, false);
        assert_eq!(
            out,
            "    # skipped: assertion type 'equals' has no renderable value for streaming field 'stream_content'\n",
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
    /// back to the old ad hoc text.
    #[test]
    fn unsupported_assertion_type_on_a_supported_field_is_recognised_by_the_funnel() {
        let assertion = Assertion {
            assertion_type: "matches_regex".into(),
            field: Some("chunks".into()),
            ..Assertion::default()
        };
        let mut out = String::new();
        emit_chat_stream_assertion(&mut out, &assertion, &E2eConfig::default(), None, false);
        assert_eq!(
            out, "    # skipped: assertion type 'matches_regex' on field 'chunks' not yet supported for streaming\n",
            "got: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(&out),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "the rendered line must round-trip through the assertion-type funnel, got: {out}"
        );
    }

    /// A matched, well-formed assertion must still render a real `expect(...)`, not a skip
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
        emit_chat_stream_assertion(&mut out, &assertion, &E2eConfig::default(), None, false);
        assert_eq!(out, "    expect(chunks.length).to be >= 2\n", "got: {out}");
    }
}
