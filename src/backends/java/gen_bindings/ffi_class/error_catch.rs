//! Shared emission of the trailing `catch` chain that closes a generated FFI method body.
//!
//! The typed clause must come first in every such chain. `checkLastError()` maps a native error
//! code onto an exception subclass of the crate's base exception and throws it from inside the
//! `try`, so a chain whose first clause is `catch (Throwable e)` re-wraps that subclass into a
//! fresh base exception carrying the placeholder "FFI call failed" message and demotes the real
//! native detail to a nested cause. Every emitting path routes through here so no path can grow
//! a bare `Throwable` chain of its own again. ~keep

const METHOD_INDENT: &str = "        ";
const VISITOR_OPERATION_INDENT: &str = "            ";

/// Closes a generated FFI method body: typed rethrow, then the `Throwable` fallback wrap.
pub(super) fn emit_method_catch_chain(out: &mut String, exception_class: &str) {
    render_into(out, exception_class, METHOD_INDENT, false, "");
}

/// Opens the visitor operation `try`, declaring the `operationFailure` slot its catch chain fills.
///
/// The slot is typed as the crate exception rather than `Throwable` because the `Throwable` clause
/// of the matching catch chain rethrows the slot itself. `throw operationFailure;` on a `Throwable`
/// slot is `unreported exception Throwable` under `javac`, since the enclosing method only declares
/// `throws <Crate>Exception`. Every value assigned to the slot is already that exception type. ~keep
pub(super) fn emit_visitor_operation_open(out: &mut String, exception_class: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_visitor_operation_open.jinja",
        crate::alef_context! {
            exception_class,
            indent => VISITOR_OPERATION_INDENT,
        },
    ));
}

/// Closes the visitor operation `try` inside a `convertWithVisitor` internal method.
///
/// Records the escaping exception in `operationFailure` so the `finally` cleanup block can
/// attach its own failures as suppressed exceptions, then opens that `finally`.
pub(super) fn emit_visitor_operation_catch_chain(out: &mut String, exception_class: &str) {
    render_into(out, exception_class, VISITOR_OPERATION_INDENT, true, " finally {");
}

fn render_into(out: &mut String, exception_class: &str, indent: &str, capture_operation_failure: bool, trailer: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        "ffi_typed_rethrow_catch.jinja",
        crate::alef_context! {
            exception_class,
            indent,
            capture_operation_failure,
            trailer,
        },
    ));
}
