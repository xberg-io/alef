//! Kotlin/Native's consumer side of the FFI presence channel.
//!
//! Kotlin/JVM never reaches the C ABI — it calls the Java facade, which already consults the
//! companion. Kotlin/Native does: `kotlinx.cinterop` calls the cbindgen exports directly, so a
//! Rust `Option<i64>` arrives as a bare `Long` and Kotlin widens it into the declared `Long?` as
//! non-null. Before this gate every `None` reached the caller as `0`.
//!
//! Like Zig's `@cImport`, cinterop generates bindings for the whole header, so the companion needs
//! no declaration here — only a call, and only when the FFI crate exported one. That is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`], the same predicate the
//! FFI backend uses to decide whether to emit the symbol. Re-deriving "is this an ambiguous
//! `Option` leaf" here would let the two sides drift into a cinterop name that resolves to
//! nothing. See `two-generators-disagree` in the repo's skill set. ~keep
//!
//! The shape follows Java's rather than Go's: the companion is captured into a local *before* the
//! primary call — every FFI wrapper clears the last-error slot on entry, so running it second
//! would erase an error the primary just recorded — and the local is consulted in the return
//! expression, *after* the wrapper's existing error check. Kotlin/Native's body is a
//! `memScoped { ... }` expression whose error handling is assembled inline from the error
//! taxonomy; folding the test into the return expression reuses that block instead of duplicating
//! it. ~keep

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::backends::kotlin::template_env::render;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::TypeRef;

/// The companion's "result is present" return value. `0` means absent and `-1` means the
/// companion itself failed, so the emitted test is `!= 1` rather than `== 0` — treating a failed
/// companion as "present" would hand the caller the primary call's sentinel as real data. A `-1`
/// also sets the crate's last-error slot, which the wrapper's own error check still sees when the
/// primary fails for the same reason. ~keep
const PRESENT: &str = "1";

/// The Kotlin local the companion's answer is captured into. Distinct from `_result`, `_code` and
/// `_msg`, the names the surrounding body already binds in the same scope.
const PRESENCE_LOCAL: &str = "_present";

/// The statement capturing the companion's answer, or `None` when the FFI crate exports no
/// companion for this return type.
///
/// `primary_c_symbol` is the wrapper's own C symbol and `c_args` the same argument expressions the
/// primary call passes — the companion's C signature *is* the primary's parameter list, so reusing
/// the caller's vector keeps the two calls from disagreeing about arity. Free functions are the
/// only Kotlin/Native emission surface, so the receiver is always `None`.
pub(super) fn presence_capture(return_type: &TypeRef, primary_c_symbol: &str, c_args: &[String]) -> Option<String> {
    if !result_presence_companion_exists(return_type, None) {
        return None;
    }
    Some(render(
        "native_presence_capture.jinja",
        crate::alef_context! {
            local => PRESENCE_LOCAL,
            symbol => result_presence_symbol(primary_c_symbol),
            args => c_args.join(", "),
        },
    ))
}

/// Wrap the wrapper's existing return expression in the presence test.
///
/// `present_expr` is what the body would have yielded unconditionally before the presence channel
/// existed; the declared return type is already nullable for every shape that reaches here, so the
/// absent branch is a bare `null`.
pub(super) fn presence_conditional(present_expr: &str) -> String {
    format!("if ({PRESENCE_LOCAL} != {PRESENT}) null else {present_expr}")
}

#[cfg(test)]
mod tests;
