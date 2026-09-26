//! Streaming collect-loop + error-check rendering for the Go e2e generator.
//!
//! Split out of `test_function.rs`, which is over the 1000-line cap and may not grow.

use std::fmt::Write as FmtWrite;

/// Render the `for chunk := range stream.Chan() { ... }` collect loop and the `stream.Err()`
/// check that must follow it.
///
/// A mid-stream failure must fail the generated test through `Err()`, not close the channel
/// silently -- see #441 (the null-chunk ambiguity the `<Recv><Method>Stream` iterator shape
/// fixes). When `streaming_item_type` is `None`, no adapter resolved an item type for this
/// fixture, so the collect loop cannot be typed and a skip marker is emitted instead.
pub(super) fn render_streaming_collect_loop(out: &mut String, import_alias: &str, streaming_item_type: Option<&str>) {
    let Some(streaming_item_type) = streaming_item_type else {
        let _ = writeln!(
            out,
            "\t// skipped: streaming fixture requires adapter item_type for Go e2e codegen"
        );
        return;
    };
    let _ = writeln!(out, "\tvar chunks []{import_alias}.{streaming_item_type}");
    let _ = writeln!(out, "\tfor chunk := range stream.Chan() {{");
    let _ = writeln!(out, "\t\tchunks = append(chunks, chunk)");
    let _ = writeln!(out, "\t}}");
    let _ = writeln!(out, "\tif streamErr := stream.Err(); streamErr != nil {{");
    let _ = writeln!(out, "\t\tt.Fatalf(\"stream failed: %v\", streamErr)");
    let _ = writeln!(out, "\t}}");
}
