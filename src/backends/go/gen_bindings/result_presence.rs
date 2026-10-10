//! Go's consumer side of the FFI presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the Go wrapper as the same bits. Go's return expression for that shape
//! (`go_return_expr`) builds `func() *int64 { v := int64(ptr); return &v }()`, which is never
//! `nil` — so before this gate a Go caller saw `Some(0)` for every `None`. The FFI backend
//! exports an additive `{fn}_has_result` companion that answers presence out of band; this
//! module emits the `if ...has_result(...) != 1 { return nil }` guard that consults it.
//!
//! Whether the companion exists at all is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`] — the same predicate
//! `gen_method_result_presence_wrapper` uses to decide whether to emit the symbol. Re-deriving
//! "is this an ambiguous `Option` leaf" here would let the two sides drift and make Go reference
//! a symbol the FFI crate never exported, which is a link error in consumer code rather than a
//! wrong value. See `two-generators-disagree` in the repo's skill set. ~keep

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::{ReceiverKind, TypeRef};

/// The presence companion's "result is present" return value. `0` means absent and `-1` means
/// the companion itself failed (bad handle, param conversion, caught panic), so the gate tests
/// for `!= 1` rather than `== 0` — treating a failed companion as "present" would hand the
/// caller the primary getter's sentinel as if it were real data. ~keep
const PRESENT: &str = "1";

/// The `return` statement the gate runs when the companion reports absence.
///
/// `lastError()` is correct for both companion outcomes the gate can see: the companion calls
/// `clear_last_error()` on entry and sets nothing on its success path, so a `0` (genuinely
/// absent) result leaves the error code at `0` and `lastError()` yields `nil`, while a `-1`
/// surfaces the real failure instead of silently reporting absence. ~keep
fn absent_return_statement(wrapper_returns_error: bool) -> &'static str {
    if wrapper_returns_error {
        "return nil, lastError()"
    } else {
        "return nil"
    }
}

/// Render the presence guard a Go wrapper emits immediately before its primary C call, or
/// `None` when the FFI crate exports no companion for this return type and receiver.
///
/// `primary_c_symbol` is the primary export's C symbol *without* the cgo `C.` qualifier (for
/// example `sample_settings_timeout`); the companion's name is derived from it through
/// [`result_presence_symbol`] so the `_has_result` spelling has exactly one definition.
/// `c_args` must be the same argument expressions the primary call passes — the companion's C
/// signature is the primary's parameter list, so reusing the caller's own vector keeps the two
/// calls from disagreeing about arity.
///
/// `receiver` is passed straight through to the eligibility authority: the companion re-invokes
/// the underlying method to observe presence, which an owned receiver cannot survive because its
/// first call already removed the handle from the registry. Go must therefore fall back to its
/// existing behavior for owned receivers rather than call a symbol that was never exported.
pub(super) fn result_presence_gate(
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    primary_c_symbol: &str,
    c_args: &[String],
    wrapper_returns_error: bool,
) -> Option<String> {
    if !result_presence_companion_exists(return_type, receiver) {
        return None;
    }
    Some(crate::backends::go::template_env::render(
        "result_presence_gate.jinja",
        crate::alef_context! {
            symbol => result_presence_symbol(primary_c_symbol),
            args => c_args.join(", "),
            present => PRESENT,
            absent_return => absent_return_statement(wrapper_returns_error),
        },
    ))
}

#[cfg(test)]
mod tests;
