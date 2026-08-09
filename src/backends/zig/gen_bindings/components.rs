pub(super) fn emit(prefix: &str, out: &mut String) {
    out.push_str(&format!(
        r#"/// Errors reported by the verified downloadable-component manager.
pub const ComponentError = error{{ComponentOperationFailed}};

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

/// Return missing, cached:<path>, or loaded:<path> for a configured component.
pub fn componentStatus(
    allocator: std.mem.Allocator,
    component: []const u8,
) (ComponentError || std.mem.Allocator.Error)![]u8 {{
    const component_z = try allocator.dupeZ(u8, component);
    defer allocator.free(component_z);
    return takeComponentString(allocator, c.{prefix}_component_status(component_z.ptr));
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
        assert!(generated.contains("c.demo_component_cache_path(component_z.ptr)"));
        assert!(generated.contains("defer c.demo_free_string(raw)"));
    }
}
