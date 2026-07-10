//! Helper functions for Zig code generation.
//!
//! Provides utilities for FFI introspection and documentation emission.

use crate::codegen::c_consumer;
use crate::core::config::Language;
use crate::docs::clean_doc;

/// Emit the two standard helpers every generated file needs:
///
/// - `_free_string`: wraps the C `{prefix}_free_string` symbol to release
///   FFI-allocated strings. Caller must NOT use the pointer after this call.
/// - `_last_error`: reads the thread-local last-error state set by the FFI
///   layer. Returns a Zig slice pointing into thread-local storage; the
///   pointer is valid until the next FFI call.
///
/// `declared_errors` is the list of error-set type names declared in the
/// module (in declaration order). `_error_with_message` uses this list to
/// dispatch to a per-error message-prefix matcher (`_from_ffi_msg_<name>`)
/// emitted by `emit_error_set`. Without this, every FFI failure returned the
/// first declared variant — masking the real error and confusing diagnostics.
pub(crate) fn emit_helpers(prefix: &str, declared_errors: &[String], out: &mut String) {
    let free_symbol = c_consumer::free_string_symbol(prefix);
    let error_code_symbol = c_consumer::last_error_code_symbol(prefix);
    let error_context_symbol = c_consumer::last_error_context_symbol(prefix);

    out.push_str("/// Free a string allocated by the FFI layer.\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_free_string_doc1.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    ));
    out.push_str("/// Do NOT call this twice on the same pointer.\n");
    out.push_str("pub fn _free_string(ptr: [*c]u8) void {\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_free_string_doc2.jinja",
        minijinja::context! {
            free_symbol => free_symbol,
        },
    ));
    out.push_str("}\n\n");

    out.push_str("/// Retrieve the last error set by the FFI layer, if any.\n");
    out.push_str("/// Returns a slice into thread-local storage valid until the next FFI call.\n");
    out.push_str("pub fn _last_error() ?[]const u8 {\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_last_error_code.jinja",
        minijinja::context! {
            symbol => error_code_symbol,
        },
    ));
    out.push_str("    if (_code == 0) return null;\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_last_error_ctx.jinja",
        minijinja::context! {
            symbol => error_context_symbol,
        },
    ));
    out.push_str("    if (_ctx == null) return null;\n");
    out.push_str("    return std.mem.sliceTo(_ctx, 0);\n");
    out.push_str("}\n\n");

    out.push_str("/// Map the last FFI error to a typed error from the given error set.\n");
    out.push_str("/// Coarse fallback: returns the first declared variant. Per-code dispatch\n");
    out.push_str("/// will replace this once the IR exposes per-variant numeric codes.\n");
    out.push_str("inline fn _first_error(comptime E: type) E {\n");
    out.push_str("    const fields = @typeInfo(E).error_set orelse unreachable;\n");
    out.push_str("    if (fields.len == 0) unreachable;\n");
    out.push_str("    return @field(E, fields[0].name);\n");
    out.push_str("}\n\n");

    out.push_str("/// Map the last FFI error to a typed error, logging the error message.\n");
    out.push_str("/// Reads the FFI error context from `_last_error()` and dispatches to a\n");
    out.push_str("/// per-error-set message-prefix matcher emitted alongside each declared error.\n");
    out.push_str("/// Falls back to the first declared variant when no prefix matches or the\n");
    out.push_str("/// FFI layer did not set a context string.\n");
    out.push_str("inline fn _error_with_message(comptime E: type) E {\n");
    out.push_str("    const msg_opt = _last_error();\n");
    out.push_str("    if (msg_opt) |msg| {\n");
    out.push_str("        std.debug.print(\"FFI error: {s}\\n\", .{msg});\n");
    out.push_str("    }\n");
    if declared_errors.is_empty() {
        out.push_str("    return _first_error(E);\n");
    } else {
        for err_name in declared_errors {
            out.push_str(&format!(
                "    if (E == {err_name}) return _from_ffi_msg_{err_name}(msg_opt);\n"
            ));
        }
        out.push_str("    return _first_error(E);\n");
    }
    out.push_str("}\n");
}

/// Emit cleaned Zig documentation for a declaration.
///
/// Cleans Rust-specific doc strings and formats as Zig doc comments (/// ...).
pub(crate) fn emit_cleaned_zig_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let cleaned = clean_doc(doc, Language::Zig);
    crate::codegen::doc_emission::emit_zig_doc(out, &cleaned, indent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_with_message_dispatches_to_each_declared_error() {
        let mut out = String::new();
        emit_helpers(
            "example_pack",
            &["Error".to_string(), "DownloadError".to_string()],
            &mut out,
        );

        assert!(
            out.contains("if (E == Error) return _from_ffi_msg_Error(msg_opt);"),
            "missing dispatch to _from_ffi_msg_Error:\n{out}"
        );
        assert!(
            out.contains("if (E == DownloadError) return _from_ffi_msg_DownloadError(msg_opt);"),
            "missing dispatch to _from_ffi_msg_DownloadError:\n{out}"
        );
        assert!(
            out.contains("std.debug.print(\"FFI error: {s}\\n\", .{msg});"),
            "missing FFI error print:\n{out}"
        );
        assert!(
            out.contains("return _first_error(E);"),
            "missing _first_error fallback:\n{out}"
        );
    }

    #[test]
    fn error_with_message_falls_back_to_first_error_when_no_errors_declared() {
        let mut out = String::new();
        emit_helpers("crate", &[], &mut out);
        assert!(
            out.contains("inline fn _error_with_message(comptime E: type) E {"),
            "missing _error_with_message decl:\n{out}"
        );
        assert!(
            !out.contains("_from_ffi_msg_"),
            "no per-error matcher should be referenced when none are declared:\n{out}"
        );
        assert!(
            out.contains("return _first_error(E);"),
            "fallback to _first_error required:\n{out}"
        );
    }
}
