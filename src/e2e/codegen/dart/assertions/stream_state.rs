//! Assertions on the state of a collected chat stream for a streaming Dart fixture.
//!
//! A stream result is the `List` of chunks collected from the stream. The fields `stream_complete`,
//! `no_chunks_after_done`, `finish_reason` and `tool_calls` are not members of a chunk, so each one
//! is derived from the whole list. A chunk is done when one of its choices carries a finish reason.

use std::fmt::Write as FmtWrite;

use crate::e2e::fixture::Assertion;

use super::super::escape_dart;

/// Every choice of every chunk, in stream order.
fn choices(result_var: &str) -> String {
    format!("{result_var}.expand((c) => c.choices)")
}

/// Every tool call of every delta, in stream order.
fn tool_calls(result_var: &str) -> String {
    format!("{}.expand((ch) => ch.delta.toolCalls ?? [])", choices(result_var))
}

/// The accessor for `tool_calls[N].function.<leaf>` as `(index, leaf)`, when `field` has that shape.
fn tool_call_leaf(field: &str) -> Option<(usize, &'static str)> {
    let rest = field.strip_prefix("tool_calls[")?;
    let (index, rest) = rest.split_once(']')?;
    let leaf = match rest {
        ".function.name" => "function?.name",
        ".function.arguments" => "function?.arguments",
        ".id" => "id",
        _ => return None,
    };
    Some((index.parse().ok()?, leaf))
}

/// Render `assertion` when it names a stream-state field and has a shape the matcher can take.
///
/// Returns `false` for any other assertion, so the caller's registered skip line still applies to
/// an assertion this renderer does not implement.
pub(super) fn try_render(out: &mut String, assertion: &Assertion, result_var: &str) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let kind = assertion.assertion_type.as_str();
    match (field, kind) {
        ("stream_complete", "is_true" | "is_false") => {
            let matcher = if kind == "is_true" { "isTrue" } else { "isFalse" };
            let _ = writeln!(
                out,
                "    expect({}.any((ch) => ch.finishReason != null), {matcher});",
                choices(result_var)
            );
        }
        // A chunk cannot arrive after the stream is collected, so the check is on the content: no
        // chunk after the first done chunk carries text or tool calls.
        ("no_chunks_after_done", "is_true") => {
            let _ = writeln!(
                out,
                "    final _doneAt = {result_var}.indexWhere((c) => c.choices.any((ch) => ch.finishReason != null));"
            );
            let _ = writeln!(
                out,
                "    expect(_doneAt >= 0 && {result_var}.skip(_doneAt + 1).every((c) => c.choices.every((ch) => (ch.delta.content ?? '').isEmpty && (ch.delta.toolCalls ?? []).isEmpty)), isTrue);"
            );
        }
        ("finish_reason", "equals") => {
            let Some(serde_json::Value::String(expected)) = &assertion.value else {
                return false;
            };
            let _ = writeln!(
                out,
                "    expect({}.map((ch) => ch.finishReason?.wireValue).nonNulls.lastOrNull, equals('{}'));",
                choices(result_var),
                escape_dart(expected)
            );
        }
        ("tool_calls", "not_empty") => {
            let _ = writeln!(out, "    expect({}, isNotEmpty);", tool_calls(result_var));
        }
        (_, "equals") => {
            let (Some((index, leaf)), Some(serde_json::Value::String(expected))) =
                (tool_call_leaf(field), &assertion.value)
            else {
                return false;
            };
            let _ = writeln!(
                out,
                "    expect({}.elementAtOrNull({index})?.{leaf}, equals('{}'));",
                tool_calls(result_var),
                escape_dart(expected)
            );
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(kind: &str, field: &str, value: Option<serde_json::Value>) -> Option<String> {
        let assertion = Assertion {
            assertion_type: kind.to_string(),
            field: Some(field.to_string()),
            value,
            ..Assertion::default()
        };
        let mut out = String::new();
        try_render(&mut out, &assertion, "result").then_some(out)
    }

    #[test]
    fn stream_complete_asks_for_a_chunk_with_a_finish_reason() {
        assert_eq!(
            render("is_true", "stream_complete", None).as_deref(),
            Some("    expect(result.expand((c) => c.choices).any((ch) => ch.finishReason != null), isTrue);\n")
        );
    }

    #[test]
    fn no_chunks_after_done_rejects_content_after_the_done_chunk() {
        let out = render("is_true", "no_chunks_after_done", None).expect("rendered");
        assert!(out.contains("final _doneAt = result.indexWhere"), "got:\n{out}");
        assert!(out.contains("result.skip(_doneAt + 1).every"), "got:\n{out}");
    }

    #[test]
    fn finish_reason_compares_the_wire_value() {
        let out = render("equals", "finish_reason", Some(serde_json::json!("tool_calls"))).expect("rendered");
        assert!(
            out.contains("ch.finishReason?.wireValue).nonNulls.lastOrNull, equals('tool_calls')"),
            "got:\n{out}"
        );
    }

    #[test]
    fn tool_calls_are_flattened_across_chunks() {
        let out = render("not_empty", "tool_calls", None).expect("rendered");
        assert!(out.contains("expand((ch) => ch.delta.toolCalls ?? [])"), "got:\n{out}");
        let named = render(
            "equals",
            "tool_calls[0].function.name",
            Some(serde_json::json!("get_weather")),
        )
        .expect("rendered");
        assert!(
            named.contains(".elementAtOrNull(0)?.function?.name, equals('get_weather')"),
            "got:\n{named}"
        );
    }

    /// The streaming entry point routes a stream-state assertion here, so the emitted lines are
    /// real `expect` statements, not the registered skip comment the arm fell to before.
    #[test]
    fn the_streaming_entry_point_asserts_the_stream_state() {
        let cases = [
            ("is_true", "stream_complete", None, "finishReason != null"),
            (
                "is_true",
                "no_chunks_after_done",
                None,
                "final _doneAt = result.indexWhere",
            ),
            (
                "equals",
                "finish_reason",
                Some(serde_json::json!("stop")),
                "ch.finishReason?.wireValue",
            ),
            (
                "not_empty",
                "tool_calls",
                None,
                "expand((ch) => ch.delta.toolCalls ?? [])",
            ),
        ];
        for (kind, field, value, expected) in cases {
            let assertion = Assertion {
                assertion_type: kind.to_string(),
                field: Some(field.to_string()),
                value,
                ..Assertion::default()
            };
            let mut out = String::new();
            super::super::render_streaming_assertion_dart(&mut out, &assertion, "result");
            assert!(
                out.contains(expected),
                "{field} must render {expected}. Emitted:\n{out}"
            );
            assert!(!out.contains("skipped"), "{field} must not be skipped. Emitted:\n{out}");
        }
    }

    #[test]
    fn an_unrelated_assertion_is_left_to_the_caller() {
        assert_eq!(render("equals", "finish_reason", Some(serde_json::json!(3))), None);
        assert_eq!(render("contains", "tool_calls", None), None);
    }
}
