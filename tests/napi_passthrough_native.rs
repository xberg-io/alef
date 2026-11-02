#[path = "support/mise_task.rs"]
mod mise_task;

use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig};
use std::path::Path;
use std::process::Output;

const CORE: &str = include_str!("fixtures/napi_passthrough/core.rs.txt");

fn write(root: &Path, relative: &Path, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
    std::fs::write(path, content).expect("fixture file");
}

fn prepare(root: &Path) {
    write(root, Path::new("crates/sample-core/src/lib.rs"), CORE);
    write(
        root,
        Path::new("crates/sample-core/Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n\
         [dependencies]\nserde={version='1',features=['derive']}\nserde_json='1'\n",
    );
    let source = root.join("crates/sample-core/src/lib.rs");
    let api = alef::extract::extractor::extract(&[&source], "sample_core", "0.1.0", None).expect("public extraction");
    assert_eq!(api.functions.len(), 9);
    assert_eq!(api.enums.len(), 3);
    let validation = alef::core::validation::validate_api_surface(&api);
    assert!(!validation.has_errors(), "{}", validation.format_errors());
    generate(root, &api);
    verify_core_dependency(root);
}

fn generate(root: &Path, api: &alef::core::ir::ApiSurface) {
    let raw: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=['node']\n[[crates]]\nname='sample-core'\nsources=['crates/sample-core/src/lib.rs']\n",
    )
    .expect("configuration");
    let mut config = raw.resolve().expect("resolved configuration").remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config
        .output_paths
        .insert("node".into(), root.join("crates/sample-core-node/src"));
    for file in alef::scaffold::scaffold(api, &config, &[Language::Node]).expect("canonical scaffold") {
        if file.path.starts_with("crates/sample-core-node")
            || file.path.starts_with(root.join("crates/sample-core-node"))
        {
            write(root, &file.path, &file.content);
        }
    }
    let bindings = NapiBackend.generate_bindings(api, &config).expect("complete bindings");
    assert_eq!(bindings.len(), 1);
    for file in bindings {
        write(root, &file.path, &file.content);
    }
    write(
        root,
        Path::new("crates/sample-core-node/build.rs"),
        "fn main() { napi_build::setup(); }\n",
    );
    let tasks = mise_task::fixture_config_with_tools(
        "[tasks.build]\nrun='cargo build --jobs 2'\n[tasks.probe]\nrun='node probe.cjs'\n",
        &["node"],
    );
    write(root, Path::new("crates/sample-core-node/mise.toml"), &tasks);
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

#[test]
fn scaffolded_passthrough_fixture_references_its_real_core() {
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
fn extracted_passthrough_enums_compile_and_reject_malformed_node_inputs() {
    let directory = tempfile::tempdir().expect("native fixture");
    prepare(directory.path());
    let binding = directory.path().join("crates/sample-core-node");
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
        include_str!("fixtures/napi_passthrough/probe.cjs"),
    );
    let output = mise_task::mise_run_without_lock(&binding, "probe")
        .output()
        .expect("Node runtime");
    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "passthrough enums: 18 checks passed\n"
    );
}
