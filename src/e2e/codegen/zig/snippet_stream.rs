use super::ZigStreamingAdapterMetadata;
use heck::ToSnakeCase;
use std::fmt::Write;

pub(super) fn render(out: &mut String, adapter: &ZigStreamingAdapterMetadata, module: &str, prefix: &str, args: &str) {
    let owner = adapter.owner_type.to_snake_case();
    let request = adapter.request_type.to_snake_case();
    let item = adapter.item_type.to_snake_case();
    let name = &adapter.adapter_name;
    let _ = writeln!(
        out,
        r#"    const _req_z = try std.heap.c_allocator.dupeZ(u8, {args});
    defer std.heap.c_allocator.free(_req_z);
    const _req_handle = {module}.c.{prefix}_{request}_from_json(_req_z.ptr);
    if (_req_handle == 0) return error.RequestConversionFailed;
    defer {module}.c.{prefix}_{request}_free(_req_handle);
    const _stream_handle = {module}.c.{prefix}_{owner}_{name}_start(_client._handle, _req_handle);
    if (_stream_handle == 0) return error.StreamStartFailed;
    defer {module}.c.{prefix}_{owner}_{name}_free(_stream_handle);
    while (true) {{
        const _chunk = {module}.c.{prefix}_{owner}_{name}_next(_stream_handle);
        if (_chunk == 0) {{
            if ({module}.c.{prefix}_last_error_code() != 0) return error.StreamReadFailed;
            break;
        }}
        defer {module}.c.{prefix}_{item}_free(_chunk);
        const _json = {module}.c.{prefix}_{item}_to_json(_chunk);
        if (_json == null) return error.ChunkSerializationFailed;
        defer {module}.c.{prefix}_free_string(_json);
        std.debug.print("{{s}}\n", .{{std.mem.span(_json)}});
    }}"#
    );
}
