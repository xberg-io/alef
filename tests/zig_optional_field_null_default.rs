use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::ApiSurface;

fn surface() -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Usage { pub total: u32 }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Chunk {
    pub id: String,
    pub usage: Option<Usage>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum Event {
    Done,
    Delta { index: u32, text: Option<String> },
}
"#,
    )
    .unwrap();
    alef::extract::extractor::extract(&[&path], "fixture", "1.0.0", None).unwrap()
}

fn render(surface: &ApiSurface) -> String {
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages = ['zig']\n[[crates]]\nname = 'fixture'\nsources = ['src/lib.rs']\n")
            .unwrap();
    let config: ResolvedCrateConfig = raw.resolve().unwrap().remove(0);
    alef::backends::zig::ZigBackend
        .generate_bindings(surface, &config)
        .unwrap()
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn should_default_every_optional_struct_field_to_null() {
    let generated = render(&surface());
    for declaration in [
        "usage: ?Usage = null,",
        "system_fingerprint: ?[]const u8 = null,",
        "service_tier: ?[]const u8 = null,",
    ] {
        assert!(generated.contains(declaration), "{generated}");
    }
}

#[test]
fn should_keep_required_struct_fields_without_a_default() {
    let generated = render(&surface());
    assert!(generated.contains("    id: []const u8,\n"), "{generated}");
    assert!(!generated.contains("id: []const u8 ="), "{generated}");
}

#[test]
fn should_default_optional_fields_of_tagged_enum_payload_structs_to_null() {
    let generated = render(&surface());
    assert!(generated.contains("text: ?[]const u8 = null,"), "{generated}");
    assert!(generated.contains("index: u32,\n"), "{generated}");
}

#[test]
fn should_parse_a_chunk_that_omits_optional_keys_with_the_strict_parser() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("bindings.zig"), render(&surface())).unwrap();
    std::fs::write(
        directory.path().join("probe.zig"),
        r#"
const std = @import("std");
const bindings = @import("bindings.zig");
test "chunk without optional keys parses" {
    const parsed = try std.json.parseFromSliceLeaky(
        bindings.Chunk, std.heap.page_allocator, "{\"id\":\"c1\"}", .{ .allocate = .alloc_always });
    try std.testing.expectEqualStrings("c1", parsed.id);
    try std.testing.expectEqual(@as(?bindings.Usage, null), parsed.usage);
    try std.testing.expectEqual(@as(?[]const u8, null), parsed.system_fingerprint);
}
test "a missing required key still fails" {
    try std.testing.expectError(error.MissingField, std.json.parseFromSliceLeaky(
        bindings.Chunk, std.heap.page_allocator, "{\"usage\":null}", .{ .allocate = .alloc_always }));
}
"#,
    )
    .unwrap();
    let output = std::process::Command::new("zig")
        .current_dir(directory.path())
        .args(["test", "probe.zig"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("All 2 tests passed."));
}
