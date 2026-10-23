//! Streaming virtual-field assertion rendering for the Go e2e generator.
//!
//! A streaming fixture's assertions can target virtual fields (`finish_reason`,
//! `stream.has_page_event`, ...) that only resolve through the streaming accessor, never
//! through `FieldResolver`. This must be checked -- and rendered, or skip-marked -- before
//! any ordinary field resolution runs, so it must run ahead of the wildcard/availability
//! router and the `ResolvedAssertionTarget` derivation.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::escape::go_string_literal;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::AssertionRenderContext;

/// Render an assertion whose field names a streaming virtual field.
///
/// Returns `true` when the assertion targets a streaming virtual field (and the
/// assertion, or its skip-marker fallback, has been written to `out`); `false` when the
/// caller must fall through to ordinary field resolution.
pub(super) fn render_streaming_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    context: &AssertionRenderContext<'_>,
) -> bool {
    let Some(f) = assertion.field.as_deref() else {
        return false;
    };
    if context.result_is_simple
        || !context.is_streaming
        || f.is_empty()
        || !crate::e2e::codegen::streaming_assertions::is_streaming_call_field(f)
    {
        return false;
    }
    let streaming_item_type = context.streaming_item_type;

    if let Some(expr) =
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
            f,
            "go",
            "chunks",
            None,
            streaming_item_type,
        )
    {
        // ~keep The value-narrowing arms below used to fall through to nothing when the
        // fixture's value did not survive `as_u64()` / the string pattern, so the assertion
        // disappeared with no line for any funnel to count.
        let value_skip = || streaming_assertion_value_skip_line("\t", "//", f, &assertion.assertion_type);
        match assertion.assertion_type.as_str() {
            "count_min" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let _ = writeln!(
                        out,
                        "\tassert.GreaterOrEqual(t, len({expr}), {n}, \"expected >= {n} chunks\")"
                    );
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            "count_equals" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let _ = writeln!(
                        out,
                        "\tassert.Equal(t, {n}, len({expr}), \"expected exactly {n} chunks\")"
                    );
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            "equals" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = go_string_literal(s);
                    let is_deep_path = f.contains('.') || f.contains('[');
                    let safe_expr = if is_deep_path {
                        format!("func() string {{ v := {expr}; if v == nil {{ return \"\" }}; return *v }}()")
                    } else {
                        expr.clone()
                    };
                    let _ = writeln!(out, "\tassert.Equal(t, {escaped}, {safe_expr})");
                } else if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let _ = writeln!(out, "\tassert.Equal(t, {n}, {expr})");
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            "not_empty" => {
                let _ = writeln!(out, "\tassert.NotEmpty(t, {expr}, \"expected non-empty\")");
            }
            "is_empty" => {
                let _ = writeln!(out, "\tassert.Empty(t, {expr}, \"expected empty\")");
            }
            "is_true" => {
                let _ = writeln!(out, "\tassert.True(t, {expr}, \"expected true\")");
            }
            "is_false" => {
                let _ = writeln!(out, "\tassert.False(t, {expr}, \"expected false\")");
            }
            "greater_than" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let _ = writeln!(out, "\tassert.Greater(t, {expr}, {n}, \"expected > {n}\")");
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let _ = writeln!(out, "\tassert.GreaterOrEqual(t, {expr}, {n}, \"expected >= {n}\")");
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            "contains" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = crate::e2e::escape::go_string_literal(s);
                    let _ = writeln!(out, "\tassert.Contains(t, {expr}, {escaped}, \"expected to contain\")");
                } else {
                    let _ = writeln!(out, "{}", value_skip());
                }
            }
            _ => {
                let _ = writeln!(
                    out,
                    "{}",
                    streaming_assertion_type_skip_line("\t", "//", f, &assertion.assertion_type)
                );
            }
        }
    } else {
        // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
        // predicate with no resolved item type, for one), and this branch used to be absent:
        // the assertion vanished with no line for `fail_on_unavailable_field_markers` to see,
        // so a clean strict-gate run was indistinguishable from one that dropped it. alef's
        // own streaming adapter owns the gap, so it is counted, never fatal.
        let _ = writeln!(
            out,
            "\t// skipped: {}",
            FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
        );
    }

    true
}
