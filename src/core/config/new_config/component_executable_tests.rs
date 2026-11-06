use super::*;

fn executable_config(component: &str) -> NewAlefConfig {
    toml::from_str(&format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.component_contracts]]
name = "poly"
trait_path = "sample_core::Poly"

[[crates.components]]
name = "sh"
provides = [{{ contract = "poly", implementation = "NewShComponent" }}]
{component}

[crates.component_distribution]
url_template = "https://downloads.example.test/{{artifact}}"

[crates.component_distribution.public_keys]
release-2026 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#
    ))
    .expect("executable component config should deserialize")
}

const VALID: &str = r#"
producer_language = "executable"
producer_dir = "components/sidecars/sh"
targets = ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]
executable = { build = "mise run sidecar:build sh {target} --out {out}", toolchain_version_cmd = "go version", inputs = ["components/sidecars/sh"] }
"#;

#[test]
fn resolve_accepts_an_executable_component_without_features_or_a_rust_implementation() {
    let resolved = executable_config(VALID).resolve().unwrap().remove(0);
    let component = &resolved.components[0];
    assert_eq!(
        component.producer_language,
        crate::core::config::ComponentProducerLanguage::Executable
    );
    assert_eq!(component.producer_dir.as_deref(), Some("components/sidecars/sh"));
    assert!(component.features.is_empty());
    let executable = component.executable.as_ref().unwrap();
    assert_eq!(executable.toolchain_version_cmd.as_deref(), Some("go version"));
    assert_eq!(executable.inputs, ["components/sidecars/sh"]);
    assert_eq!(component.provides[0].implementation, "NewShComponent");
}

#[test]
fn resolve_rejects_an_invalid_executable_declaration() {
    for (component, expected) in [
        (
            "producer_language = \"executable\"\nproducer_dir = \"x\"\n",
            "must declare `executable`",
        ),
        (
            "producer_language = \"executable\"\nexecutable = { build = \"make {out}\" }\n",
            "must declare `producer_dir`",
        ),
        (
            "producer_language = \"executable\"\nproducer_dir = \"x\"\nexecutable = { build = \"make\" }\n",
            "`{out}` placeholder",
        ),
        (
            "producer_language = \"executable\"\nproducer_dir = \"x\"\nexecutable = { build = \"make {out} {triple}\" }\n",
            "{triple}",
        ),
        (
            "producer_language = \"executable\"\nproducer_dir = \"x\"\nsource_manifest = \"a/Cargo.toml\"\nexecutable = { build = \"make {out}\" }\n",
            "executable component `sh` cannot declare `source_manifest`",
        ),
        (
            "producer_language = \"executable\"\nproducer_dir = \"x\"\nextra_dependencies = { serde = \"1\" }\nexecutable = { build = \"make {out}\" }\n",
            "executable component `sh` cannot declare Rust extra_dependencies",
        ),
        (
            "producer_language = \"go\"\nproducer_dir = \"x\"\nexecutable = { build = \"make {out}\" }\n",
            "is not \"executable\"",
        ),
    ] {
        let error = executable_config(component).resolve().unwrap_err().to_string();
        assert!(error.contains(expected), "{component}: {error}");
    }
}

#[test]
fn rebasing_roots_executable_inputs_with_the_producer_dir() {
    let mut component = executable_config(VALID).crates.remove(0).components.remove(0);
    component.rebase_paths(std::path::Path::new("sub"));
    assert_eq!(component.producer_dir.as_deref(), Some("sub/components/sidecars/sh"));
    assert_eq!(component.executable.unwrap().inputs, ["sub/components/sidecars/sh"]);
}

#[test]
fn generated_schema_documents_the_executable_surface() {
    let schema = serde_json::to_string(&schemars::schema_for!(crate::core::config::ComponentConfig)).unwrap();
    for key in [
        "executable",
        "toolchain_version_cmd",
        "target_files",
        "\"build\"",
        "inputs",
    ] {
        assert!(schema.contains(key), "{key} missing from the component schema");
    }
    assert!(
        schema.contains("\"executable\""),
        "producer_language must offer `executable`"
    );
}
