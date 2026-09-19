//! `alef` publishes as three crates in dependency order (`alef-component-abi` ->
//! `alef-component-runtime` -> `alef`, see `.github/workflows/publish.yaml`), but each one
//! still carries its own `[package] version` and the two dependency pins that reference
//! them are hand-written strings, not `version.workspace = true` inheritance. Nothing at
//! compile time catches those four numbers drifting apart, and a mismatch either breaks
//! `cargo publish` (a path dependency whose `version` req the registry copy can't satisfy)
//! or ships generated consumer manifests pinned to a component version that was never
//! published (`src/scaffold/languages/*.rs`, `src/codegen/component_producer.rs` all emit
//! `env!("CARGO_PKG_VERSION")`, i.e. the root crate's own version, as the pin).
//!
//! `task set-version -- X.Y.Z` is what is supposed to keep the four in lockstep; this test
//! is the regression guard for that invariant, independent of whether `set-version` was
//! actually run.

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest(relative_path: &str) -> toml::Value {
    let path = repository_root().join(relative_path);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    toml::from_str(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn package_version<'a>(manifest: &'a toml::Value, manifest_path: &str) -> &'a str {
    manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{manifest_path} must declare [package] version"))
}

/// The `version` requirement string a path dependency on `name` declares in `manifest`.
fn dependency_version_pin<'a>(manifest: &'a toml::Value, manifest_path: &str, name: &str) -> &'a str {
    manifest
        .get("dependencies")
        .and_then(|dependencies| dependencies.get(name))
        .unwrap_or_else(|| panic!("{manifest_path} must depend on {name}"))
        .get("version")
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("{manifest_path}'s {name} dependency must declare a version requirement"))
}

#[test]
fn component_crates_and_their_dependency_pins_share_the_root_version() {
    let root_manifest_path = "Cargo.toml";
    let abi_manifest_path = "crates/alef-component-abi/Cargo.toml";
    let runtime_manifest_path = "crates/alef-component-runtime/Cargo.toml";

    let root_manifest = read_manifest(root_manifest_path);
    let abi_manifest = read_manifest(abi_manifest_path);
    let runtime_manifest = read_manifest(runtime_manifest_path);

    let root_version = package_version(&root_manifest, root_manifest_path);
    let abi_version = package_version(&abi_manifest, abi_manifest_path);
    let runtime_version = package_version(&runtime_manifest, runtime_manifest_path);

    assert_eq!(
        abi_version, root_version,
        "alef-component-abi must ship at the same version as alef itself"
    );
    assert_eq!(
        runtime_version, root_version,
        "alef-component-runtime must ship at the same version as alef itself"
    );

    let root_runtime_pin = dependency_version_pin(&root_manifest, root_manifest_path, "alef-component-runtime");
    assert_eq!(
        root_runtime_pin, root_version,
        "alef's dependency on alef-component-runtime must be pinned to the version it will \
         actually be published at, or `cargo publish` fails once the workspace path \
         dependency is resolved against the registry copy"
    );

    let runtime_abi_pin = dependency_version_pin(&runtime_manifest, runtime_manifest_path, "alef-component-abi");
    assert_eq!(
        runtime_abi_pin, abi_version,
        "alef-component-runtime's dependency on alef-component-abi must be pinned to the \
         version it will actually be published at"
    );
}

/// `.github/workflows/publish.yaml` must publish the two component crates before `alef`
/// itself (crates.io rejects a manifest whose path dependency has no matching published
/// version), and never drop one from the batch — see `Finding 1` in the components design
/// review this test was added for.
#[test]
fn publish_workflow_publishes_component_crates_before_alef() {
    let workflow_path = Path::new(".github/workflows/publish.yaml");
    let workflow = std::fs::read_to_string(repository_root().join(workflow_path))
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let crates_line = workflow
        .lines()
        .find(|line| line.trim_start().starts_with("crates:"))
        .unwrap_or_else(|| {
            panic!(
                "{} must declare a `crates:` input for publish-crates",
                workflow_path.display()
            )
        });
    let crates: Vec<&str> = crates_line
        .split_once(':')
        .expect("crates: line has a value")
        .1
        .split_whitespace()
        .collect();

    let abi_index = crates
        .iter()
        .position(|&name| name == "alef-component-abi")
        .unwrap_or_else(|| panic!("publish-crates must publish alef-component-abi, got {crates:?}"));
    let runtime_index = crates
        .iter()
        .position(|&name| name == "alef-component-runtime")
        .unwrap_or_else(|| panic!("publish-crates must publish alef-component-runtime, got {crates:?}"));
    let alef_index = crates
        .iter()
        .position(|&name| name == "alef")
        .unwrap_or_else(|| panic!("publish-crates must publish alef, got {crates:?}"));

    assert!(
        abi_index < runtime_index,
        "alef-component-abi must publish before alef-component-runtime depends on it, got {crates:?}"
    );
    assert!(
        runtime_index < alef_index,
        "alef-component-runtime must publish before alef depends on it, got {crates:?}"
    );
}
