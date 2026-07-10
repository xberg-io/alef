/// Emit helper functions for error introspection and string memory management.
pub(super) fn emit_error_helpers(
    prefix: &str,
    free_symbol: &str,
    error_code_symbol: &str,
    error_context_symbol: &str,
    out: &mut String,
) {
    use crate::backends::dart::template_env;
    out.push_str("/// Free a string allocated by the FFI layer.\n");
    out.push_str("/// The pointer must have been returned by a `");
    out.push_str(prefix);
    out.push_str("_*` C function.\n");
    out.push_str("typedef _FreeStringNative = Void Function(Pointer<Char> ptr);\n");
    out.push_str("typedef _FreeStringDart = void Function(Pointer<Char> ptr);\n");
    out.push_str("final void Function(Pointer<Char>) _freeString =\n");
    out.push_str(&template_env::render(
        "ffi_free_string_lookup.jinja",
        minijinja::context! {
            free_symbol => free_symbol,
        },
    ));

    out.push_str("/// Check the last FFI error code (0 = success).\n");
    out.push_str("typedef _LastErrorCodeNative = Int32 Function();\n");
    out.push_str("typedef _LastErrorCodeDart = int Function();\n");
    out.push_str("final int Function() _lastErrorCode =\n");
    out.push_str(&template_env::render(
        "ffi_last_error_code_lookup.jinja",
        minijinja::context! {
            error_code_symbol => error_code_symbol,
        },
    ));

    out.push_str("/// Retrieve the last FFI error message, or null if none.\n");
    out.push_str("typedef _LastErrorContextNative = Pointer<Utf8> Function();\n");
    out.push_str("typedef _LastErrorContextDart = Pointer<Utf8> Function();\n");
    out.push_str("final Pointer<Utf8> Function() _lastErrorContext =\n");
    out.push_str(&template_env::render(
        "ffi_last_error_context_lookup.jinja",
        minijinja::context! {
            error_context_symbol => error_context_symbol,
        },
    ));

    out.push_str("/// Throw a [StateError] if the last FFI call failed.\n");
    out.push_str("void _checkError() {\n");
    out.push_str("  final code = _lastErrorCode();\n");
    out.push_str("  if (code != 0) {\n");
    out.push_str("    final ctx = _lastErrorContext();\n");
    out.push_str("    final message = ctx == nullptr ? 'FFI error code $code' : ctx.toDartString();\n");
    out.push_str("    throw StateError(message);\n");
    out.push_str("  }\n");
    out.push_str("}\n\n");
}
