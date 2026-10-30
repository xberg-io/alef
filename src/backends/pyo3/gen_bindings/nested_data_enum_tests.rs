use super::Pyo3Backend;
use crate::core::backend::Backend;
use crate::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use std::path::Path;

const CORE_SOURCE: &str = r#"
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum Policy {
    #[default]
    None,
    Analyze { label: String },
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Request { pub policy: Policy }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Prepared { pub request: Request }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Batch { pub items: Vec<Prepared> }
"#;

const ROUND_TRIP_TEST: &str = r#"
#[test]
fn nested_enum_payload_survives_core_to_binding_and_serde_round_trips() {
    let expected = serde_json::json!({"items": [
        {"request": {"policy": {"Analyze": {"label": "retain me"}}}}
    ]});
    let core: test_lib::Batch = serde_json::from_value(expected.clone()).unwrap();
    let binding: Batch = core.into();
    assert_eq!(serde_json::to_value(&binding).unwrap(), expected);
    let restored: Batch = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), expected);
}
"#;

fn record(name: &str, field: &str, ty: TypeRef) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_path: format!("test_lib::{name}"),
        has_serde: true,
        has_default: true,
        fields: vec![FieldDef {
            name: field.into(),
            ty,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".into(),
        version: "0.1.0".into(),
        enums: vec![EnumDef {
            name: "Policy".into(),
            rust_path: "test_lib::Policy".into(),
            has_serde: true,
            has_default: true,
            variants: vec![
                EnumVariant {
                    name: "None".into(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Analyze".into(),
                    fields: vec![FieldDef {
                        name: "label".into(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        types: vec![
            record("Request", "policy", TypeRef::Named("Policy".into())),
            record("Prepared", "request", TypeRef::Named("Request".into())),
            record(
                "Batch",
                "items",
                TypeRef::Vec(Box::new(TypeRef::Named("Prepared".into()))),
            ),
        ],
        ..Default::default()
    }
}

fn fixture_config(binding_directory: &Path) -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_test_lib"
"#,
    )
    .unwrap();
    let mut config = config.resolve().unwrap().remove(0);
    config
        .output_paths
        .insert("python".into(), binding_directory.join("src"));
    config
}

fn write_fixture(root: &Path) -> std::path::PathBuf {
    let core = root.join("test-lib");
    let binding = root.join("test-lib-py");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(core.join("src/lib.rs"), CORE_SOURCE).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        r#"
[package]
name = "test-lib"
version = "0.1.0"
edition = "2024"
[dependencies]
serde = { version = "1", features = ["derive"] }
"#,
    )
    .unwrap();
    std::fs::write(
        binding.join("Cargo.toml"),
        r#"
[package]
name = "test-lib-py"
version = "0.1.0"
edition = "2024"
[dependencies]
pyo3 = "=0.29.2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
test-lib = { path = "../test-lib" }
"#,
    )
    .unwrap();
    binding
}

#[test]
fn nested_data_enum_dtos_compile_and_round_trip() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let binding = write_fixture(temporary_directory.path());
    let generated = Pyo3Backend
        .generate_bindings(&api_surface(), &fixture_config(&binding))
        .unwrap();
    let mut source = generated[0].content.clone();
    source.push_str(ROUND_TRIP_TEST);
    std::fs::write(binding.join("src/lib.rs"), &source).unwrap();
    let mut command = std::process::Command::new("cargo");
    crate::test_support::python_native::configure_python(&mut command);
    let output = command
        .args(["test", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", temporary_directory.path().join("target"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generated nested DTO bindings must compile and round-trip:\n{}\n{}\n{source}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
