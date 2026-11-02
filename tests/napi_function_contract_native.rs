#[path = "support/mise_task.rs"]
mod mise_task;

use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig};
use alef::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use std::path::Path;
use std::process::{Command, Output};

const CORE: &str = r#"
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub values: Option<Vec<String>>,
    pub fallbacks: Option<Option<Vec<String>>>,
}
#[derive(Clone)]
pub struct Child { pub value: String }
#[derive(Clone)]
pub struct Owner { pub child: std::sync::Arc<Child> }
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct MapPacket {
    pub flags: Vec<bool>,
    pub rows: Vec<std::collections::BTreeMap<String, serde_json::Value>>,
    pub maybe_rows: Option<Vec<std::collections::BTreeMap<String, serde_json::Value>>>,
}
pub fn echo_map_packet(value: MapPacket) -> MapPacket { value }
pub fn echo_owner(value: Owner) -> Owner { value }
pub fn json_owned(value: serde_json::Value) -> serde_json::Value { value }
pub fn json_borrowed(value: &serde_json::Value) -> &serde_json::Value { value }
pub fn json_with_packet(packet: Packet, value: &serde_json::Value) -> serde_json::Value {
    let _ = packet;
    value.clone()
}
pub fn echo_packet(value: Packet) -> Packet { value }
pub async fn consume_unit(value: String) { let _ = value; }
pub async fn consume_fallible(value: String) -> Result<(), String> {
    if value == "fail" { Err("core failure".into()) } else { Ok(()) }
}
pub async fn consume_promoted(first: Option<String>, second: String) { let _ = (first, second); }
pub fn consume_sync(value: String) { let _ = value; }
"#;

fn write(root: &Path, relative: &Path, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("fixture directories");
    std::fs::write(path, contents).expect("fixture file");
}

fn prepare(root: &Path) {
    let source = root.join("crates/sample-core/src/lib.rs");
    write(root, Path::new("crates/sample-core/src/lib.rs"), CORE);
    write(
        root,
        Path::new("crates/sample-core/Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n\
         [dependencies]\nserde={version='1',features=['derive']}\nserde_json='1'\n",
    );
    let api = alef::extract::extractor::extract(&[source.as_path()], "sample_core", "0.1.0", None)
        .expect("public extraction");
    assert_eq!(api.functions.len(), 10);
    assert_eq!(api.types.len(), 4);
    assert_map_packet_shape(&api);
    let validation = alef::core::validation::validate_api_surface(&api);
    assert!(!validation.has_errors(), "{}", validation.format_errors());
    let raw: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=['node']\n[[crates]]\nname='sample-core'\nsources=['crates/sample-core/src/lib.rs']\n",
    )
    .expect("configuration");
    let mut config = raw.resolve().expect("resolved configuration").remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config
        .output_paths
        .insert("node".into(), root.join("crates/sample-core-node/src"));
    for file in alef::scaffold::scaffold(&api, &config, &[Language::Node]).expect("canonical scaffold") {
        if file.path.starts_with("crates/sample-core-node")
            || file.path.starts_with(root.join("crates/sample-core-node"))
        {
            write(root, &file.path, &file.content);
        }
    }
    let bindings = NapiBackend
        .generate_bindings(&api, &config)
        .expect("complete binding source");
    assert_eq!(bindings.len(), 1);
    for file in bindings {
        write(root, &file.path, &file.content);
    }
    write(
        root,
        Path::new("crates/sample-core-node/build.rs"),
        "fn main() { napi_build::setup(); }\n",
    );
    let tasks = mise_task::fixture_config_with_tools("[tasks.build]\nrun='cargo build --jobs 2'\n", &["node"]);
    write(root, Path::new("crates/sample-core-node/mise.toml"), &tasks);
    verify_core_dependency(root);
}

fn verify_core_dependency(root: &Path) {
    let binding = root.join("crates/sample-core-node");
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(binding.join("Cargo.toml")).expect("generated native manifest"))
            .expect("valid generated manifest");
    let dependency = manifest["dependencies"]["sample-core"]["path"]
        .as_str()
        .expect("generated core dependency");
    assert_eq!(
        binding
            .join(dependency)
            .canonicalize()
            .expect("existing core dependency"),
        root.join("crates/sample-core").canonicalize().expect("fixture core"),
    );
}

fn assert_map_packet_shape(api: &ApiSurface) {
    let packet = api
        .types
        .iter()
        .find(|typ| typ.name == "MapPacket")
        .expect("public DTO");
    assert_eq!(packet.fields.len(), 3);
    assert_eq!(
        packet.fields[0].ty,
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::Bool)))
    );
    let rows = TypeRef::Vec(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Json),
    )));
    assert_eq!(packet.fields[1].ty, rows);
    assert_eq!(packet.fields[2].ty, rows);
    assert!(!packet.fields[1].optional);
    assert!(packet.fields[2].optional);
}

#[test]
fn scaffolded_function_fixture_references_its_real_core() {
    let directory = tempfile::tempdir().expect("scaffold fixture");
    prepare(directory.path());
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires cached Cargo NAPI dependencies, Node and mise"]
fn extracted_function_contracts_compile_and_execute_in_node() {
    let dir = tempfile::tempdir().expect("native fixture");
    prepare(dir.path());
    let binding = dir.path().join("crates/sample-core-node");
    let output = mise_task::mise_run_without_lock(&binding, "build")
        .env_remove("CARGO_TARGET_DIR")
        .env("RUSTC_WRAPPER", "")
        .output()
        .expect("native build");
    assert_success(&output);
    let library = format!(
        "target/debug/{}sample_core_node{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    std::fs::copy(binding.join(library), binding.join("index.node")).expect("native addon");
    write(
        &binding,
        Path::new("probe.cjs"),
        include_str!("fixtures/napi_function_contract.cjs"),
    );
    let node = Command::new("node")
        .arg("probe.cjs")
        .current_dir(&binding)
        .output()
        .expect("Node runtime");
    assert_success(&node);
    assert_eq!(
        String::from_utf8_lossy(&node.stdout),
        "function contracts: 23 checks passed\n"
    );
}
