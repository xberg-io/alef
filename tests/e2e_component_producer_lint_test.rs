//! Compiles a generated native producer under `cargo clippy -- -D warnings`.
//!
//! The generated `src/lib.rs` carries no crate- or module-level lint allowance, so every wire
//! shape it emits must be warning-free on its own: borrowed and owned string and byte arguments,
//! boolean and integer arguments and results, a `()` argument, fallible and infallible methods,
//! and async methods (the cancellation task, the worker thread, and borrowed arguments copied
//! before that thread starts). The bytes-contract lane
//! (`e2e_component_bytes_contract_test`) covers the runtime round trip; this lane covers breadth.

use alef::codegen::component_producer::generate_component_producers;
use alef::core::config::{ComponentConfig, ComponentContractConfig, ComponentProvidesConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CORE_SOURCE: &str = r#"
use std::future::Future;

pub trait Probe: Send + Sync {
    fn name(&self) -> String;
    fn check(&self, text: &str, data: Vec<u8>) -> Result<bool, Error>;
    fn count(&self, flag: bool) -> u32;
    fn warm(&self) -> impl Future<Output = Result<String, Error>> + Send;
    fn digest(&self, label: &str, data: &[u8]) -> impl Future<Output = u64> + Send;
    fn ping(&self, marker: ()) -> bool;
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}
"#;

const COMPONENT_IMPL_SOURCE: &str = r#"
#[derive(Default)]
struct DemoProbe;

impl demo_core::Probe for DemoProbe {
    fn name(&self) -> String {
        String::from("probe")
    }

    fn check(&self, text: &str, data: Vec<u8>) -> Result<bool, demo_core::Error> {
        if text.is_empty() {
            return Err(demo_core::Error(String::from("empty")));
        }
        Ok(text.len() == data.len())
    }

    fn count(&self, flag: bool) -> u32 {
        u32::from(flag)
    }

    async fn warm(&self) -> Result<String, demo_core::Error> {
        Ok(String::from("warm"))
    }

    async fn digest(&self, label: &str, data: &[u8]) -> u64 {
        (label.len() + data.len()) as u64
    }

    fn ping(&self, _marker: ()) -> bool {
        true
    }
}
"#;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, fallible: bool, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.into(),
        params,
        return_type,
        receiver: Some(ReceiverKind::Ref),
        error_type: fallible.then(|| "Error".into()),
        is_async,
        ..MethodDef::default()
    }
}

fn param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        ty,
        is_ref,
        ..ParamDef::default()
    }
}

fn probe_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo_core".into(),
        version: "1.0.0".into(),
        types: vec![TypeDef {
            name: "Probe".into(),
            rust_path: "demo_core::Probe".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![
                method("name", Vec::new(), TypeRef::String, false, false),
                method(
                    "check",
                    vec![
                        param("text", TypeRef::String, true),
                        param("data", TypeRef::Bytes, false),
                    ],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    true,
                    false,
                ),
                method(
                    "count",
                    vec![param("flag", TypeRef::Primitive(PrimitiveType::Bool), false)],
                    TypeRef::Primitive(PrimitiveType::U32),
                    false,
                    false,
                ),
                method("warm", Vec::new(), TypeRef::String, true, true),
                method(
                    "digest",
                    vec![
                        param("label", TypeRef::String, true),
                        param("data", TypeRef::Bytes, true),
                    ],
                    TypeRef::Primitive(PrimitiveType::U64),
                    false,
                    true,
                ),
                method(
                    "ping",
                    vec![param("marker", TypeRef::Unit, false)],
                    TypeRef::Primitive(PrimitiveType::Bool),
                    false,
                    false,
                ),
            ],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn probe_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "demo-core".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "probe".into(),
            trait_path: "demo_core::Probe".into(),
            interface_version: 1,
        }],
        components: vec![ComponentConfig {
            producer_language: Default::default(),
            executable: None,
            name: "probe".into(),
            provides: vec![ComponentProvidesConfig {
                contract: "probe".into(),
                implementation: "DemoProbe".into(),
            }],
            features: vec![],
            default_features: true,
            targets: Some(vec!["test-target".into()]),
            bundled_on: Vec::new(),
            source_manifest: None,
            producer_dir: None,
            extra_dependencies: Default::default(),
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn write_producer(root: &Path) -> PathBuf {
    let files = generate_component_producers(&probe_api(), &probe_config()).expect("generate the producer crate");
    let unpatched = format!("alef-component-abi = \"={}\"", env!("CARGO_PKG_VERSION"));
    let patched = format!(
        "alef-component-abi = {{ path = {:?} }}",
        repo_root().join("crates/alef-component-abi")
    );
    let mut producer_dir = None;
    for file in files {
        let destination = root.join(&file.path);
        fs::create_dir_all(destination.parent().expect("generated file has a parent")).expect("create directories");
        let mut content = file.content;
        if file.path.ends_with("Cargo.toml") {
            assert!(content.contains(&unpatched), "{content}");
            content = content.replace(&unpatched, &patched);
            producer_dir = destination.parent().map(Path::to_path_buf);
        }
        if file.path.ends_with("src/lib.rs") {
            assert!(!content.contains("allow("), "{content}");
            content.push_str(COMPONENT_IMPL_SOURCE);
        }
        fs::write(&destination, content).expect("write generated file");
    }
    producer_dir.expect("producer Cargo.toml was generated")
}

#[test]
#[ignore = "compiles a generated producer with cargo clippy against the host toolchain; run via \
            `cargo test --test e2e_component_producer_lint_test -- --ignored`"]
fn generated_producer_is_clippy_clean_without_lint_allowances() {
    let workspace = tempfile::tempdir().expect("create fixture workspace");
    let root = workspace.path();
    fs::create_dir_all(root.join("crates/demo-core/src")).expect("create core crate directories");
    fs::write(
        root.join("crates/demo-core/Cargo.toml"),
        "[package]\nname = \"demo-core\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write core Cargo.toml");
    fs::write(root.join("crates/demo-core/src/lib.rs"), CORE_SOURCE).expect("write core source");

    let producer_dir = write_producer(root);
    let output = Command::new("cargo")
        .args(["clippy", "--manifest-path", "Cargo.toml", "--", "-D", "warnings"])
        .current_dir(&producer_dir)
        .output()
        .expect("run cargo clippy");
    assert!(
        output.status.success(),
        "the generated producer is not clippy-clean under -D warnings:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
