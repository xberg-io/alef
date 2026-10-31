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
        r#"    const _req_z = try std.heap.c_allocator.dupeSentinel(u8, {args}, 0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_emit_explicit_zero_sentinels_for_stream_snippets() {
        let adapter = ZigStreamingAdapterMetadata {
            owner_type: "Client".into(),
            request_type: "Request".into(),
            item_type: "Chunk".into(),
            adapter_name: "watch".into(),
        };
        let mut generated = String::new();
        render(&mut generated, &adapter, "sample", "sample", "request_json");
        assert!(generated.contains("std.heap.c_allocator.dupeSentinel(u8, request_json, 0)"));
        assert!(!generated.contains(".dupeZ("));
    }

    #[test]
    fn should_emit_explicit_zero_sentinels_that_compile_and_preserve_request_bytes() {
        assert!(
            crate::test_support::tool_path_with_stable_path("zig").is_some(),
            "the declared Zig toolchain is required for the native sentinel regression"
        );
        let adapter = ZigStreamingAdapterMetadata {
            owner_type: "Client".into(),
            request_type: "Request".into(),
            item_type: "Chunk".into(),
            adapter_name: "watch".into(),
        };
        let mut generated = String::new();
        render(&mut generated, &adapter, "sample", "sample", "\"sample\"");
        let allocation = generated.lines().next().expect("generated request allocation");
        let source = [
            "const std = @import(\"std\");",
            "test \"generated request sentinel\" {",
            allocation,
            "    defer std.heap.c_allocator.free(_req_z);",
            "    try std.testing.expectEqualSlices(u8, \"sample\", _req_z);",
            "    try std.testing.expectEqual(@as(u8, 0), _req_z[_req_z.len]);",
            "}",
        ]
        .join("\n");
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("sentinel.zig"), source).unwrap();
        std::fs::write(
            directory.path().join("mise.toml"),
            crate::test_support::mise_task::fixture_config_with_tools(
                "[tasks.test]\nrun = 'zig test sentinel.zig -lc --cache-dir cache --global-cache-dir global-cache'\n",
                &["zig"],
            ),
        )
        .unwrap();
        let output = crate::test_support::mise_task::mise_run(directory.path(), "test")
            .output()
            .expect("compile the generated Zig allocation");
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{report}");
        assert!(report.contains("All 1 tests passed."), "{report}");
    }
}
