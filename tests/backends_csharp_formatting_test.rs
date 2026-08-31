use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeRef};

/// Whether `dotnet` runs, not merely resolves: a version-manager shim spawns fine then exits
/// non-zero, so a spawn-only check (`.output().is_err()`) would leave the skip below unreachable
/// and fire the assert everywhere the .NET SDK is absent. ~keep
fn dotnet_is_runnable() -> bool {
    std::process::Command::new("dotnet")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn config() -> alef::core::config::ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.csharp]
namespace = "Sample"
"#,
    )
    .unwrap();
    config.resolve().unwrap().remove(0)
}

#[test]
fn generated_csharp_uses_formatter_stable_layout() {
    let parameter = ParamDef {
        name: "value".into(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "sample_core".into(),
        version: "1.0.0".into(),
        functions: vec![
            FunctionDef {
                name: "convert".into(),
                params: vec![parameter.clone()],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
            FunctionDef {
                name: "convert_async".into(),
                params: vec![parameter],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                is_async: true,
                ..Default::default()
            },
        ],
        enums: vec![EnumDef {
            name: "Choice".into(),
            rust_path: "sample_core::Choice".into(),
            serde_tag: Some("kind".into()),
            variants: vec![
                EnumVariant {
                    name: "First".into(),
                    fields: vec![FieldDef {
                        name: "0".into(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    is_tuple: true,
                    originally_had_data_fields: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Second".into(),
                    fields: vec![FieldDef {
                        name: "value".into(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    originally_had_data_fields: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = CsharpBackend.generate_bindings(&api, &config()).unwrap();
    let wrapper = files
        .iter()
        .find(|file| file.path.ends_with("SampleCoreConverter.cs"))
        .expect("SampleCoreConverter.cs");
    assert!(
        wrapper
            .content
            .contains("using System.Text.Json.Serialization;\nusing System.Threading.Tasks;\n")
    );
    assert!(
        wrapper
            .content
            .contains("\n        var nativeResult = NativeMethods.Convert(\n            value\n        );\n"),
        "{}",
        wrapper.content
    );
    assert!(
        !wrapper
            .content
            .contains("\n            var nativeResult = NativeMethods.Convert(\n")
    );

    let choice = files
        .iter()
        .find(|file| file.path.ends_with("Choice.cs"))
        .expect("Choice.cs");
    assert!(
        choice.content.contains("\n            case Choice.First v_first:\n"),
        "{}",
        choice.content
    );
    assert!(
        choice.content.contains("\n            case Choice.Second v_second:\n"),
        "{}",
        choice.content
    );
    assert!(
        choice
            .content
            .contains("\n                JsonSerializer.Deserialize<Choice.First>"),
        "{}",
        choice.content
    );
    assert!(
        choice
            .content
            .contains("\n[JsonConverter(typeof(ChoiceJsonConverter))]\n")
    );

    if !dotnet_is_runnable() {
        assert!(
            std::env::var_os("ALEF_REQUIRE_DOTNET").is_none(),
            "ALEF_REQUIRE_DOTNET is set but dotnet is unavailable"
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    for file in &files {
        let path = directory.path().join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &file.content).unwrap();
    }
    let project = directory.path().join("Sample.csproj");
    std::fs::write(
        &project,
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>",
    )
    .unwrap();
    let output = std::process::Command::new("dotnet")
        .args(["format", "whitespace", "--verify-no-changes"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
