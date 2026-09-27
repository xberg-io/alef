//! Issue #441: the streaming iterator's forwarding goroutine has three paths that used to
//! return silently, discarding the reason the stream ended -- a null chunk (which the native
//! layer overloads to mean either a clean end-of-stream *or* a stream error), a `to_json` call
//! that returned null, and a failed JSON unmarshal. These tests pin the fix: a
//! `<Recv><Method>Stream` iterator struct with `Chan()`/`Err()` accessors, whose goroutine
//! assigns `stream.err` on each of the three paths instead of dropping it, and reads
//! `lastError()` on the null-chunk path while the per-item `runtime.LockOSThread()` from #439
//! is still held (the native last-error slot is per-OS-thread, so reading it after
//! `UnlockOSThread` can observe a different thread's error). ~keep

use super::gen_streaming_method_wrapper;
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        is_opaque: true,
        ..TypeDef::default()
    }
}

fn streaming_method(name: &str, params: Vec<ParamDef>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type: TypeRef::Unit,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

/// Return the exact text of the `if <prefix> { ... }` block that starts with `if_prefix`
/// (e.g. `"if chunkPtr == 0 {"`), matched by brace depth rather than by searching for the next
/// occurrence of some marker -- a plain `source[pos..]` tail is NOT scoped to one branch and
/// silently matches content from a later branch instead, which is exactly the vacuous-check
/// shape this helper exists to rule out. ~keep
fn extract_if_block<'a>(source: &'a str, if_prefix: &str) -> &'a str {
    let start = source
        .find(if_prefix)
        .unwrap_or_else(|| panic!("`{if_prefix}` not found in:\n{source}"));
    let brace_start = start + source[start..].find('{').expect("an `if` statement must open a brace");
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut idx = brace_start;
    while idx < bytes.len() {
        match bytes[idx] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=idx];
                }
            }
            _ => {}
        }
        idx += 1;
    }
    panic!("unbalanced braces scanning the `{if_prefix}` block in:\n{source}");
}

/// Render `Engine.crawl_stream` (item type `CrawlEvent`, no sum-type unmarshal branch) with
/// empty helper sets -- the minimal shape every test in this file starts from.
fn render_crawl_stream() -> String {
    let typ = opaque_type("Engine");
    let method = streaming_method("crawl_stream", Vec::new());
    let empty_str: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_string: std::collections::HashSet<String> = std::collections::HashSet::new();

    gen_streaming_method_wrapper(
        &typ,
        &method,
        "krz",
        "CrawlEvent",
        &empty_str,
        &empty_str,
        &empty_string,
        &empty_string,
        &empty_string,
    )
}

#[test]
fn stream_type_is_declared_with_recv_method_stream_naming_and_chan_err_accessors() {
    let out = render_crawl_stream();

    assert!(
        out.contains("type EngineCrawlStreamStream struct {"),
        "expected the `<Recv><Method>Stream` naming convention, got:\n{out}"
    );
    assert!(
        out.contains("ch  <-chan CrawlEvent"),
        "the struct must hold the item channel, got:\n{out}"
    );
    assert!(
        out.contains("err error"),
        "the struct must hold the terminal error, got:\n{out}"
    );
    assert!(
        out.contains("func (s *EngineCrawlStreamStream) Chan() <-chan CrawlEvent { return s.ch }"),
        "Chan() must return the item channel, got:\n{out}"
    );
    assert!(
        out.contains("func (s *EngineCrawlStreamStream) Err() error { return s.err }"),
        "Err() must return the terminal error, got:\n{out}"
    );
}

#[test]
fn method_signature_and_start_failure_return_a_stream_pointer_not_a_bare_channel() {
    let out = render_crawl_stream();

    assert!(
        out.contains("func (h *Engine) CrawlStream(ctx context.Context) (*EngineCrawlStreamStream, error) {"),
        "the outer signature must take a leading ctx context.Context (issue #448) and return \
         `*<Recv><Method>Stream, error`, got:\n{out}"
    );
    assert!(
        !out.contains("<-chan CrawlEvent, error"),
        "the old bare-channel return type must be gone entirely, got:\n{out}"
    );
    assert!(
        out.contains("stream := &EngineCrawlStreamStream{ch: ch}"),
        "the goroutine must be handed a stream value to report its error through, got:\n{out}"
    );
    assert!(
        out.contains("return stream, nil"),
        "a successfully started stream must return the iterator, not the raw channel, got:\n{out}"
    );
}

/// The defect: a null chunk means either a clean end-of-stream (native errno 0) or a stream
/// error (errno != 0), and the loop used to return without ever reading `lastError()`. The fix
/// reads it into `stream.err` -- and must do so before `runtime.UnlockOSThread()` releases the
/// per-item lock #439 added, since the native error slot is per-OS-thread.
#[test]
fn null_chunk_path_assigns_stream_err_from_last_error_while_still_locked() {
    let out = render_crawl_stream();
    let branch = extract_if_block(&out, "if chunkPtr == 0 {");

    let assign_pos = branch.find("stream.err = lastError()").unwrap_or_else(|| {
        panic!("the null-chunk path must assign stream.err from lastError(), got branch:\n{branch}")
    });
    let unlock_pos = branch[assign_pos..]
        .find("runtime.UnlockOSThread()")
        .map(|offset| assign_pos + offset)
        .unwrap_or_else(|| {
            panic!("the null-chunk path must still unlock after reading the error, got branch:\n{branch}")
        });
    let return_pos = branch[unlock_pos..]
        .find("return")
        .map(|offset| unlock_pos + offset)
        .unwrap_or_else(|| panic!("the null-chunk path must still return, got branch:\n{branch}"));

    assert!(
        assign_pos < unlock_pos && unlock_pos < return_pos,
        "expected lastError() to be read before the per-item unlock, which must precede the \
         return, got branch:\n{branch}"
    );
}

/// A dropped `to_json` failure (`jsonPtr == nil`) must also terminate the stream with a
/// reported error rather than a silent close.
#[test]
fn to_json_null_path_assigns_a_reported_stream_err() {
    let out = render_crawl_stream();
    let branch = extract_if_block(&out, "if jsonPtr == nil {");

    let assign_pos = branch
        .find("stream.err = fmt.Errorf(")
        .unwrap_or_else(|| panic!("the to_json-null path must assign a reported stream.err, got branch:\n{branch}"));
    let unlock_pos = branch[assign_pos..]
        .find("runtime.UnlockOSThread()")
        .map(|offset| assign_pos + offset)
        .unwrap_or_else(|| panic!("the to_json-null path must still unlock, got branch:\n{branch}"));

    assert!(
        assign_pos < unlock_pos,
        "expected the error to be assigned before unlocking, got branch:\n{branch}"
    );
}

/// A dropped unmarshal failure must also terminate the stream with a reported (and wrapped)
/// error rather than a silent close.
#[test]
fn unmarshal_error_path_wraps_and_assigns_a_reported_stream_err() {
    let out = render_crawl_stream();
    let branch = extract_if_block(&out, "if unmarshalErr != nil {");

    assert!(
        branch.contains("stream.err = fmt.Errorf(") && branch.contains("%w") && branch.contains("unmarshalErr"),
        "the unmarshal-error path must wrap unmarshalErr into stream.err, got:\n{branch}"
    );
}

/// Issue #448: an unbuffered `ch <- chunk` parks the forwarding goroutine forever when a
/// consumer breaks out of `range stream.Chan()` early, leaking the goroutine and the native
/// stream handle (the deferred `close(ch)`/`C.<fn_free>(handle)` never run). The fix races the
/// send against the caller's `ctx.Done()`.
#[test]
fn chunk_send_races_against_ctx_done_instead_of_blocking_forever() {
    let out = render_crawl_stream();

    assert!(
        !out.contains("\tch <- chunk\n"),
        "the bare unbuffered send must be gone, got:\n{out}"
    );
    let branch = extract_if_block(&out, "select {");
    assert!(
        branch.contains("case ch <- chunk:"),
        "the select must still deliver the chunk on the happy path, got:\n{branch}"
    );
    assert!(
        branch.contains("case <-ctx.Done():") && branch.contains("stream.err = ctx.Err()") && branch.contains("return"),
        "the select must let a cancelled ctx unblock the goroutine and report ctx.Err(), got:\n{branch}"
    );
}

#[test]
fn generated_stream_is_valid_go_syntax() {
    use std::io::Write as _;

    let out = render_crawl_stream();
    let Ok(mut child) = crate::test_support::spawn_from_stable_dir("gofmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    let source = format!("package sample\n\ntype Example struct {{}}\n\n{out}");
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(source.as_bytes())
        .expect("write generated Go source");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(
        output.status.success(),
        "generated Go syntax is invalid: {}\n{out}",
        String::from_utf8_lossy(&output.stderr)
    );
}
