// Test module: diagnostic output to stdout/stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

//! End-to-end proof of the downloadable-component host side: a real cdylib built by the real
//! producer codegen, packaged, loaded through a cache seeded with that packaged artifact (no
//! network download — see `package_and_seed_cache` below), activated into
//! `alef_component_abi`'s provider registry through a real, compiled copy of the generated
//! `ComponentProxy`, and called through the contract trait with bytes actually reversed by the
//! component.
//!
//! # Fidelity, and what is and isn't a real toolchain run here
//!
//! Steps 1-6 (build the fixture core crate + a real producer cdylib, package it, seed a local
//! `ArtifactCache` from the packaged archive via a `file://` URL, load and activate it, and call
//! through the trait) are a real, compiled, running proof: `cargo build`/`cargo run` really
//! execute, the manifest/digest verification in `alef_component_runtime::cache` really runs
//! against a real archive, and the byte reversal really happens inside a real dynamically
//! loaded library.
//!
//! `alef_component_activate`/`alef_component_register_providers` themselves
//! (`src/backends/native_components.rs`) are `pub(crate)`, so this integration test (a separate
//! crate) cannot call them directly. The harness below reimplements their handful of lines
//! (ensure the component, build the proxy, register it) against the same public building
//! blocks they call (`ComponentManager::ensure_contract`, `<Contract>Proxy::new`,
//! `alef_component_abi::register_provider`).
//! `component_proxy::generate_component_proxies` IS `pub`, so `CodecProxy` here is the real
//! generated output, compiled for real — this is the part that could not be faked by hand.
//!
//! Step 7 (Python/Go) is text-assertion on the real backend generators' output — the same
//! fidelity every other backend test in this repo uses (see `tests/backends_go_gen_bindings_test.rs`
//! and friends). No Python venv or Go module is actually built or run; that would be new,
//! unproven-in-this-repo infrastructure and is out of scope for this pass.

use alef::backends::ffi::FfiBackend;
use alef::backends::go::GoBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::codegen::component::{ResolvedComponent, entry_point_symbol, resolve_components};
use alef::codegen::component_producer::generate_component_producers;
use alef::codegen::component_proxy::generate_component_proxies;
use alef::component::artifact::{PackageInput, ProvidedContractInput, create_manifest, feature_hash, write_package};
use alef::core::backend::Backend;
use alef::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn core_cargo_toml() -> String {
    format!(
        r#"[package]
name = "demo-core"
version = "1.0.0"
edition = "2024"

[dependencies]
alef-component-abi = {{ path = {abi:?} }}
"#,
        abi = repo_root().join("crates/alef-component-abi"),
    )
}

const CORE_SOURCE: &str = r#"
pub trait Codec: Send + Sync {
    fn transform(&self, input: &[u8]) -> Result<Vec<u8>, Error>;
}

#[derive(Debug)]
pub struct Error(pub String);

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

/// Core-crate code that never sees a `LoadedComponent`: it only knows the contract trait and
/// looks up whatever the host activated for it.
pub fn transform(input: &[u8]) -> Result<Vec<u8>, Error> {
    alef_component_abi::provider::<dyn Codec>("codec")
        .ok_or_else(|| Error("codec component is not activated".to_string()))?
        .transform(input)
}
"#;

/// Appended to the generated producer's own `src/lib.rs`. `ComponentProvidesConfig::implementation`
/// is set to the bare name `ReverseCodec` (see `codec_config`) so the producer's
/// `type CodecComponentImplementation = ReverseCodec;` resolves this local type directly, with no
/// need to fake a separate `demo_components` crate.
const COMPONENT_IMPL_SOURCE: &str = r#"
#[derive(Default)]
struct ReverseCodec;

impl demo_core::Codec for ReverseCodec {
    fn transform(&self, input: &[u8]) -> Result<Vec<u8>, demo_core::Error> {
        Ok(input.iter().rev().copied().collect())
    }
}
"#;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(program: &str, args: &[&str], cwd: &Path) -> (bool, String) {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("running `{program} {}` in {}: {error}", args.join(" "), cwd.display()));
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The `ApiSurface` extracted from [`CORE_SOURCE`], built by hand rather than run through the
/// real extraction pipeline (a large, separate effort — see the module doc and the equivalent
/// hand-built fixtures in `src/codegen/component_proxy.rs`'s own unit tests).
fn codec_api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "demo_core".into(),
        version: "1.0.0".into(),
        types: vec![TypeDef {
            name: "Codec".into(),
            rust_path: "demo_core::Codec".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "transform".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Bytes,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("Error".into()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn codec_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "demo-core".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "codec".into(),
            trait_path: "demo_core::Codec".into(),
            interface_version: 1,
        }],
        components: vec![ComponentConfig {
            name: "reverse".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "codec".into(),
                implementation: "ReverseCodec".into(),
            }],
            features: vec![],
            default_features: true,
            targets: Some(vec!["test-target".into()]),
            bundled_on: Vec::new(),
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn find_built_library(target_dir: &Path) -> PathBuf {
    let release_dir = target_dir.join("release");
    fs::read_dir(&release_dir)
        .unwrap_or_else(|error| panic!("reading {}: {error}", release_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            name.contains("component")
                && matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("dylib" | "so" | "dll")
                )
        })
        .unwrap_or_else(|| panic!("no built component library found under {}", release_dir.display()))
}

/// Patch a dependency line the generator emits as a bare crates.io version requirement into a
/// local path dependency, for a crate this repo has not published (`alef-component-abi`). The
/// core crate path dependency needs no patch: `component_crate_dir` and `core_crate_dir` already
/// agree on `crates/{core}` and `crates/{core}-{component}-component` as siblings.
fn patch_unpublished_deps(manifest: &str) -> String {
    let unpatched = format!("alef-component-abi = \"{}\"", env!("CARGO_PKG_VERSION"));
    let patched = format!(
        "alef-component-abi = {{ path = {:?} }}",
        repo_root().join("crates/alef-component-abi")
    );
    manifest.replace(&unpatched, &patched)
}

/// Generate, patch, and build the real producer cdylib for `profile`.
fn build_producer_library(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) -> PathBuf {
    let mut files = generate_component_producers(api, config).expect("generate the producer crate");
    assert_eq!(files.len(), 3, "expected Cargo.toml, src/lib.rs, and the C header");

    let mut producer_dir = root.to_path_buf();
    for file in &mut files {
        if file.path.ends_with("Cargo.toml") {
            file.content = patch_unpublished_deps(&file.content);
        }
        if file.path.ends_with("src/lib.rs") {
            file.content.push_str(COMPONENT_IMPL_SOURCE);
        }
        let destination = root.join(&file.path);
        fs::create_dir_all(destination.parent().expect("generated file has a parent"))
            .expect("create producer output directory");
        if file.path.ends_with("Cargo.toml") {
            producer_dir = destination.parent().expect("Cargo.toml has a parent").to_path_buf();
        }
        fs::write(&destination, &file.content)
            .unwrap_or_else(|error| panic!("writing {}: {error}", destination.display()));
    }

    let (passed, output) = run(
        "cargo",
        &["build", "--release", "--manifest-path", "Cargo.toml"],
        &producer_dir,
    );
    assert!(passed, "building the generated component producer failed:\n{output}");

    find_built_library(&producer_dir.join("target"))
}

/// Package the built library exactly as `alef component package --unsigned` would, then seed a
/// local `ArtifactCache` from the resulting archive via a `file://` URL — the sanctioned local
/// substitute the cache already supports (`ArtifactCache::install` strips a `file://` prefix and
/// reads the archive directly), used here instead of a real HTTP download. This still exercises
/// the full hash/manifest verification chain in `alef_component_runtime::cache`.
///
/// Builds the manifest's `provides` list from every contract `component` provides (iterating,
/// not indexing `[0]`), even though this fixture only ever configures one.
fn package_and_seed_cache(
    root: &Path,
    library_path: &Path,
    component: &ResolvedComponent,
) -> (alef_component_runtime::ComponentLock, PathBuf) {
    let contract_hashes: Vec<String> = component
        .provides
        .iter()
        .map(|provided| hex_encode(&provided.contract_hash))
        .collect();
    let provides: Vec<ProvidedContractInput<'_>> = component
        .provides
        .iter()
        .zip(&contract_hashes)
        .map(|(provided, contract_hash)| ProvidedContractInput {
            contract: &provided.contract.name,
            interface_version: provided.contract.interface_version,
            contract_hash,
            implementation: &provided.implementation,
        })
        .collect();
    let feature_hash_hex = feature_hash(&component.features, component.default_features);
    let manifest = create_manifest(
        library_path,
        PackageInput {
            crate_name: "demo-core",
            component: &component.component_name,
            version: "1.0.0",
            target: "test-target",
            provides: &provides,
            features: &component.features,
            default_features: component.default_features,
            feature_hash: &feature_hash_hex,
        },
    )
    .expect("build the component manifest");

    let package_dir = root.join("package-output");
    let record = write_package(library_path, &package_dir, manifest.clone(), None).expect("write the unsigned package");
    let archive_path = package_dir.join(&record.archive);

    let cache_root = root.join("cache");
    fs::create_dir_all(&cache_root).expect("create cache root");
    let entry = alef_component_runtime::ComponentLockEntry {
        identity: manifest.identity.clone(),
        provides: manifest.provides.clone(),
        mode: alef_component_runtime::ComponentDeliveryMode::Download,
        url: format!("file://{}", archive_path.display()),
        sha256: record.archive_sha256.clone(),
        size: record.archive_size,
        manifest_sha256: record.manifest_sha256.clone(),
        key_id: String::new(),
    };
    let cache =
        alef_component_runtime::ArtifactCache::new(&cache_root, alef_component_runtime::TrustPolicy::DigestOnly);
    let installed = cache
        .install(&entry)
        .expect("seed the cache from the packaged archive and verify it");
    assert_eq!(installed.manifest.identity, manifest.identity);

    let lock = alef_component_runtime::ComponentLock {
        schema_version: alef_component_runtime::COMPONENT_MANIFEST_SCHEMA,
        public_keys: BTreeMap::new(),
        artifacts: vec![entry],
    };
    (lock, cache_root)
}

/// Write the harness crate: a real, compiled copy of the generated `CodecProxy`
/// (`component_proxy::generate_component_proxies`) plus a thin library that ensures the
/// component through a real `ComponentManager`, registers the proxy, and calls through the
/// trait. Two binaries (`positive`/`negative`) run in separate processes so the process-wide
/// `alef_component_abi` registry from one never contaminates the other.
fn write_harness(
    root: &Path,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lock_json: &str,
    cache_root: &Path,
) -> PathBuf {
    let resolved = resolve_components(api, config).expect("resolve the codec contract");
    let proxy_source = generate_component_proxies(config, &resolved).expect("generate the codec proxy");
    let entry_symbol = entry_point_symbol("codec");

    let harness_dir = root.join("crates/harness");
    fs::create_dir_all(harness_dir.join("src/bin")).expect("create harness crate directories");

    let manifest = format!(
        r#"[package]
name = "harness"
version = "0.1.0"
edition = "2024"

[dependencies]
demo-core = {{ path = "../demo-core" }}
alef-component-abi = {{ path = {abi:?} }}
alef-component-runtime = {{ path = {runtime:?} }}
serde_json = "1"
"#,
        abi = repo_root().join("crates/alef-component-abi"),
        runtime = repo_root().join("crates/alef-component-runtime"),
    );
    fs::write(harness_dir.join("Cargo.toml"), manifest).expect("write harness Cargo.toml");
    fs::write(harness_dir.join("components.lock.json"), lock_json).expect("write embedded lock");

    let lib_source = format!(
        r#"{proxy_source}

fn manager() -> alef_component_runtime::ComponentManager {{
    let lock: alef_component_runtime::ComponentLock =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/components.lock.json")))
            .expect("parse the embedded lock");
    alef_component_runtime::ComponentManager::from_lock(lock, {cache_root:?}, "test-target", host_api())
        .expect("build the component manager from the seeded lock")
}}

fn host_api() -> alef_component_abi::AlefHostApiV1 {{
    alef_component_abi::AlefHostApiV1 {{
        struct_size: 0,
        abi_major: 0,
        abi_minor: 0,
        context: std::ptr::null_mut(),
        log: None,
    }}
}}

pub fn activate_and_call(input: &[u8]) -> Result<Vec<u8>, String> {{
    let loaded = manager()
        .ensure_contract("reverse", "codec", b"{entry_symbol}\0")
        .map_err(|error| error.to_string())?;
    let proxy = std::sync::Arc::new(CodecProxy::new(&loaded)?);
    alef_component_abi::register_provider::<dyn demo_core::Codec>("codec", proxy);
    demo_core::transform(input).map_err(|error| error.to_string())
}}

pub fn call_without_activation(input: &[u8]) -> Result<Vec<u8>, String> {{
    demo_core::transform(input).map_err(|error| error.to_string())
}}
"#,
        cache_root = cache_root.display(),
    );
    fs::write(harness_dir.join("src/lib.rs"), lib_source).expect("write harness src/lib.rs");

    fs::write(
        harness_dir.join("src/bin/positive.rs"),
        r#"fn main() {
    let result = harness::activate_and_call(b"abc").expect("activated component call must succeed");
    assert_eq!(result, b"cba");
    println!("HARNESS_POSITIVE_OK");
}
"#,
    )
    .expect("write positive harness binary");

    fs::write(
        harness_dir.join("src/bin/negative.rs"),
        r#"fn main() {
    match harness::call_without_activation(b"abc") {
        Err(message) => {
            assert!(message.contains("not activated"), "unexpected message: {message}");
            println!("HARNESS_NEGATIVE_OK");
        }
        Ok(_) => panic!("expected an error before the component was activated"),
    }
}
"#,
    )
    .expect("write negative harness binary");

    harness_dir
}

#[test]
#[ignore = "builds and runs real cdylibs against the host toolchain; run via \
            `cargo test --test e2e_component_bytes_contract_test -- --ignored`"]
fn bytes_contract_round_trips_through_a_real_downloaded_component() {
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    let root = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());

    fs::create_dir_all(root.join("crates/demo-core/src")).expect("create core crate directories");
    fs::write(root.join("crates/demo-core/Cargo.toml"), core_cargo_toml()).expect("write core Cargo.toml");
    fs::write(root.join("crates/demo-core/src/lib.rs"), CORE_SOURCE).expect("write core source");

    let api = codec_api_surface();
    let config = codec_config();

    let library_path = build_producer_library(&root, &api, &config);

    let resolved = resolve_components(&api, &config).expect("resolve the codec contract");
    let component = &resolved[0];
    let (lock, cache_root) = package_and_seed_cache(&root, &library_path, component);
    let lock_json = serde_json::to_string(&lock).expect("serialize the lock");

    let harness_dir = write_harness(&root, &api, &config, &lock_json, &cache_root);

    let (build_passed, build_output) = run(
        "cargo",
        &["build", "--release", "--manifest-path", "Cargo.toml"],
        &harness_dir,
    );
    assert!(build_passed, "building the harness crate failed:\n{build_output}");

    let target_dir = harness_dir.join("target/release");
    let positive_bin = target_dir.join("positive");
    let negative_bin = target_dir.join("negative");

    let (positive_passed, positive_output) = run(&positive_bin.display().to_string(), &[], &harness_dir);
    assert!(positive_passed, "positive harness run failed:\n{positive_output}");
    assert!(
        positive_output.contains("HARNESS_POSITIVE_OK"),
        "positive harness did not report success:\n{positive_output}"
    );

    let (negative_passed, negative_output) = run(&negative_bin.display().to_string(), &[], &harness_dir);
    assert!(negative_passed, "negative harness run failed:\n{negative_output}");
    assert!(
        negative_output.contains("HARNESS_NEGATIVE_OK"),
        "negative harness did not report the expected failure:\n{negative_output}"
    );
}

#[test]
fn pyo3_and_go_backends_expose_component_activate() {
    let api = codec_api_surface();
    let config = codec_config();

    let pyo3_files = Pyo3Backend
        .generate_bindings(&api, &config)
        .expect("generate the pyo3 bindings");
    assert!(
        pyo3_files.iter().any(|file| file
            .content
            .contains("fn component_activate(py: Python<'_>, component: String)")),
        "no generated pyo3 file exposes component_activate"
    );

    let ffi_files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("generate the ffi bindings");
    assert!(
        ffi_files
            .iter()
            .any(|file| file.content.contains("pub unsafe extern \"C\" fn")
                && file
                    .content
                    .contains("_component_activate(component: *const c_char) -> i32")),
        "no generated ffi file exposes an extern \"C\" component_activate"
    );

    let go_files = GoBackend
        .generate_bindings(&api, &config)
        .expect("generate the go bindings");
    assert!(
        go_files
            .iter()
            .any(|file| file.content.contains("func ComponentActivate(component string) error")),
        "no generated go file exposes ComponentActivate"
    );
}
