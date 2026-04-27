use alef_codegen::c_consumer;

/// Emit the two standard helpers every generated file needs:
///
/// - `_free_string`: wraps the C `{prefix}_free_string` symbol to release
///   FFI-allocated strings. Caller must NOT use the pointer after this call.
/// - `_last_error`: reads the thread-local last-error state set by the FFI
///   layer. Returns a Zig slice pointing into thread-local storage; the
///   pointer is valid until the next FFI call.
pub(crate) fn emit_helpers(prefix: &str, out: &mut String) {
    let free_symbol = c_consumer::free_string_symbol(prefix);
    let error_code_symbol = c_consumer::last_error_code_symbol(prefix);
    let error_context_symbol = c_consumer::last_error_context_symbol(prefix);

    out.push_str("/// Free a string allocated by the FFI layer.\n");
    out.push_str(&format!(
        "/// The pointer must have been returned by a `{prefix}_*` C function.\n"
    ));
    out.push_str("/// Do NOT call this twice on the same pointer.\n");
    out.push_str("pub fn _free_string(ptr: [*c]u8) void {\n");
    out.push_str(&format!("    c.{free_symbol}(ptr);\n"));
    out.push_str("}\n\n");

    out.push_str("/// Retrieve the last error set by the FFI layer, if any.\n");
    out.push_str("/// Returns a slice into thread-local storage valid until the next FFI call.\n");
    out.push_str("pub fn _last_error() ?[]const u8 {\n");
    out.push_str(&format!("    const _code = c.{error_code_symbol}();\n"));
    out.push_str("    if (_code == 0) return null;\n");
    out.push_str(&format!("    const _ctx = c.{error_context_symbol}();\n"));
    out.push_str("    if (_ctx == null) return null;\n");
    out.push_str("    return std.mem.sliceTo(_ctx, 0);\n");
    out.push_str("}\n\n");

    out.push_str("/// Map the last FFI error to a typed error from the given error set.\n");
    out.push_str("/// Coarse fallback: returns the first declared variant. Per-code dispatch\n");
    out.push_str("/// will replace this once the IR exposes per-variant numeric codes.\n");
    out.push_str("inline fn _first_error(comptime E: type) E {\n");
    out.push_str("    const fields = @typeInfo(E).ErrorSet orelse return @as(E, error.Unknown);\n");
    out.push_str("    if (fields.len == 0) unreachable;\n");
    out.push_str("    return @field(E, fields[0].name);\n");
    out.push_str("}\n");
}
