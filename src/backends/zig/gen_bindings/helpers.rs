//! Helper functions for Zig code generation.
//!
//! Provides utilities for FFI introspection and documentation emission.

use crate::codegen::c_consumer;
use crate::core::config::Language;
use crate::core::ir::ErrorDef;
use crate::docs::clean_doc;

/// Emit the two standard helpers every generated file needs:
///
/// - `_free_string`: wraps the C `{prefix}_free_string` symbol to release
///   FFI-allocated strings. Caller must NOT use the pointer after this call.
/// - `_last_error`: returns the binding-owned copy of the last captured error
///   message, if any. `_capture_error_message` populates that copy from the
///   FFI layer's thread-local state *before* `_error_with_message` returns —
///   every generated wrapper function's deferred `_free` calls run once the
///   error value is produced but before the caller regains control, and each
///   `_free` re-enters the FFI layer through `catch_ffi_panic`, which clears
///   that thread-local state on entry. Reading it lazily from `_last_error`
///   after the wrapper returns would therefore observe an already-cleared
///   (and potentially freed) buffer; capturing eagerly, before any deferred
///   call can run, is mandatory.
///
/// `declared_errors` is the list of error sets declared in the module (in
/// declaration order). `_error_with_message` dispatches on the stable numeric
/// FFI taxonomy code carried by `#[alef(error_code = N)]`. A failure that no
/// declared code substantiates maps to `error.UnknownFfiError` — the member
/// `emit_error_set` injects into every generated error set — mirroring how the
/// C FFI layer surfaces `ALEF_FFI_UNKNOWN_ERROR`. It must never resolve to an
/// arbitrary declared variant, which would report a specific, wrong error.
pub(crate) fn emit_helpers(prefix: &str, declared_errors: &[ErrorDef], out: &mut String) {
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

    out.push_str("/// Binding-owned copy of the last error message captured by\n");
    out.push_str("/// `_capture_error_message`. Freed and replaced each time a new error is\n");
    out.push_str("/// captured; outlives the deferred FFI calls that clear the underlying\n");
    out.push_str("/// thread-local state in the FFI layer.\n");
    out.push_str("threadlocal var _captured_error: ?[]u8 = null;\n\n");

    out.push_str("/// Retrieve the last error message captured by `_error_with_message`, if any.\n");
    out.push_str("/// Returns a slice into binding-owned storage, valid until the next captured\n");
    out.push_str("/// error overwrites it.\n");
    out.push_str("pub fn _last_error() ?[]const u8 {\n");
    out.push_str("    return _captured_error;\n");
    out.push_str("}\n\n");

    out.push_str("/// Copy the FFI layer's current error message into binding-owned storage.\n");
    out.push_str(&format!(
        "/// Must run before any deferred `_free` call: every `{prefix}_*` FFI entry point\n"
    ));
    out.push_str("/// clears the thread-local error state on entry (`catch_ffi_panic` opens with\n");
    out.push_str("/// `clear_last_error()`), so a `_free` invoked between the failing call and this\n");
    out.push_str("/// capture would wipe the message before it could be read.\n");
    out.push_str("fn _capture_error_message() void {\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_last_error_code.jinja",
        minijinja::context! {
            symbol => error_code_symbol,
        },
    ));
    out.push_str("    if (_code == 0) return;\n");
    out.push_str(&crate::backends::zig::template_env::render(
        "helper_last_error_ctx.jinja",
        minijinja::context! {
            symbol => error_context_symbol,
        },
    ));
    out.push_str("    if (_ctx == null) return;\n");
    out.push_str("    const _msg = std.mem.sliceTo(_ctx, 0);\n");
    out.push_str("    if (_captured_error) |_old| std.heap.c_allocator.free(_old);\n");
    out.push_str("    _captured_error = std.heap.c_allocator.dupe(u8, _msg) catch null;\n");
    out.push_str("}\n\n");

    let dispatching: Vec<&ErrorDef> = declared_errors
        .iter()
        .filter(|error| error.variants.iter().any(|variant| variant.error_code.is_some()))
        .collect();

    out.push_str("/// Map the last FFI error to a typed error.\n");
    if dispatching.is_empty() {
        out.push_str("/// No declared variant carries a stable numeric FFI taxonomy code\n");
        out.push_str("/// (`#[alef(error_code = N)]`), so no specific variant can be\n");
        out.push_str("/// substantiated: every failure maps to `error.UnknownFfiError`.\n");
    } else {
        out.push_str("/// Dispatches exclusively on the stable numeric FFI taxonomy code.\n");
        out.push_str("/// A code matching no declared variant maps to `error.UnknownFfiError`.\n");
    }
    out.push_str("inline fn _error_with_message(comptime E: type) E {\n");
    out.push_str("    _capture_error_message();\n");
    if !dispatching.is_empty() {
        out.push_str(&format!(
            "    const code = @as(i32, @intCast(c.{error_code_symbol}()));\n"
        ));
        for error in dispatching {
            out.push_str(&format!("    if (E == {}) return switch (code) {{\n", error.name));
            for variant in &error.variants {
                let Some(error_code) = variant.error_code else {
                    continue;
                };
                let variant_name = crate::codegen::naming::public_host_identifier(
                    Language::Zig,
                    crate::codegen::naming::PublicIdentifierKind::Type,
                    &variant.name,
                );
                out.push_str(&format!("        {error_code} => error.{variant_name},\n"));
            }
            out.push_str("        else => error.UnknownFfiError,\n    };\n");
        }
    }
    out.push_str("    return error.UnknownFfiError;\n");
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

    fn request_error(error_code: Option<u32>) -> ErrorDef {
        ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample::RequestError".to_string(),
            variants: vec![crate::core::ir::ErrorVariant {
                error_code,
                name: "InvalidInput".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn error_with_message_dispatches_to_each_declared_error() {
        let errors = vec![request_error(Some(100))];
        let mut out = String::new();
        emit_helpers("example_pack", &errors, &mut out);

        assert!(
            out.contains("100 => error.InvalidInput"),
            "missing numeric taxonomy dispatch:\n{out}"
        );
        assert!(
            !out.contains("std.debug.print"),
            "FFI helpers must not write to stderr:\n{out}"
        );
        assert!(
            out.contains("        else => error.UnknownFfiError,\n"),
            "a code matching no declared variant must resolve to the opaque unknown error:\n{out}"
        );
        assert!(
            out.contains("    return error.UnknownFfiError;\n"),
            "an error set with no dispatch block must resolve to the opaque unknown error:\n{out}"
        );
    }

    /// A variant with no `#[alef(error_code = N)]` cannot be substantiated from the FFI
    /// taxonomy code, so it must not appear in the switch — and, critically, the arm it does
    /// not fill must not be back-filled with some other declared variant. Zig used to emit
    /// `_first_error(E)` here, which returns `@field(E, fields[0].name)` — the *first declared*
    /// variant — so every failure was reported as a specific, wrong error rather than an
    /// unknown one. See `ALEF_FFI_UNKNOWN_ERROR` in the C layer for the honest equivalent. ~keep
    #[test]
    fn unnumbered_error_variants_never_resolve_to_a_declared_variant() {
        let errors = vec![request_error(None)];
        let mut out = String::new();

        emit_helpers("example_pack", &errors, &mut out);

        assert!(
            !out.contains("error.InvalidInput"),
            "an uncoded variant must never be named by the dispatcher:\n{out}"
        );
        assert!(
            !out.contains("_first_error"),
            "the first-declared-variant fallback must not be emitted at all:\n{out}"
        );
        assert!(
            !out.contains("fields[0].name"),
            "nothing may resolve an FFI failure by declaration order:\n{out}"
        );
        assert!(
            !out.contains("switch (code)"),
            "with no coded variant there is nothing to dispatch on:\n{out}"
        );
        assert!(
            out.contains("    return error.UnknownFfiError;\n"),
            "every failure must surface as the opaque unknown error:\n{out}"
        );
        assert!(
            !out.contains("Dispatches exclusively on the stable numeric FFI taxonomy code"),
            "the emitted doc must not claim a dispatch that does not exist:\n{out}"
        );
    }

    /// Regression: `_error_with_message` used to discard the FFI error message with
    /// `_ = _last_error();` when no declared variant carried a taxonomy code. Every generated
    /// wrapper function's deferred `_free` calls run before the caller regains control and
    /// re-enter the FFI layer, which clears the thread-local error state on entry -- so by the
    /// time a caller invoked `_last_error()` afterwards, the message was already gone. The fix
    /// captures the message into binding-owned storage inside `_error_with_message`, before any
    /// deferred call can run. ~keep
    #[test]
    fn error_with_message_captures_context_into_binding_owned_storage() {
        let mut out = String::new();
        emit_helpers("example_pack", &[], &mut out);

        assert!(
            !out.contains("_ = _last_error();"),
            "the FFI error context must not be read and discarded:\n{out}"
        );
        assert!(
            out.contains("threadlocal var _captured_error: ?[]u8 = null;"),
            "missing binding-owned error message storage:\n{out}"
        );
        assert!(
            out.contains("fn _capture_error_message() void {"),
            "missing the capture routine:\n{out}"
        );
        assert!(
            out.contains("    _capture_error_message();\n"),
            "_error_with_message must capture before returning:\n{out}"
        );
        assert!(
            out.contains("pub fn _last_error() ?[]const u8 {\n    return _captured_error;\n}"),
            "_last_error must read the binding-owned copy, not the FFI layer directly:\n{out}"
        );
    }

    #[test]
    fn error_with_message_returns_unknown_when_no_errors_declared() {
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
            !out.contains("_first_error"),
            "the first-declared-variant fallback must not be emitted at all:\n{out}"
        );
        assert!(
            out.contains("    return error.UnknownFfiError;\n"),
            "unknown-error fallback required:\n{out}"
        );
        assert!(
            !out.contains("const code ="),
            "no taxonomy code is read when nothing dispatches on it (unused local):\n{out}"
        );
    }
}
