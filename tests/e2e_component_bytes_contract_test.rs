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
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};
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
    fn describe(&self, request: Request, extra: Vec<u8>) -> Response;
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

/// A record with an optional field, so the wire format's `Option<T>`
/// encoding is exercised inside a `Record` rather than as a bare param.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub label: String,
    pub weight: Option<u32>,
}

/// A record returned by value, with a fieldless-enum field so `describe`
/// exercises the `Record` and `Enum` wire encodings on the result side too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub label: String,
    pub total_len: u32,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Empty,
    NonEmpty,
}

/// Core-crate code that never sees a `LoadedComponent`: it only knows the contract trait and
/// looks up whatever the host activated for it.
pub fn transform(input: &[u8]) -> Result<Vec<u8>, Error> {
    alef_component_abi::provider::<dyn Codec>("codec")
        .ok_or_else(|| Error("codec component is not activated".to_string()))?
        .transform(input)
}

pub fn describe(request: Request, extra: Vec<u8>) -> Result<Response, String> {
    Ok(alef_component_abi::provider::<dyn Codec>("codec")
        .ok_or_else(|| "codec component is not activated".to_string())?
        .describe(request, extra))
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

    fn describe(&self, request: demo_core::Request, extra: Vec<u8>) -> demo_core::Response {
        let total_len = request.label.len() as u32 + request.weight.unwrap_or(0) + extra.len() as u32;
        let kind = if extra.is_empty() { demo_core::Kind::Empty } else { demo_core::Kind::NonEmpty };
        demo_core::Response { label: request.label, total_len, kind }
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
        types: vec![
            TypeDef {
                name: "Codec".into(),
                rust_path: "demo_core::Codec".into(),
                is_trait: true,
                is_opaque: true,
                methods: vec![
                    MethodDef {
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
                    },
                    MethodDef {
                        name: "describe".into(),
                        params: vec![
                            ParamDef {
                                name: "request".into(),
                                ty: TypeRef::Named("Request".into()),
                                ..ParamDef::default()
                            },
                            ParamDef {
                                name: "extra".into(),
                                ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
                                ..ParamDef::default()
                            },
                        ],
                        return_type: TypeRef::Named("Response".into()),
                        receiver: Some(ReceiverKind::Ref),
                        ..MethodDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Request".into(),
                rust_path: "demo_core::Request".into(),
                fields: vec![
                    FieldDef {
                        name: "label".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "weight".into(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Response".into(),
                rust_path: "demo_core::Response".into(),
                fields: vec![
                    FieldDef {
                        name: "label".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "total_len".into(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "kind".into(),
                        ty: TypeRef::Named("Kind".into()),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
        ],
        enums: vec![EnumDef {
            name: "Kind".into(),
            rust_path: "demo_core::Kind".into(),
            variants: vec![
                EnumVariant {
                    name: "Empty".into(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "NonEmpty".into(),
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
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
///
/// `impl_source` is appended to the generated `src/lib.rs` verbatim: it supplies whatever
/// concrete implementation type the config's `provides.implementation` names.
fn build_producer_library(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig, impl_source: &str) -> PathBuf {
    let mut files = generate_component_producers(api, config).expect("generate the producer crate");
    assert_eq!(files.len(), 3, "expected Cargo.toml, src/lib.rs, and the C header");

    let mut producer_dir = root.to_path_buf();
    for file in &mut files {
        if file.path.ends_with("Cargo.toml") {
            file.content = patch_unpublished_deps(&file.content);
        }
        if file.path.ends_with("src/lib.rs") {
            file.content.push_str(impl_source);
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

fn activate() -> Result<(), String> {{
    let loaded = manager()
        .ensure_contract("reverse", "codec", b"{entry_symbol}\0")
        .map_err(|error| error.to_string())?;
    let proxy = std::sync::Arc::new(CodecProxy::new(&loaded)?);
    alef_component_abi::register_provider::<dyn demo_core::Codec>("codec", proxy);
    Ok(())
}}

pub fn activate_and_call(input: &[u8]) -> Result<Vec<u8>, String> {{
    activate()?;
    demo_core::transform(input).map_err(|error| error.to_string())
}}

/// Round-trips a record with an optional field plus a `Vec<u8>` slice
/// parameter through the real compiled `CodecProxy`, returning a record with
/// a nested fieldless-enum field -- proof that `Record`, `Optional`,
/// `Slice`, and `Enum` all marshal correctly end to end, not just scalars
/// and `Bytes`/`Utf8`.
pub fn activate_and_describe(label: &str, weight: Option<u32>, extra: Vec<u8>) -> Result<(String, u32, bool), String> {{
    activate()?;
    let request = demo_core::Request {{
        label: label.to_string(),
        weight,
    }};
    let response = demo_core::describe(request, extra)?;
    Ok((response.label, response.total_len, response.kind == demo_core::Kind::NonEmpty))
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

    let (label, total_len, non_empty) = harness::activate_and_describe("widget", Some(3), vec![1, 2, 3, 4])
        .expect("activated describe call must succeed");
    assert_eq!(label, "widget");
    assert_eq!(total_len, "widget".len() as u32 + 3 + 4);
    assert!(non_empty);

    let (empty_label, empty_total_len, empty_non_empty) = harness::activate_and_describe("solo", None, Vec::new())
        .expect("activated describe call with no optional weight must succeed");
    assert_eq!(empty_label, "solo");
    assert_eq!(empty_total_len, "solo".len() as u32);
    assert!(!empty_non_empty);
    println!("HARNESS_DESCRIBE_OK");
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

    let library_path = build_producer_library(&root, &api, &config, COMPONENT_IMPL_SOURCE);

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
    assert!(
        positive_output.contains("HARNESS_DESCRIBE_OK"),
        "positive harness did not report a successful record/optional/slice/enum round trip:\n{positive_output}"
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

// --- Async producer proof -------------------------------------------------
//
// The component proxy does not support async methods yet (`component_proxy`'s own
// `validate_proxy_methods` rejects them; widening it is a separate follow-up), so this cannot
// reuse `write_harness`/`ComponentManager` the way the bytes contract above does. Instead it
// loads the real producer cdylib through `alef_component_runtime::LoadedComponent` directly (the
// same public building block a generated proxy would use) and calls its async function pointer
// by hand, matching the `#[repr(C)]` shape `component_producer::rust_function_type` generates for
// an async method exactly. This proves the worker-pool `start`/`cancel`/`drop` wiring end to end:
// `AlefTaskV1::start` is genuinely populated (not `None`), calling it is what makes the shared
// worker pool actually run the call, and the typed completion callback fires with the real result.

const ASYNC_CORE_SOURCE: &str = r#"
pub trait Counter: Send + Sync {
    async fn increment(&self, amount: u32) -> u32;
}
"#;

const ASYNC_COMPONENT_IMPL_SOURCE: &str = r#"
#[derive(Default)]
struct AddOneCounter;

impl demo_async::Counter for AddOneCounter {
    async fn increment(&self, amount: u32) -> u32 {
        amount + 1
    }
}
"#;

fn counter_api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "demo_async".into(),
        version: "1.0.0".into(),
        types: vec![TypeDef {
            name: "Counter".into(),
            rust_path: "demo_async::Counter".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "increment".into(),
                params: vec![ParamDef {
                    name: "amount".into(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                receiver: Some(ReceiverKind::Ref),
                is_async: true,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn counter_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "demo-async".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "counter".into(),
            trait_path: "demo_async::Counter".into(),
            interface_version: 1,
        }],
        components: vec![ComponentConfig {
            name: "adder".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "counter".into(),
                implementation: "AddOneCounter".into(),
            }],
            features: vec![],
            default_features: true,
            targets: Some(vec!["test-target".into()]),
            bundled_on: Vec::new(),
        }],
        ..ResolvedCrateConfig::default()
    }
}

/// Mirrors the `#[repr(C)]` shape `component_producer::rust_function_type` generates for an
/// infallible async method whose result is a direct (non-buffer) scalar: `u32`, not
/// `AlefOwnedBuffer`, since `increment`'s result is a plain integer.
type CounterIncrementCompletion =
    unsafe extern "C" fn(*mut std::ffi::c_void, alef_component_abi::AlefStatus, u32, alef_component_abi::AlefOwnedBuffer);
type IncrementFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u32,
    CounterIncrementCompletion,
    *mut std::ffi::c_void,
    *mut alef_component_abi::AlefTaskV1,
) -> alef_component_abi::AlefStatus;

#[repr(C)]
struct CounterApiV1 {
    struct_size: usize,
    increment: Option<IncrementFn>,
}

unsafe extern "C" fn record_completion(
    context: *mut std::ffi::c_void,
    status: alef_component_abi::AlefStatus,
    result: u32,
    error: alef_component_abi::AlefOwnedBuffer,
) {
    // SAFETY: `context` was set to this sender's address just below, and the send happens
    // before the sender is dropped (the test blocks on `recv_timeout` first).
    let sender = unsafe { &*context.cast::<std::sync::mpsc::Sender<(i32, u32)>>() };
    let _ = sender.send((status.0, result));
    if !error.ptr.is_null() {
        // SAFETY: `error` is a real `AlefOwnedBuffer` the producer just handed us.
        drop(unsafe { alef_component_runtime::take_owned_bytes(error) });
    }
}

unsafe extern "C" fn ignore_generic_task_callback(
    _context: *mut std::ffi::c_void,
    _status: alef_component_abi::AlefStatus,
    _result: alef_component_abi::AlefOwnedBuffer,
    _error: alef_component_abi::AlefOwnedBuffer,
) {
    // `AlefTaskV1::start`'s generic callback is unused by this producer: the async wrapper
    // already captured the typed `record_completion` above at call time. `start` still requires
    // some `AlefTaskCallback` value to satisfy its C signature.
}

#[test]
#[ignore = "builds and runs a real cdylib against the host toolchain; run via \
            `cargo test --test e2e_component_bytes_contract_test -- --ignored`"]
fn async_task_round_trips_through_the_shared_worker_pool() {
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    let root = workspace
        .path()
        .canonicalize()
        .unwrap_or_else(|_| workspace.path().to_path_buf());

    fs::create_dir_all(root.join("crates/demo-async/src")).expect("create async core crate directories");
    fs::write(
        root.join("crates/demo-async/Cargo.toml"),
        format!(
            r#"[package]
name = "demo-async"
version = "1.0.0"
edition = "2024"

[dependencies]
alef-component-abi = {{ path = {abi:?} }}
"#,
            abi = repo_root().join("crates/alef-component-abi"),
        ),
    )
    .expect("write async core Cargo.toml");
    fs::write(root.join("crates/demo-async/src/lib.rs"), ASYNC_CORE_SOURCE).expect("write async core source");

    let api = counter_api_surface();
    let config = counter_config();
    let library_path = build_producer_library(&root, &api, &config, ASYNC_COMPONENT_IMPL_SOURCE);

    let resolved = resolve_components(&api, &config).expect("resolve the counter contract");
    let provided = &resolved[0].provides[0];
    let entry_symbol = entry_point_symbol("counter");

    let requirements = alef_component_runtime::ComponentRequirements {
        component_id: "adder".to_string(),
        contract_name: "counter".to_string(),
        contract_hash: provided.contract_hash,
        feature_set_hash: None,
    };
    let host = alef_component_abi::AlefHostApiV1 {
        struct_size: 0,
        abi_major: 0,
        abi_minor: 0,
        context: std::ptr::null_mut(),
        log: None,
    };
    let loaded = alef_component_runtime::LoadedComponent::load(
        &library_path,
        format!("{entry_symbol}\0").as_bytes(),
        &requirements,
        host,
    )
    .expect("load the real counter producer library");

    assert!(
        loaded.contract_size() >= std::mem::size_of::<CounterApiV1>(),
        "producer's contract table is smaller than the expected CounterApiV1 shape"
    );
    // SAFETY: `contract_size` was just checked against `CounterApiV1`'s own size, and the
    // producer's table shape is `component_producer::rust_function_type`'s output for one
    // infallible async `u32`-returning method, which `CounterApiV1` mirrors exactly.
    let table = unsafe { &*loaded.contract().cast::<CounterApiV1>() };
    let increment = table.increment.expect("generated table exposes `increment`");

    let create = loaded.descriptor().create.expect("descriptor exposes create");
    let destroy = loaded.descriptor().destroy.expect("descriptor exposes destroy");
    let mut instance = std::ptr::null_mut();
    let mut create_error = alef_component_abi::AlefOwnedBuffer::EMPTY;
    // SAFETY: `create` is the producer's real create function; `instance`/`create_error` are
    // valid out-parameters.
    let status = unsafe { create(loaded.host_api(), &mut instance, &mut create_error) };
    assert!(status.is_ok(), "creating the counter instance failed");

    let (sender, receiver) = std::sync::mpsc::channel::<(i32, u32)>();
    let mut out_task = std::mem::MaybeUninit::<alef_component_abi::AlefTaskV1>::uninit();
    // SAFETY: `instance` was just created above; `out_task` is a valid out-parameter; `sender`
    // outlives this call (it is not dropped until after `receiver.recv_timeout` returns below).
    let status = unsafe {
        increment(
            instance,
            41,
            record_completion,
            std::ptr::addr_of!(sender).cast_mut().cast(),
            out_task.as_mut_ptr(),
        )
    };
    assert_eq!(
        status,
        alef_component_abi::AlefStatus::PENDING,
        "increment must return PENDING and defer work to AlefTaskV1::start"
    );
    // SAFETY: `increment` returned PENDING, so it filled in `out_task`.
    let out_task = unsafe { out_task.assume_init() };
    assert!(
        out_task.start.is_some(),
        "AlefTaskV1::start must be populated by the shared-worker producer, not None"
    );

    // SAFETY: `out_task.context`/`start` were just produced by `increment`, above.
    let start_status =
        unsafe { (out_task.start.expect("checked above"))(out_task.context, ignore_generic_task_callback, std::ptr::null_mut()) };
    assert!(start_status.is_ok(), "starting the async task on the shared worker pool failed");

    let (status, result) = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the shared worker pool must deliver a completion");
    assert_eq!(status, alef_component_abi::AlefStatus::OK.0, "completion reported a non-OK status");
    assert_eq!(result, 42, "increment(41) must complete with 42");

    // SAFETY: `out_task.drop`/`destroy` are real function pointers from the same producer;
    // `out_task.context`/`instance` are each freed exactly once, here.
    unsafe {
        (out_task.drop.expect("drop is populated"))(out_task.context);
        destroy(instance);
    }
}

