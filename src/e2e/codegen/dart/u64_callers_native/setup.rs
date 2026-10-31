use crate::backends::dart::DartBackend;
use crate::core::backend::Backend;
use crate::core::config::{DartConfig, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use sha2::{Digest, Sha256};
use std::path::Path;

const CORE: &str = include_str!("../fixtures/u64_callers.rs");
const ENDPOINT: &str = "\n// Test-only identity endpoint exercises the complete recursive native DTO argument.\npub fn echo_envelope(req: Envelope) -> Envelope { req }\n";

fn sha256_hex(source: &str) -> String {
    Sha256::digest(source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn write(root: &Path, name: &str, content: &str) {
    let destination = root.join(name);
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(destination, content).unwrap();
}

pub(super) fn fixture(root: &Path) -> (ApiSurface, ResolvedCrateConfig) {
    let core = format!("{CORE}{ENDPOINT}");
    assert_eq!(&core.as_bytes()[..CORE.len()], CORE.as_bytes());
    eprintln!(
        "caller_immutable_source_sha256={}\ncaller_combined_source_sha256={}",
        sha256_hex(CORE),
        sha256_hex(&core)
    );
    write_sources(root, &core);
    let source = root.join("core/src/lib.rs");
    let api = crate::extract::extractor::extract(&[&source], "wide-codec", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 6);
    assert_eq!(api.types.len(), 3);
    let echo = api.functions.iter().find(|f| f.name == "echo_u64").unwrap();
    assert_eq!(echo.params[0].ty, TypeRef::Primitive(PrimitiveType::U64));
    let endpoint = api.functions.iter().find(|f| f.name == "echo_envelope").unwrap();
    assert_eq!(endpoint.params[0].name, "req");
    assert_eq!(endpoint.params[0].ty, TypeRef::Named("Envelope".into()));
    let parsed: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"wide-codec\"\nsources=[\"core/src/lib.rs\"]\n",
    )
    .unwrap();
    let mut config = parsed.resolve().unwrap().remove(0);
    config.dart = Some(DartConfig {
        pubspec_name: Some("wide_codec_probe".into()),
        frb_version: Some("2.13.0".into()),
        ..Default::default()
    });
    write_bindings(root, &api, &config);
    (api, config)
}

fn write_sources(root: &Path, core: &str) {
    for (name, content) in [
        ("core/src/lib.rs", core),
        (
            "core/Cargo.toml",
            "[package]\nname=\"wide-codec\"\nversion=\"1.0.0\"\nedition=\"2024\"\n[dependencies]\nserde={version=\"1\",features=[\"derive\"]}\n",
        ),
        (
            "packages/dart/rust/Cargo.toml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/dart_u64_canonical/bridge_Cargo.toml"
            )),
        ),
        (
            "packages/dart/pubspec.yaml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/dart_u64_canonical/pubspec.yaml"
            )),
        ),
        (
            "mise.lock",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/dart_u64_canonical/mise.lock"
            )),
        ),
        (
            ".cargo/config.toml",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/dart_u64_canonical/cargo_config.toml"
            )),
        ),
        (
            "rust-toolchain.toml",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/rust-toolchain.toml")),
        ),
    ] {
        write(root, name, content);
    }
    let tasks = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dart_u64_canonical/mise.toml"
    ));
    write(
        root,
        "mise.toml",
        &format!("{tasks}\n[tasks.poison]\nrun = \"cd packages/dart && dart run poison.dart\"\n"),
    );
}

fn write_bindings(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    let mut written = 0;
    for file in DartBackend.generate_bindings(api, config).unwrap() {
        if file.path.starts_with("packages/dart/lib")
            || [
                "packages/dart/rust/src/lib.rs",
                "packages/dart/rust/build.rs",
                "packages/dart/rust/flutter_rust_bridge.yaml",
            ]
            .iter()
            .any(|p| file.path == Path::new(p))
        {
            write(root, file.path.to_str().unwrap(), &file.content);
            written += 1;
        }
    }
    assert!(written >= 5, "actual Dart facade, DTOs, FRB config and Rust bridge");
    assert!(
        std::fs::read_to_string(root.join("packages/dart/rust/src/lib.rs"))
            .unwrap()
            .contains("pub fn echo_envelope(")
    );
}
