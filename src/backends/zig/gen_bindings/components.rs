pub(super) fn emit(prefix: &str, out: &mut String) {
    out.push_str(&format!(
        r#"/// Errors reported by the verified downloadable-component manager. Use
/// `componentLastErrorCode` after catching one of these for the stable failure code --
/// "offline", "signature_invalid", "unsupported_host", and so on -- instead of matching on
/// `_last_error()`'s free-form message.
pub const ComponentError = error{{ComponentOperationFailed}};

/// The stable failure code of the most recent failing component operation on this thread
/// ("offline", "signature_invalid", "unsupported_host", ...), or null if the last operation
/// succeeded or the native layer did not prefix its message with one. The native layer
/// prefixes every component failure message with "<code>: " (see
/// alef_component_error_message in alef::backends::native_components); this splits that
/// prefix back out of `_last_error()` rather than exposing a second native accessor.
pub fn componentLastErrorCode() ?[]const u8 {{
    const message = _last_error() orelse return null;
    const separator = std.mem.indexOf(u8, message, ": ") orelse return null;
    return message[0..separator];
}}

/// Download, verify, dynamically load, and pin a configured component.
pub fn componentLoad(allocator: std.mem.Allocator, component: []const u8) (ComponentError || std.mem.Allocator.Error)!void {{
    const component_z = try allocator.dupeZ(u8, component);
    defer allocator.free(component_z);
    if (c.{prefix}_component_load(component_z.ptr) != 0) return ComponentError.ComponentOperationFailed;
}}

/// Download and verify one component, or all configured components when component is null.
/// The returned bytes are a JSON array of cache paths owned by the caller's allocator.
pub fn componentPrefetch(
    allocator: std.mem.Allocator,
    component: ?[]const u8,
) (ComponentError || std.mem.Allocator.Error)![]u8 {{
    const component_z = if (component) |value| try allocator.dupeZ(u8, value) else null;
    defer if (component_z) |value| allocator.free(value);
    const raw = c.{prefix}_component_prefetch(if (component_z) |value| value.ptr else null);
    return takeComponentString(allocator, raw);
}}

/// Return ready, cached, not_downloaded, bundled, or unsupported:<reason> for a configured
/// component. See `componentStatusCode` for the matching numeric code.
pub fn componentStatus(
    allocator: std.mem.Allocator,
    component: []const u8,
) (ComponentError || std.mem.Allocator.Error)![]u8 {{
    const component_z = try allocator.dupeZ(u8, component);
    defer allocator.free(component_z);
    return takeComponentString(allocator, c.{prefix}_component_status(component_z.ptr));
}}

/// The numeric counterpart to `componentStatus`, stable across releases: 0 ready, 1 cached,
/// 2 not_downloaded, 3 bundled, 4 unsupported.
pub fn componentStatusCode(
    allocator: std.mem.Allocator,
    component: []const u8,
) (ComponentError || std.mem.Allocator.Error)!i32 {{
    const component_z = try allocator.dupeZ(u8, component);
    defer allocator.free(component_z);
    const result = c.{prefix}_component_status_code(component_z.ptr);
    if (result < 0) return ComponentError.ComponentOperationFailed;
    return result;
}}

/// Return the content-addressed cache path for a configured component.
pub fn componentCachePath(
    allocator: std.mem.Allocator,
    component: []const u8,
) (ComponentError || std.mem.Allocator.Error)![]u8 {{
    const component_z = try allocator.dupeZ(u8, component);
    defer allocator.free(component_z);
    return takeComponentString(allocator, c.{prefix}_component_cache_path(component_z.ptr));
}}

fn takeComponentString(
    allocator: std.mem.Allocator,
    raw: [*c]u8,
) (ComponentError || std.mem.Allocator.Error)![]u8 {{
    if (raw == null) return ComponentError.ComponentOperationFailed;
    defer c.{prefix}_free_string(raw);
    return allocator.dupe(u8, std.mem.span(raw));
}}
"#,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_zig_wrappers_over_shared_c_manager() {
        let mut generated = String::new();
        emit("demo", &mut generated);
        assert!(generated.contains("pub fn componentLoad"));
        assert!(generated.contains("pub fn componentPrefetch"));
        assert!(generated.contains("c.demo_component_status(component_z.ptr)"));
        assert!(generated.contains("pub fn componentStatusCode"));
        assert!(generated.contains("c.demo_component_status_code(component_z.ptr)"));
        assert!(generated.contains("pub fn componentLastErrorCode"));
        assert!(generated.contains("c.demo_component_cache_path(component_z.ptr)"));
        assert!(generated.contains("defer c.demo_free_string(raw)"));
    }
}
