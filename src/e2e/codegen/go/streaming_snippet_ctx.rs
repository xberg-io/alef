//! Split out of `snippet.rs`, which is at the 1,000-line remediation cap and may not grow: the
//! doc-snippet call-expression logic issue #448 needs lives here instead.
//!
//! #448 gives every generated Go streaming start method a leading `ctx context.Context`
//! parameter (Go convention), so the documented call in a rendered doc snippet must pass one
//! too, and the snippet's `import (...)` block must pull in `"context"` when it does.

use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Fixture;

/// Build the doc-snippet call expression, prefixing `context.Background()` when `call` resolves
/// to a streaming fixture -- mirroring the leading `ctx context.Context` parameter
/// `gen_streaming_method_wrapper` (`backends/go/gen_bindings/methods.rs`) now always emits.
/// Returns the call expression alongside whether it is streaming, so the caller can also decide
/// whether to import `"context"` without re-deriving the same check.
pub(super) fn streaming_call_expr_and_flag(
    fixture: &Fixture,
    call: &CallConfig,
    call_prefix: &str,
    function_name: &str,
    args: &str,
) -> (String, bool) {
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    if !is_streaming {
        return (format!("{call_prefix}.{function_name}({args})"), false);
    }
    let ctx_args = if args.is_empty() {
        "context.Background()".to_string()
    } else {
        format!("context.Background(), {args}")
    };
    (format!("{call_prefix}.{function_name}({ctx_args})"), true)
}
