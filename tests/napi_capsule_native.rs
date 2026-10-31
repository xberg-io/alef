use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, TypeDef, TypeRef};
use alef::scaffold::scaffold;
use std::path::Path;
use std::process::{Command, Output};

#[path = "support/mise_task.rs"]
mod mise_task;

fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "sample-core"
sources = ["crates/sample-core/src/lib.rs"]
[crates.node.capsule_types.Handle]
type = "Handle"
from_module = "capsule-probe"
property_name = "__parser"
[crates.node.capsule_types.Handle.type_tag]
lower = "0x1"
upper = "0x2"
"#,
    )
    .expect("fixture configuration");
    let api = ApiSurface {
        crate_name: "sample-core".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Handle".into(),
            rust_path: "sample_core::Handle".into(),
            is_opaque: true,
            is_return_type: true,
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "get_handle".into(),
            rust_path: "sample_core::get_handle".into(),
            return_type: TypeRef::Named("Handle".into()),
            error_type: Some("std::io::Error".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    (api, config.resolve().expect("resolve configuration").remove(0))
}

fn write(root: &Path, relative: &Path, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directories");
    std::fs::write(path, content).expect("fixture file");
}

fn prepare(root: &Path) {
    let (api, config) = fixture();
    for file in scaffold(&api, &config, &[Language::Node]).expect("Node scaffold") {
        if file.path.starts_with("crates/sample-core-node") {
            write(root, &file.path, &file.content);
        }
    }
    let files = NapiBackend.generate_bindings(&api, &config).expect("Node bindings");
    assert_eq!(files.len(), 1, "exactly one generated native source");
    let source = format!("{}\n{}", files[0].content, include_str!("napi_capsule_native/probe.rs"));
    write(root, &files[0].path, &source);
    write(
        root,
        Path::new("crates/sample-core/src/lib.rs"),
        include_str!("napi_capsule_native/core.rs"),
    );
    write(
        root,
        Path::new("crates/sample-core/Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n",
    );
    write(
        root,
        Path::new("crates/sample-core-node/build.rs"),
        "fn main() { napi_build::setup(); }\n",
    );
    write(
        root,
        Path::new("crates/sample-core-node/mise.toml"),
        "[tasks.build]\nrun='cargo build --jobs 2'\n",
    );
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
#[ignore = "requires Cargo NAPI dependencies, Node and mise; run by the native capsule CI gate"]
fn generated_tagged_capsule_builds_and_loads_in_node() {
    let directory = tempfile::tempdir().expect("native fixture");
    prepare(directory.path());
    let binding = directory.path().join("crates/sample-core-node");
    let built = mise_task::mise_run(&binding, "build")
        .env_remove("CARGO_TARGET_DIR")
        .env("RUSTC_WRAPPER", "")
        .output()
        .expect("build native capsule");
    assert_success(&built);
    let library = format!(
        "target/debug/{}sample_core_node{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    std::fs::copy(binding.join(library), binding.join("index.node")).expect("loadable native addon");
    std::fs::write(binding.join("probe.cjs"), include_str!("napi_capsule_native/probe.cjs")).expect("Node probe");
    let node = Command::new("node")
        .arg("probe.cjs")
        .current_dir(&binding)
        .output()
        .expect("Node must be installed for the native capsule gate");
    assert_success(&node);
    assert_eq!(
        String::from_utf8_lossy(&node.stdout),
        "capsule native: 3 tag checks passed\n"
    );
}

#[test]
fn native_capsule_ci_gate_collects_the_runtime_test_and_installs_node() {
    let collected = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--list",
            "--ignored",
            "--exact",
            "generated_tagged_capsule_builds_and_loads_in_node",
        ])
        .output()
        .expect("list native capsule tests");
    assert_success(&collected);
    assert_eq!(
        String::from_utf8_lossy(&collected.stdout).replace("\r\n", "\n"),
        "generated_tagged_capsule_builds_and_loads_in_node: test\n\n1 test, 0 benchmarks\n"
    );
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../.github/workflows/ci.yml")).expect("CI workflow YAML");
    let steps = workflow["jobs"]["test"]["steps"].as_array().expect("test job steps");
    let gate: Vec<_> = steps
        .iter()
        .filter(|step| step["name"].as_str() == Some("Verify generated Node capsules load natively"))
        .collect();
    assert_eq!(gate.len(), 1, "exactly one native capsule gate");
    assert_eq!(
        gate[0]["run"].as_str(),
        Some(
            "mise run test:run -- --test napi_capsule_native -- --ignored --exact generated_tagged_capsule_builds_and_loads_in_node --nocapture"
        )
    );
    let setup = steps
        .iter()
        .find(|step| step["uses"].as_str() == Some("xberg-io/actions/setup-mise@v1"))
        .expect("mise tool setup");
    assert_eq!(
        setup["with"]["install-args"].as_str(),
        Some("${{ runner.os == 'Windows' && 'python uv node' || 'python uv bats node' }}")
    );
}
