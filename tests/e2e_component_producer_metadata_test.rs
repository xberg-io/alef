//! Regression coverage for the Rust producer's Cargo and rustdoc metadata.

use alef::codegen::component_producer::generate_component_producers;
use alef::core::backend::GeneratedFile;
use alef::core::config::{
    ComponentConfig, ComponentContractConfig, ComponentProvidesConfig, PackageMetadataConfig, ResolvedCrateConfig,
};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef};
use std::fs;
use std::path::Path;
use std::process::Command;

fn fixture(is_async: bool) -> (ApiSurface, ResolvedCrateConfig) {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "1.0.0".into(),
        types: vec![TypeDef {
            name: "Codec".into(),
            rust_path: "demo::Codec".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "warm".into(),
                is_async,
                receiver: Some(ReceiverKind::Ref),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "demo".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "codec".into(),
            trait_path: "demo::Codec".into(),
            interface_version: 1,
        }],
        components: vec![ComponentConfig {
            producer_language: Default::default(),
            name: "native".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "codec".into(),
                implementation: "demo::NativeCodec".into(),
            }],
            features: Vec::new(),
            default_features: true,
            targets: None,
            bundled_on: Vec::new(),
            source_manifest: None,
            producer_dir: None,
            extra_dependencies: Default::default(),
        }],
        ..ResolvedCrateConfig::default()
    };
    (api, config)
}

fn file<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing generated {suffix}"))
        .content
}

fn manifest(api: &ApiSurface, config: &ResolvedCrateConfig) -> toml::Value {
    let files = generate_component_producers(api, config).expect("generate producer");
    toml::from_str(file(&files, "Cargo.toml")).expect("valid generated Cargo manifest")
}

#[test]
fn should_omit_futures_when_every_provided_method_is_synchronous() {
    let (api, config) = fixture(false);
    let manifest = manifest(&api, &config);
    assert!(manifest["dependencies"].get("futures").is_none());
}

#[test]
fn should_include_futures_when_a_provided_method_is_async() {
    let (api, config) = fixture(true);
    let manifest = manifest(&api, &config);
    assert_eq!(manifest["dependencies"]["futures"].as_str(), Some("0.3"));
}

fn licensed_fixture(license: &str) -> (tempfile::TempDir, ApiSurface, ResolvedCrateConfig) {
    let directory = tempfile::tempdir().expect("workspace");
    let (api, mut config) = fixture(false);
    fs::create_dir_all(directory.path().join("crates/demo/src")).expect("core directory");
    fs::write(
        directory.path().join("crates/demo/Cargo.toml"),
        format!("[package]\nname = \"demo\"\nversion = \"1.0.0\"\nedition = \"2024\"\n{license}\n"),
    )
    .expect("core manifest");
    config.workspace_root = Some(directory.path().into());
    config.sources = vec!["crates/demo/src/lib.rs".into()];
    (directory, api, config)
}

#[test]
fn should_copy_the_core_package_license_as_a_literal() {
    let (_directory, api, config) = licensed_fixture("license = \"MIT OR Apache-2.0\"");
    assert_eq!(
        manifest(&api, &config)["package"]["license"].as_str(),
        Some("MIT OR Apache-2.0")
    );
}

#[test]
fn should_resolve_the_core_workspace_license_as_a_literal() {
    let (directory, api, config) = licensed_fixture("license.workspace = true");
    fs::write(
        directory.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/demo\"]\n[workspace.package]\nlicense = \"BSD-3-Clause\"\n",
    )
    .expect("workspace manifest");
    assert_eq!(
        manifest(&api, &config)["package"]["license"].as_str(),
        Some("BSD-3-Clause")
    );
}

#[test]
fn should_resolve_the_core_self_hosted_workspace_license_before_the_outer_workspace() {
    let (directory, api, config) =
        licensed_fixture("license.workspace = true\n[workspace]\n[workspace.package]\nlicense = \"BSD-3-Clause\"");
    fs::write(
        directory.path().join("Cargo.toml"),
        "[workspace]\nexclude = [\"crates/demo\"]\n[workspace.package]\nlicense = \"MIT\"\n",
    )
    .expect("outer workspace manifest");
    assert_eq!(
        manifest(&api, &config)["package"]["license"].as_str(),
        Some("BSD-3-Clause")
    );
}

#[test]
fn should_resolve_the_core_explicit_package_workspace_license() {
    let (directory, api, config) = licensed_fixture("license.workspace = true\nworkspace = \"../../alternate\"");
    fs::write(
        directory.path().join("Cargo.toml"),
        "[workspace]\nexclude = [\"crates/demo\"]\n[workspace.package]\nlicense = \"MIT\"\n",
    )
    .expect("outer workspace manifest");
    fs::create_dir(directory.path().join("alternate")).expect("explicit workspace directory");
    fs::write(
        directory.path().join("alternate/Cargo.toml"),
        "[workspace]\nmembers = [\"../crates/demo\"]\n[workspace.package]\nlicense = \"BSD-3-Clause\"\n",
    )
    .expect("explicit workspace manifest");
    assert_eq!(
        manifest(&api, &config)["package"]["license"].as_str(),
        Some("BSD-3-Clause")
    );
}

#[test]
fn should_prefer_explicit_package_metadata_license() {
    let (_directory, api, mut config) = licensed_fixture("license = \"MIT\"");
    config.package_metadata = Some(PackageMetadataConfig {
        license: Some("Apache-2.0".into()),
        ..PackageMetadataConfig::default()
    });
    assert_eq!(
        manifest(&api, &config)["package"]["license"].as_str(),
        Some("Apache-2.0")
    );
}

#[test]
fn should_leave_license_absent_when_no_source_declares_one() {
    let (_directory, api, config) = licensed_fixture("");
    assert!(manifest(&api, &config)["package"].get("license").is_none());
}

#[test]
fn should_document_the_crate_and_every_exported_entrypoint() {
    let (mut api, mut config) = fixture(false);
    let mut second_trait = api.types[0].clone();
    second_trait.name = "Decoder".into();
    second_trait.rust_path = "demo::Decoder".into();
    api.types.push(second_trait);
    config.component_contracts.push(ComponentContractConfig {
        name: "decoder".into(),
        trait_path: "demo::Decoder".into(),
        interface_version: 1,
    });
    config.components[0].provides.push(ComponentProvidesConfig {
        contract: "decoder".into(),
        implementation: "demo::NativeDecoder".into(),
    });
    let files = generate_component_producers(&api, &config).expect("generate producer");
    let source = syn::parse_file(file(&files, "src/lib.rs")).expect("valid generated Rust");
    assert!(
        source.attrs.iter().any(|attribute| attribute.path().is_ident("doc")),
        "missing crate docs"
    );
    let entries = source.items.iter().filter_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident.to_string().starts_with("alef_component_entry_v1_") => {
            Some(function)
        }
        _ => None,
    });
    let mut documented_entries = Vec::new();
    for entry in entries {
        assert!(
            entry.attrs.iter().any(|attribute| attribute.path().is_ident("doc")),
            "missing docs for {}",
            entry.sig.ident
        );
        documented_entries.push(entry.sig.ident.to_string());
    }
    documented_entries.sort();
    assert_eq!(
        documented_entries,
        ["alef_component_entry_v1_codec", "alef_component_entry_v1_decoder"]
    );
}

fn qualify_command(program: &str, arguments: &[&str], directory: &Path) -> String {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .unwrap_or_else(|error| panic!("running {program} in {}: {error}", directory.display()));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{program} {arguments:?}:\n{stdout}\n{stderr}");
    stdout.into_owned()
}

fn write_qualification_producer(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) -> std::path::PathBuf {
    let files = generate_component_producers(api, config).expect("generate qualification producer");
    assert_eq!(files.len(), 3, "one manifest, one Rust source, one contract header");
    let producer = root.join("crates/demo-native-component");
    for generated in files {
        let path = root.join(&generated.path);
        fs::create_dir_all(path.parent().expect("generated parent")).expect("create generated directory");
        let content = if path.ends_with("Cargo.toml") {
            let mut manifest: toml::Value = toml::from_str(&generated.content).expect("generated manifest");
            let abi = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/alef-component-abi");
            manifest["dependencies"]["alef-component-abi"] = toml::Value::Table(toml::Table::from_iter([(
                "path".into(),
                abi.display().to_string().into(),
            )]));
            manifest
                .as_table_mut()
                .expect("producer manifest table")
                .insert("workspace".into(), toml::Value::Table(toml::Table::new()));
            toml::to_string(&manifest).expect("local qualification manifest")
        } else {
            generated.content
        };
        fs::write(path, content).expect("write generated qualification file");
    }
    producer
}

#[test]
#[ignore = "requires Rust compilation and cargo-machete; run explicitly for producer qualification"]
fn should_qualify_sync_and_async_producers_with_rustdoc_machete_and_cargo_metadata() {
    let mut qualified = 0;
    for is_async in [false, true] {
        let (directory, mut api, mut config) = licensed_fixture("license = \"MIT OR Apache-2.0\"");
        api.types[0].methods[0].is_async = is_async;
        config.sources = vec![directory.path().join("crates/demo/src/lib.rs")];
        let qualifier = if is_async { "async " } else { "" };
        fs::write(
            &config.sources[0],
            format!(
                "pub trait Codec: Send + Sync {{ {qualifier}fn warm(&self); }}\n\
                 #[derive(Default)]\npub struct NativeCodec;\n\
                 impl Codec for NativeCodec {{ {qualifier}fn warm(&self) {{}} }}\n"
            ),
        )
        .expect("qualification core source");
        let producer = write_qualification_producer(directory.path(), &api, &config);
        let metadata = qualify_command("cargo", &["metadata", "--no-deps", "--format-version", "1"], &producer);
        let metadata: serde_json::Value = serde_json::from_str(&metadata).expect("Cargo metadata JSON");
        let packages = metadata["packages"].as_array().expect("Cargo packages");
        assert_eq!(
            packages.len(),
            1,
            "the isolated producer must be the only workspace package"
        );
        assert_eq!(packages[0]["name"], "demo-native-component");
        assert_eq!(packages[0]["license"], "MIT OR Apache-2.0");
        qualify_command("cargo", &["rustdoc", "--lib", "--", "-Dmissing-docs"], &producer);
        qualify_command("cargo-machete", &[], &producer);
        qualified += 1;
    }
    assert_eq!(qualified, 2, "both synchronous and asynchronous producers must qualify");
}
