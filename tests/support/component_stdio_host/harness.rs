use alef::codegen::component_host::generate_component_host;
use alef::component::artifact::sha256_file;
use alef::core::config::ResolvedCrateConfig;
use alef::core::ir::ApiSurface;
use alef_component_runtime::ComponentLock;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SELECTION: &str = ".with_stdio_component(\"reverse\")";
const NEGATIVE_EXIT: i32 = 42;
const EXPECTED_JOBS: usize = 3;
const EXPECTED_CALLS: usize = 2;

#[allow(
    clippy::print_stderr,
    reason = "retained proof must remain discoverable when a later assertion fails"
)]
pub fn proof_root() -> PathBuf {
    let root = match std::env::var_os("ALEF_STDIO_HOST_PROOF_DIR") {
        Some(path) => {
            let path = PathBuf::from(path);
            fs::create_dir(&path).expect("proof directory must be new; its parent must exist");
            path
        }
        None => tempfile::Builder::new()
            .prefix("alef-generated-stdio-host-")
            .tempdir()
            .expect("create retained proof directory")
            .keep(),
    };
    let root = root.canonicalize().expect("canonical proof root");
    fs::write(root.join("proof-location.txt"), root.display().to_string()).unwrap();
    eprintln!("generated stdio host proof: {}", root.display());
    root
}

pub fn write_driver(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig, lock: &ComponentLock) {
    let generated = generate_component_host(api, config)
        .expect("generate actual standalone host")
        .expect("configured host must generate one module");
    assert_eq!(generated.path, PathBuf::from("src/components_gen.rs"));
    assert_eq!(
        generated.content.matches(SELECTION).count(),
        1,
        "negative must remove exactly one selection"
    );
    let negative = generated.content.replacen(SELECTION, "", 1);
    assert!(!negative.contains("with_stdio_component"));
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join(&generated.path), &generated.content).unwrap();
    fs::write(root.join("src/components_negative.rs"), negative).unwrap();
    fs::write(root.join("src/main.rs"), DRIVER).unwrap();
    fs::write(root.join("Cargo.toml"), driver_manifest()).unwrap();
    fs::write(
        root.join("components.lock.json"),
        serde_json::to_vec_pretty(lock).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("input-hashes.json"),
        serde_json::to_vec_pretty(&source_hashes(root)).unwrap(),
    )
    .unwrap();
}

fn driver_manifest() -> String {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    format!(
        r#"[package]
name = "stdio-host-driver"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
alef-component-abi = {{ path = {abi:?} }}
alef-component-runtime = {{ path = {runtime:?} }}
serde_json = "1"
"#,
        abi = repository.join("crates/alef-component-abi"),
        runtime = repository.join("crates/alef-component-runtime")
    )
}

pub fn run_driver(root: &Path) -> Vec<Value> {
    assert!(
        !root.join("target").exists(),
        "standalone build must use a fresh private target"
    );
    let mut jobs = Vec::with_capacity(EXPECTED_JOBS);
    let mut build = Command::new("cargo");
    build
        .args(["build", "--offline", "--manifest-path", "Cargo.toml"])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .env_remove("CARGO_BUILD_TARGET");
    jobs.push(run_job(root, "build", &mut build));
    save_jobs(root, &jobs);
    assert_eq!(
        jobs[0]["exit_code"],
        0,
        "standalone generated host build failed: {}",
        root.display()
    );
    for scenario in ["positive", "negative"] {
        let mut command = Command::new(root.join("target/debug/stdio-host-driver"));
        command
            .arg(scenario)
            .arg(root.join(format!("cache-{scenario}")))
            .current_dir(root)
            .env("ALEF_STDIO_AUDIT", root.join("calls.jsonl"));
        jobs.push(run_job(root, scenario, &mut command));
        save_jobs(root, &jobs);
    }
    jobs
}

fn run_job(root: &Path, name: &str, command: &mut Command) -> Value {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{name} failed to spawn in {}: {error}", root.display()));
    fs::write(root.join(format!("{name}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{name}.stderr.log")), &output.stderr).unwrap();
    json!({"name":name,"exit_code":output.status.code(),
        "stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
}

fn save_jobs(root: &Path, jobs: &[Value]) {
    fs::write(root.join("jobs.json"), serde_json::to_vec_pretty(jobs).unwrap()).unwrap();
}

pub fn verify_and_record_proof(root: &Path, jobs: &[Value]) {
    assert_eq!(
        jobs.len(),
        EXPECTED_JOBS,
        "one build and two executed controls required"
    );
    assert_eq!(jobs[1]["name"], "positive");
    assert_eq!(
        jobs[1]["exit_code"],
        0,
        "positive failed; retained proof: {}",
        root.display()
    );
    assert_eq!(jobs[2]["name"], "negative");
    assert_eq!(
        jobs[2]["exit_code"], NEGATIVE_EXIT,
        "negative must fail for native loading only"
    );
    let positive: Value = serde_json::from_str(jobs[1]["stdout"].as_str().unwrap()).expect("positive result JSON");
    let negative: Value = serde_json::from_str(jobs[2]["stdout"].as_str().unwrap()).expect("negative result JSON");
    assert_eq!(positive, json!({"scenario":"positive","calls":2,"status":"ready"}));
    assert_eq!(
        negative,
        json!({"scenario":"negative","error_code":"library_load_failed"})
    );
    let observed_calls = verify_calls(root);
    let before: Value = serde_json::from_slice(&fs::read(root.join("input-hashes.json")).unwrap()).unwrap();
    let after = source_hashes(root);
    assert_eq!(before, after, "sources changed during standalone compilation");
    let proof = json!({"expected_jobs":EXPECTED_JOBS,"observed_jobs":jobs.len(),"jobs":jobs,
        "expected_calls":EXPECTED_CALLS,"observed_calls":observed_calls,"source_hashes":after,
        "driver_lock_sha256":sha256_file(&root.join("Cargo.lock")).unwrap()});
    fs::write(root.join("proof.json"), serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
}

fn verify_calls(root: &Path) -> usize {
    let audit = fs::read_to_string(root.join("calls.jsonl")).expect("actual sidecar call audit");
    let calls: Vec<Value> = audit.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(
        calls.len(),
        EXPECTED_CALLS,
        "negative control must not execute the sidecar"
    );
    for (call, payload) in calls.iter().zip([b"\0binary\xff".as_slice(), "héllo\0".as_bytes()]) {
        assert_eq!(
            call,
            &json!({"contract":"codec","method":"transform","payload":payload})
        );
    }
    calls.len()
}

fn source_hashes(root: &Path) -> Value {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<PathBuf> = [
        "src/components_gen.rs",
        "src/components_negative.rs",
        "src/main.rs",
        "Cargo.toml",
        "components.lock.json",
        "reverse-sidecar",
    ]
    .iter()
    .map(|path| root.join(path))
    .collect();
    sources.extend(
        [
            "crates/alef-component-runtime/src/lib.rs",
            "crates/alef-component-runtime/src/loader.rs",
            "crates/alef-component-runtime/src/manager.rs",
            "crates/alef-component-runtime/src/stdio/mod.rs",
            "crates/alef-component-runtime/src/stdio/process.rs",
            "crates/alef-component-abi/src/lib.rs",
        ]
        .iter()
        .map(|path| repository.join(path)),
    );
    let rows: Vec<_> = sources
        .iter()
        .map(|path| json!({"path":path,"sha256":sha256_file(path).unwrap()}))
        .collect();
    json!(rows)
}

const DRIVER: &str = r#"mod components_gen;
mod components_negative;

use alef_component_runtime::{ComponentError, ComponentStatus};
use std::path::PathBuf;

fn positive(cache: PathBuf) {
    assert_eq!(components_gen::COMPONENT_IDS, &["reverse"]);
    let manager = components_gen::component_manager(cache, false).expect("generated manager");
    assert_eq!(manager.status("reverse").unwrap(), ComponentStatus::NotDownloaded);
    let component = manager.ensure_contract("reverse", "codec", components_gen::CODEC_ENTRY_SYMBOL)
        .expect("generated stdio selection must load the executable");
    for payload in [b"\0binary\xff".as_slice(), "héllo\0".as_bytes()] {
        let expected: Vec<u8> = payload.iter().rev().copied().collect();
        assert_eq!(component.invoke_bytes("transform", payload).unwrap(), expected);
    }
    assert_eq!(manager.status("reverse").unwrap(), ComponentStatus::Ready);
    println!("{}", serde_json::json!({"scenario":"positive","calls":2,"status":"ready"}));
}

fn negative(cache: PathBuf) {
    let manager = components_negative::component_manager(cache, false).expect("negative generated manager");
    let error = manager.ensure_contract("reverse", "codec", components_negative::CODEC_ENTRY_SYMBOL)
        .err().expect("removing stdio selection must reject an executable as a native library");
    assert!(matches!(error, ComponentError::LibraryLoad { .. }), "wrong failure: {error}");
    assert_eq!(error.code(), "library_load_failed");
    println!("{}", serde_json::json!({"scenario":"negative","error_code":error.code()}));
    std::process::exit(42);
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let scenario = arguments.next().expect("positive or negative mode");
    let cache = PathBuf::from(arguments.next().expect("fresh cache root"));
    assert!(arguments.next().is_none());
    match scenario.as_str() {
        "positive" => positive(cache),
        "negative" => negative(cache),
        _ => panic!("unknown scenario: {scenario}"),
    }
}
"#;
