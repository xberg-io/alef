use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, DefaultValue, TypeRef};

fn surface() -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BlockDetails { pub vendor: String }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[alef(legacy_constructor_fields = ["url"])]
pub struct Response {
    pub url: String,
    #[serde(default)]
    pub block_details: Option<BlockDetails>,
    #[serde(default)]
    pub retry_enabled: Option<bool>,
    #[serde(default)]
    pub attempts: Option<u32>,
}
"#,
    )
    .unwrap();
    alef::extract::extractor::extract(&[&path], "fixture", "1.0.0", None).unwrap()
}

fn render(surface: &ApiSurface) -> anyhow::Result<String> {
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages = ['zig']\n[[crates]]\nname = 'fixture'\nsources = ['src/lib.rs']\n")
            .unwrap();
    let config: ResolvedCrateConfig = raw.resolve().unwrap().remove(0);
    Ok(alef::backends::zig::ZigBackend
        .generate_bindings(surface, &config)?
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n"))
}

#[test]
fn should_default_extracted_optional_constructor_extensions_to_null() {
    let api = surface();
    let response = api.types.iter().find(|ty| ty.name == "Response").unwrap();
    let details = response
        .fields
        .iter()
        .find(|field| field.name == "block_details")
        .unwrap();
    assert_eq!(details.ty, TypeRef::Named("BlockDetails".into()));
    assert!(details.optional);
    assert_eq!(details.typed_default, Some(DefaultValue::Empty));
    let generated = render(&api).unwrap();
    for declaration in [
        "block_details: ?BlockDetails = null,",
        "retry_enabled: ?bool = null,",
        "attempts: ?u32 = null,",
    ] {
        assert!(generated.contains(declaration), "{generated}");
    }
    assert!(generated.contains("url: []const u8,"), "{generated}");
    assert!(!generated.contains("url: []const u8 ="), "{generated}");
}

#[test]
fn should_preserve_explicit_false_and_zero_optional_constructor_defaults() {
    let mut api = surface();
    let response = api.types.iter_mut().find(|ty| ty.name == "Response").unwrap();
    response.fields.retain(|field| field.name != "block_details");
    for field in &mut response.fields {
        field.typed_default = match field.name.as_str() {
            "retry_enabled" => Some(DefaultValue::BoolLiteral(false)),
            "attempts" => Some(DefaultValue::IntLiteral(0)),
            _ => field.typed_default.clone(),
        };
    }
    let generated = render(&api).unwrap();
    assert!(generated.contains("retry_enabled: ?bool = false,"), "{generated}");
    assert!(generated.contains("attempts: ?u32 = 0,"), "{generated}");
}

#[test]
fn should_reject_unsupported_required_named_constructor_defaults() {
    let mut api = surface();
    let response = api.types.iter_mut().find(|ty| ty.name == "Response").unwrap();
    response
        .fields
        .iter_mut()
        .find(|field| field.name == "block_details")
        .unwrap()
        .optional = false;
    let error = render(&api).unwrap_err().to_string();
    assert!(
        error.contains("Response.block_details") && error.contains("BlockDetails"),
        "{error}"
    );
}

#[test]
fn should_construct_legacy_and_explicit_nullable_values_in_generated_zig() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("bindings.zig"), render(&surface()).unwrap()).unwrap();
    std::fs::write(
        directory.path().join("probe.zig"),
        r#"
const std = @import("std");
const bindings = @import("bindings.zig");
test "legacy defaults and explicit nullable values" {
    const legacy = bindings.Response{ .url = "example.test" };
    try std.testing.expectEqual(@as(?bindings.BlockDetails, null), legacy.block_details);
    try std.testing.expectEqual(@as(?bool, null), legacy.retry_enabled);
    try std.testing.expectEqual(@as(?u32, null), legacy.attempts);
    const explicit = bindings.Response{
        .url = "example.test", .block_details = .{ .vendor = "probe" },
        .retry_enabled = false, .attempts = 0,
    };
    try std.testing.expectEqualStrings("probe", explicit.block_details.?.vendor);
    try std.testing.expectEqual(false, explicit.retry_enabled.?);
    try std.testing.expectEqual(@as(u32, 0), explicit.attempts.?);
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("All 1 tests passed."));
}
