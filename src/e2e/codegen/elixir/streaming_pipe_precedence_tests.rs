//! Regression coverage for the Elixir `|>` / `not in` precedence defect in streaming assertions.
//!
//! Elixir's operator table binds `in` / `not in` TIGHTER than `|>`. A streaming accessor that
//! led with a pipe therefore could not be substituted into `render_assertion`'s `not_empty` arm
//! (`assert <expr> not in [nil, "", [], %{}]`): the parser attached the `not in [...]` tail to
//! the pipe's right-hand side and rejected the whole line. Observed in a consumer's E2E elixir
//! job as:
//!
//! ```text
//! cannot pipe chunks into Enum.flat_map(...) not in [nil, "", [], %{}]
//! ```
//!
//! The fix is in `streaming_assertions::accessors`: every Elixir streaming accessor is emitted
//! as a primary expression (a single call), never a bare pipe chain, because call sites paste it
//! into operator contexts they do not parenthesize.
//!
//! Lives in its own file rather than growing `elixir/assertions.rs`, which is over the repo's
//! 1,000-line cap and may not grow (see `file-modularization` in CLAUDE.md). ~keep

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::process::Command;

use super::assertions::render_assertion;
use super::snippet::render_snippet_body;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::streaming_assertions::{STREAMING_VIRTUAL_FIELDS, StreamingFieldResolver};
use crate::e2e::config::{E2eConfig, StreamingConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

/// The exact `tool_calls` accessor the Elixir backend must emit.
const TOOL_CALLS_ACCESSOR: &str = "Enum.flat_map(chunks, fn c -> (Map.get((Map.get(List.first(c.choices) || %{}, :delta, %{}) || %{}), :tool_calls, []) || []) end)";

/// The exact `stream_content` accessor the Elixir backend must emit.
const STREAM_CONTENT_ACCESSOR: &str = "Enum.join(Enum.map(chunks, fn c -> \
     (Map.get((Map.get((Enum.at(c.choices, 0) || %{}), :delta, %{}) || %{}), :content, \"\") || \"\") end), \"\")";

/// The exact `stream_complete` accessor the Elixir backend must emit.
const STREAM_COMPLETE_ACCESSOR: &str = concat!(
    "(case List.last(chunks) do nil -> false; c -> case ",
    "List.first(Map.get(c, :choices, []) || []) do nil -> false; ",
    "choice -> Map.get(choice, :finish_reason) != nil end end)"
);

/// The pre-fix `stream_complete` accessor: a `case/do/end` emitted as a bare paren-less
/// expression. Pasted into `assert <expr>`, Elixir's `do` block binds to the OUTERMOST
/// paren-less call (`assert`) rather than `case`, reparsing the clause list as `assert`'s own
/// do-block and failing to compile with "misplaced operator ->". Kept verbatim so the scanner
/// below can be shown to reject the shape that actually broke a consumer build. ~keep
const DEFECTIVE_STREAM_COMPLETE_ACCESSOR: &str = concat!(
    "case List.last(chunks) do nil -> false; c -> case ",
    "List.first(Map.get(c, :choices, []) || []) do nil -> false; ",
    "choice -> Map.get(choice, :finish_reason) != nil end end"
);

/// The pre-fix `tool_calls` accessor, kept verbatim so the scanner below can be shown to reject
/// the shape that actually broke a consumer build rather than passing vacuously. ~keep
const DEFECTIVE_TOOL_CALLS_ACCESSOR: &str =
    "chunks |> Enum.flat_map(fn c -> ((List.first(c.choices) || %{}).delta |> Map.get(:tool_calls, [])) || [] end)";

/// Lower bound on how many Elixir streaming accessors the defect-class sweep must inspect: the
/// 13 entries of `STREAMING_VIRTUAL_FIELDS` plus `usage` and three `tool_calls` deep paths.
const MIN_SWEPT_ACCESSORS: usize = 17;

/// True when `expr` contains a `|>` outside every bracket group — the shape that cannot be
/// pasted in front of `not in [...]`. Pipes nested inside `(...)`, `[...]` or `%{...}` are
/// already delimited and parse correctly. String literals are skipped so a `"|>"` inside a
/// generated Elixir string is not mistaken for an operator. ~keep
fn has_top_level_pipe(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'|' if depth == 0 && bytes.get(index + 1) == Some(&b'>') => return true,
            _ => {}
        }
        index += 1;
    }
    false
}

/// True when `expr` contains a keyword `do` block (`case`/`fn`/`if`/`cond`/... `do ... end`)
/// outside every bracket group — the shape that cannot be pasted into `assert <expr>` or
/// `<expr> not in [...]`. Elixir's `do` block binds to the OUTERMOST paren-less call in its
/// statement, so a bare `case ... do ... end` substituted into `assert <expr>` reparses as
/// `assert(case(...), do: [...])`: the clause list becomes `assert`'s own do-block and the
/// generated file fails to compile with "misplaced operator ->". A `do` nested inside `(...)`,
/// `[...]` or `%{...}` is already a self-contained primary expression and parses correctly, so
/// only a depth-0 `do` is flagged. String literals are skipped so a `"do"` inside a generated
/// Elixir string is not mistaken for the keyword. ~keep
fn has_top_level_do_block(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'd' if depth == 0 && bytes[index..].starts_with(b"do") => {
                let is_identifier_byte = |b: u8| (b as char).is_alphanumeric() || b == b'_';
                let before_is_boundary = index == 0 || !is_identifier_byte(bytes[index - 1]);
                let after_is_boundary = bytes.get(index + 2).is_none_or(|&b| !is_identifier_byte(b));
                if before_is_boundary && after_is_boundary {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

fn streaming_assertion(assertion_type: &str, field: &str) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// Renders one assertion the way `test_case.rs` renders a streaming fixture: the collected list
/// is bound to `chunks`, so the accessor is built over `chunks`, exactly as in the failing job.
fn render_streaming_assertion(assertion_type: &str, field: &str) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &streaming_assertion(assertion_type, field),
        "chunks",
        &resolver,
        "Sample",
        &HashSet::new(),
        &HashMap::new(),
        false,
        true,
        false,
        false,
    );
    out
}

fn run_elixir(program: &str, script: &str, required: bool) -> Option<std::process::Output> {
    match Command::new(program).args(["-e", script]).output() {
        Ok(output) => Some(output),
        Err(error) if error.kind() == ErrorKind::NotFound && required => {
            panic!("ALEF_REQUIRE_ELIXIR is set but Elixir is unavailable: {error}")
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => panic!("failed to execute installed Elixir runtime: {error}"),
    }
}

/// Pins the accessor character-for-character. Restoring the pre-fix
/// `chunks |> Enum.flat_map(fn c -> ... end)` fails on the leading `chunks |> `.
#[test]
fn elixir_tool_calls_accessor_is_a_plain_call_not_a_pipe_chain() {
    let expr = StreamingFieldResolver::accessor("tool_calls", "elixir", "chunks").expect("elixir tool_calls accessor");
    assert_eq!(expr, TOOL_CALLS_ACCESSOR, "got: {expr}");
}

/// Same pin for `stream_content`. Restoring
/// `chunks |> Enum.map(...) |> Enum.join("")` fails on the leading `chunks |> `.
#[test]
fn elixir_stream_content_accessor_is_a_plain_call_not_a_pipe_chain() {
    let expr =
        StreamingFieldResolver::accessor("stream_content", "elixir", "chunks").expect("elixir stream_content accessor");
    assert_eq!(expr, STREAM_CONTENT_ACCESSOR, "got: {expr}");
}

/// The line that actually failed to compile in the consumer's E2E elixir job, pinned whole.
/// Reverting the accessor renders
/// `assert chunks |> Enum.flat_map(...) not in [nil, "", [], %{}]`, which Elixir rejects with
/// "cannot pipe chunks into Enum.flat_map(...) not in [nil, \"\", [], %{}]".
#[test]
fn not_empty_on_tool_calls_emits_a_parsable_membership_test() {
    let out = render_streaming_assertion("not_empty", "tool_calls");
    assert_eq!(
        out,
        format!("      assert {TOOL_CALLS_ACCESSOR} not in [nil, \"\", [], %{{}}]\n"),
        "got: {out}"
    );
}

/// Executes the emitted accessor in a real Elixir runtime. A usage-only terminal chunk may have
/// no choices, and decoded input may omit or explicitly null the delta/tool-call fields; all are
/// empty contributors rather than reasons for the generated assertion suite to crash. ~keep
#[test]
fn tool_calls_accessor_handles_sparse_chunks_in_elixir_runtime() {
    let accessor =
        StreamingFieldResolver::accessor("tool_calls", "elixir", "chunks").expect("elixir tool_calls accessor");
    let script = format!(
        r#"
chunks = [
  %{{choices: []}},
  %{{choices: [%{{}}]}},
  %{{choices: [%{{delta: nil}}]}},
  %{{choices: [%{{delta: %{{tool_calls: nil}}}}]}},
  %{{choices: [%{{delta: %{{tool_calls: [%{{id: "call-1"}}]}}}}]}}
]

actual = {accessor}

unless actual == [%{{id: "call-1"}}] do
  raise "unexpected tool calls: #{{inspect(actual)}}"
end
"#,
    );

    let Some(output) = run_elixir("elixir", &script, std::env::var_os("ALEF_REQUIRE_ELIXIR").is_some()) else {
        return;
    };

    assert!(
        output.status.success(),
        "generated accessor failed in Elixir runtime\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Content aggregation has the same sparse input domain as tool-call aggregation. In particular,
/// `Map.get/3` does not replace an explicitly stored nil delta, so the delta must be normalized
/// before the content lookup. ~keep
#[test]
fn stream_content_accessor_handles_sparse_chunks_in_elixir_runtime() {
    let accessor =
        StreamingFieldResolver::accessor("stream_content", "elixir", "chunks").expect("elixir content accessor");
    let script = format!(
        r#"
chunks = [
  %{{choices: []}},
  %{{choices: [%{{}}]}},
  %{{choices: [%{{delta: nil}}]}},
  %{{choices: [%{{delta: %{{content: nil}}}}]}},
  %{{choices: [%{{delta: %{{content: "hello"}}}}]}}
]

unless {accessor} == "hello" do
  raise "unexpected content"
end
"#,
    );
    let Some(output) = run_elixir("elixir", &script, std::env::var_os("ALEF_REQUIRE_ELIXIR").is_some()) else {
        return;
    };

    assert!(
        output.status.success(),
        "generated content accessor failed in Elixir runtime\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Completion is false, not an exception, when the stream is empty or its terminal chunk has no
/// choice. A choice with a finish reason remains the positive control. ~keep
#[test]
fn stream_complete_accessor_handles_empty_and_usage_only_streams_in_elixir_runtime() {
    let accessor =
        StreamingFieldResolver::accessor("stream_complete", "elixir", "chunks").expect("elixir completion accessor");
    assert_eq!(accessor, STREAM_COMPLETE_ACCESSOR);
    let script = format!(
        r#"
completion = fn chunks -> {accessor} end

unless completion.([]) == false do
  raise "empty stream reported complete"
end

unless completion.([%{{choices: []}}, %{{choices: [], usage: %{{total_tokens: 1}}}}]) == false do
  raise "usage-only stream reported complete"
end

unless completion.([%{{choices: [%{{finish_reason: "stop"}}]}}]) == true do
  raise "finished stream reported incomplete"
end
"#,
    );
    let Some(output) = run_elixir("elixir", &script, std::env::var_os("ALEF_REQUIRE_ELIXIR").is_some()) else {
        return;
    };

    assert!(
        output.status.success(),
        "generated completion accessor failed in Elixir runtime\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Proves required-toolchain mode cannot turn a missing executable into a green runtime test.
#[test]
fn required_elixir_mode_fails_when_runtime_is_unavailable() {
    let result = std::panic::catch_unwind(|| run_elixir("alef-elixir-does-not-exist", "", true));
    let panic = result.expect_err("required mode must fail when Elixir is unavailable");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(message.contains("ALEF_REQUIRE_ELIXIR is set"), "got: {message}");
}

/// Keeps the required mode wired to a job that actually installs Elixir. Checking both needles
/// prevents either half from drifting into a vacuous green runtime test. ~keep
#[test]
fn ci_installs_and_requires_elixir_for_runtime_regressions() {
    let workflow = include_str!("../../../../.github/workflows/ci.yml");
    assert!(workflow.contains("uses: xberg-io/actions/setup-elixir@v1"));
    assert!(workflow.contains("ALEF_REQUIRE_ELIXIR: \"1\""));
}

/// The same latent break on the other pipe-headed accessor: `not_empty` on `stream_content` was
/// never exercised by the consumer, but produced the identical unparsable shape.
#[test]
fn not_empty_on_stream_content_emits_a_parsable_membership_test() {
    let out = render_streaming_assertion("not_empty", "stream_content");
    assert_eq!(
        out,
        format!("      assert {STREAM_CONTENT_ACCESSOR} not in [nil, \"\", [], %{{}}]\n"),
        "got: {out}"
    );
}

/// Defect-class sweep: NO Elixir streaming accessor may lead with a pipe or a bare `do` block,
/// not just the instances that actually broke a consumer build. A future accessor written as
/// `chunks |> ...` or `case ... do ... end` fails here even if nothing pastes it into an
/// operator or paren-less context yet.
#[test]
fn no_elixir_streaming_accessor_leads_with_a_top_level_pipe() {
    let mut fields: Vec<&str> = STREAMING_VIRTUAL_FIELDS.to_vec();
    fields.extend([
        "usage",
        "tool_calls[0]",
        "tool_calls[0].id",
        "tool_calls[0].function.name",
    ]);

    let resolved: Vec<(&str, String)> = fields
        .iter()
        .filter_map(|field| {
            StreamingFieldResolver::accessor_with_streaming_context(
                field,
                "elixir",
                "chunks",
                None,
                Some("StreamEvent"),
            )
            .map(|expr| (*field, expr))
        })
        .collect();

    // ~keep Read the count, not the pass: an accessor that silently resolved to `None` would be
    // swept over without ever being inspected, and the loop below would still report green.
    let unresolved: Vec<&&str> = fields
        .iter()
        .filter(|field| !resolved.iter().any(|(name, _)| name == *field))
        .collect();
    assert!(
        unresolved.is_empty(),
        "elixir accessors resolved to None: {unresolved:?}"
    );
    assert!(
        resolved.len() >= MIN_SWEPT_ACCESSORS,
        "sweep shrank to {} accessors",
        resolved.len()
    );

    for (field, expr) in &resolved {
        assert!(
            !has_top_level_pipe(expr),
            "elixir accessor for '{field}' leads with a pipe and cannot be pasted before \
             `not in [...]`: {expr}"
        );
        assert!(
            !has_top_level_do_block(expr),
            "elixir accessor for '{field}' leads with a bare `do` block and cannot be pasted \
             into `assert <expr>` -- Elixir's `do` would bind to the outermost paren-less call \
             instead of this accessor's own keyword construct: {expr}"
        );
    }
}

/// Proves the sweep above is wired to the real defect instead of passing on everything: the
/// verbatim pre-fix accessor must be flagged, and the shipped one must not. ~keep
#[test]
fn the_top_level_pipe_scanner_flags_the_shape_that_broke_the_build() {
    assert!(
        has_top_level_pipe(DEFECTIVE_TOOL_CALLS_ACCESSOR),
        "scanner missed the pre-fix accessor"
    );
    assert!(!has_top_level_pipe(TOOL_CALLS_ACCESSOR));
    assert!(!has_top_level_pipe(STREAM_CONTENT_ACCESSOR));
    // A pipe nested inside a bracket group is delimited and legal — the scanner must not
    // report it, or "no top-level pipe" would degrade into "no pipe anywhere". ~keep
    assert!(!has_top_level_pipe("Enum.join(a |> Enum.map(f), \"\")"));
    assert!(!has_top_level_pipe("Enum.map(c, fn x -> x |> to_string() end)"));
}

/// Same wiring proof for the `do`-block scanner: the verbatim pre-fix `stream_complete`
/// accessor (a bare `case ... do ... end`) must be flagged, and the shipped, parenthesized one
/// must not. ~keep
#[test]
fn the_top_level_do_block_scanner_flags_the_shape_that_broke_the_build() {
    assert!(
        has_top_level_do_block(DEFECTIVE_STREAM_COMPLETE_ACCESSOR),
        "scanner missed the pre-fix accessor"
    );
    assert!(!has_top_level_do_block(STREAM_COMPLETE_ACCESSOR));
    assert!(!has_top_level_do_block(TOOL_CALLS_ACCESSOR));
    assert!(!has_top_level_do_block(STREAM_CONTENT_ACCESSOR));
    // A `do` nested inside a bracket group (e.g. an anonymous function literal passed as an
    // argument) is delimited and legal — the scanner must not report it. ~keep
    assert!(!has_top_level_do_block("Enum.map(c, fn x -> case x do y -> y end end)"));
    // A `do` that is merely a substring of a longer identifier (e.g. `undo`) must not trip the
    // word-boundary check.
    assert!(!has_top_level_do_block("undo(chunks)"));
}

/// Control: a pipe in STATEMENT position is valid Elixir and must survive untouched. A "fix"
/// that removed pipes from the Elixir emitters wholesale would pass every test above and fail
/// this one.
#[test]
fn statement_position_pipe_in_the_streaming_snippet_is_unchanged() {
    let fixture = Fixture {
        id: "sample_stream".into(),
        description: "Sample stream".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "stream_items".into();
    e2e.call.module = "sample".into();
    e2e.call.result_var = "stream_result".into();
    e2e.call.streaming = Some(StreamingConfig::Enabled(true));

    let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

    assert!(
        body.contains("stream_result = Sample.stream_items() |> Enum.to_list()"),
        "statement-position pipe must be preserved verbatim, got:\n{body}"
    );
}
