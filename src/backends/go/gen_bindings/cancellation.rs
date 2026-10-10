//! Go's consumer side of the FFI cancel-token API.
//!
//! The FFI crate exports a `<symbol>_cancellable` sibling for every async export, taking one
//! extra trailing cancel-token handle. A Go wrapper backed by one of those is emitted as
//! `<Name>WithContext(ctx, ...)`: it hands the native call a token, trips that token from a
//! watcher goroutine when `ctx` ends, and maps the resulting `Cancelled` failure back to
//! `ctx.Err()`. The plain `<Name>(...)` stays as a thin `context.Background()` wrapper, so its
//! signature is unchanged.
//!
//! Which exports have a sibling is decided by the FFI backend (`is_async`); this module only adds
//! the Go-side eligibility the wrapper shape needs. Symbol spellings come from
//! [`crate::codegen::c_consumer`], the same helpers the FFI backend names its exports with.

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::codegen::c_consumer;
use crate::codegen::naming::go_param_name;
use crate::core::ir::{ApiSurface, ParamDef, ReceiverKind, TypeRef};

const CTX_PARAM: &str = "ctx";

/// Go identifiers the `WithContext` body declares itself; a Rust parameter spelled the same would
/// collide with them.
const BODY_LOCALS: [&str; 4] = [CTX_PARAM, "cancelToken", "cancelDone", "cancelStopped"];

/// The Go call-site spelling of the trailing token argument.
const TOKEN_ARG: &str = "cancelToken";

/// Whether a Go wrapper over an async export gets a `WithContext` variant.
///
/// Requires an `error` return (a cancelled call has nothing else to report it through), no
/// presence companion (that companion re-runs the whole call without a token, so a "cancellable"
/// wrapper would run the uncancellable request first), and no parameter that shadows a local the
/// variant declares.
pub(super) fn is_cancellable(
    is_async: bool,
    can_return_error: bool,
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    params: &[ParamDef],
) -> bool {
    is_async
        && can_return_error
        && !result_presence_companion_exists(return_type, receiver)
        && !params
            .iter()
            .any(|param| BODY_LOCALS.contains(&go_param_name(&param.name).as_str()))
}

/// The `ctx context.Context` parameter list entry, joined onto an existing parameter list.
pub(super) fn with_ctx_param(params: &str) -> String {
    if params.is_empty() {
        format!("{CTX_PARAM} context.Context")
    } else {
        format!("{CTX_PARAM} context.Context, {params}")
    }
}

/// The argument list that forwards a plain wrapper's parameters to its `WithContext` sibling.
pub(super) fn forwarded_args(param_names: &[String]) -> String {
    std::iter::once("context.Background()".to_string())
        .chain(param_names.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Statements that run first in a `WithContext` body: the already-done fast path, then a token
/// that a watcher goroutine trips when `ctx` ends. The deferred cleanup stops the watcher before
/// freeing the token, so the token is never tripped after it is freed.
///
/// `err_return_prefix` is the `"<zero>, "` (or `""`) the enclosing function's other early
/// `return ..., err` exits use.
pub(super) fn prelude(ffi_prefix: &str, err_return_prefix: &str) -> String {
    render_prelude(ffi_prefix, err_return_prefix, true)
}

/// The same setup for a call whose native work outlives the Go function, such as a stream: the
/// cleanup is a `stopCancel()` closure the caller runs when the work ends, instead of a `defer`.
pub(super) fn prelude_with_stop_closure(ffi_prefix: &str, err_return_prefix: &str) -> String {
    render_prelude(ffi_prefix, err_return_prefix, false)
}

fn render_prelude(ffi_prefix: &str, err_return_prefix: &str, defer_cleanup: bool) -> String {
    crate::backends::go::template_env::render(
        "ctx_cancel_prelude.jinja",
        crate::alef_context! {
            defer_cleanup => defer_cleanup,
            err_return_prefix => err_return_prefix,
            new_fn => c_consumer::cancel_token_new_symbol(ffi_prefix),
            cancel_fn => c_consumer::cancel_token_cancel_symbol(ffi_prefix),
            free_fn => c_consumer::cancel_token_free_symbol(ffi_prefix),
        },
    )
}

/// Append the token to a finished cgo call expression, after any out-params, because the FFI
/// sibling takes it as its last parameter.
pub(super) fn with_token_arg(c_call: &str) -> String {
    let open_call = c_call
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("a cgo call expression ends with `)`: {c_call}"));
    if open_call.ends_with('(') {
        format!("{open_call}{TOKEN_ARG})")
    } else {
        format!("{open_call}, {TOKEN_ARG})")
    }
}

/// The cgo symbol of an export's cancellable sibling, given the primary's bare symbol.
pub(super) fn cancellable_symbol(primary_symbol: &str) -> String {
    c_consumer::cancellable_symbol(primary_symbol)
}

/// The `lastError` call a wrapper body uses: the context-aware one inside a `WithContext` body.
pub(super) fn last_error_call(with_context: bool) -> &'static str {
    if with_context {
        "lastErrorContext(ctx)"
    } else {
        "lastError()"
    }
}

/// The `lastErrorContext` helper, emitted once into a package that has at least one caller.
pub(super) fn gen_last_error_context_helper(ffi_prefix: &str) -> String {
    crate::backends::go::template_env::render(
        "last_error_context_helper.jinja",
        crate::alef_context! {
            last_error_code_fn => crate::backends::go::c_symbols::last_error_code_symbol(ffi_prefix),
            cancelled_code => ApiSurface::FFI_ERROR_CODE_CANCELLED,
        },
    )
}

#[cfg(test)]
mod tests;
