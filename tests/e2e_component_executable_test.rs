//! End to end: an executable component planned, built, packaged and locked by the `alef` binary,
//! next to a Rust component in the same lock. The producer is a shell script, built by `cp`.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use base64::Engine as _;
use ed25519_dalek::SigningKey;
use serde_json::Value;

const KEY_SEED: [u8; 32] = [9; 32];

const HOST: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";

fn fixture(build: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("exe")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub trait Engine { fn run(&self) -> u32; }\n").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"dev-core\"\nversion = \"1.0.0\"\nedition = \"2024\"\n\n[features]\nfast = []\n",
    )
    .unwrap();
    fs::write(root.join("exe/main.sh"), "#!/bin/sh\necho tiny-sidecar v1\n").unwrap();
    fs::write(root.join("exe/go.mod"), "module tiny\n").unwrap();
    fs::write(root.join("tool.txt"), "toolchain 1\n").unwrap();
    fs::write(root.join("library.so"), b"rust component bytes").unwrap();
    fs::write(root.join("release.key"), KEY_SEED).unwrap();
    let public =
        base64::engine::general_purpose::STANDARD.encode(SigningKey::from_bytes(&KEY_SEED).verifying_key().as_bytes());
    fs::write(
        root.join("alef.toml"),
        format!(
            r#"[workspace]

[[crates]]
name = "dev-core"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[[crates.component_contracts]]
name = "engine"
trait_path = "dev_core::Engine"

[[crates.components]]
name = "tiny"
producer_language = "executable"
producer_dir = "exe"
provides = [{{ contract = "engine", implementation = "NewTiny" }}]
targets = ["{HOST}", "{WINDOWS}"]
executable = {{ build = {build:?}, toolchain_version_cmd = "cat ../tool.txt" }}

[[crates.components]]
name = "fast"
provides = [{{ contract = "engine", implementation = "dev_components::Fast" }}]
features = ["fast"]
targets = ["{HOST}"]

[crates.component_distribution]
url_template = "https://example.test/{{artifact}}"
artifact_name_template = "dev-{{component}}-{{target}}.tar.gz"

[crates.component_distribution.public_keys]
release = {public:?}
"#
        ),
    )
    .unwrap();
    directory
}

const COPY: &str = "cp main.sh {out} && chmod +x {out}";

fn alef(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_alef"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn component(root: &Path, args: &[&str]) -> Output {
    let mut full = vec!["component"];
    full.extend(args);
    alef(root, &full)
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn failure(output: Output, expected: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "expected {expected:?} in {stderr}");
}

fn json(root: &Path, path: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(path)).unwrap()).unwrap()
}

fn plan(root: &Path, output: &str, previous: Option<&str>) -> Value {
    let mut args = vec!["plan", "--component", "tiny", "--output", output];
    if let Some(previous) = previous {
        args.extend(["--previous-lock", previous]);
    }
    success(component(root, &args));
    json(root, output)
}

fn row<'a>(plan: &'a Value, target: &str) -> &'a Value {
    plan["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["target"] == target)
        .unwrap_or_else(|| panic!("no row for {target} in {plan}"))
}

fn built(root: &Path, target: &str, name: &str) -> std::path::PathBuf {
    root.join(".alef/components/build/dev-core/tiny")
        .join(target)
        .join("release")
        .join(name)
}

#[test]
fn plan_fingerprints_the_declared_inputs_and_toolchain() {
    let directory = fixture(COPY);
    let root = directory.path();
    let first = plan(root, "plan1.json", None);
    assert_eq!(first["rows"].as_array().unwrap().len(), 2);
    for target in [HOST, WINDOWS] {
        let row = row(&first, target);
        assert_eq!(row["action"], "build");
        let hash = row["input_hash"].as_str().unwrap();
        assert_eq!(hash.len(), 64, "{hash}");
    }
    assert_ne!(row(&first, HOST)["input_hash"], row(&first, WINDOWS)["input_hash"]);
    assert_eq!(first, plan(root, "plan2.json", None), "planning is deterministic");

    fs::write(root.join("exe/main.sh"), "#!/bin/sh\necho tiny-sidecar v2\n").unwrap();
    let edited = plan(root, "plan3.json", None);
    assert_ne!(
        row(&first, HOST)["input_hash"],
        row(&edited, HOST)["input_hash"],
        "source edit"
    );

    fs::write(root.join("exe/go.mod"), "module tiny2\n").unwrap();
    let remodule = plan(root, "plan4.json", None);
    assert_ne!(
        row(&edited, HOST)["input_hash"],
        row(&remodule, HOST)["input_hash"],
        "go.mod edit"
    );

    fs::write(root.join("tool.txt"), "toolchain 2\n").unwrap();
    let upgraded = plan(root, "plan5.json", None);
    assert_ne!(
        row(&remodule, HOST)["input_hash"],
        row(&upgraded, HOST)["input_hash"],
        "toolchain"
    );

    // An unrelated file outside the producer directory does not count.
    fs::write(root.join("README.md"), "docs").unwrap();
    assert_eq!(upgraded, plan(root, "plan6.json", None));
}

#[test]
fn build_package_and_lock_cover_rust_and_executable_components_in_one_lock() {
    let directory = fixture(COPY);
    let root = directory.path();
    let planned = plan(root, "plan.json", None);
    success(component(root, &["build", "--plan", "plan.json"]));

    // Cross-compilation is the build command's business: the windows payload is built on this host
    // and carries the `.exe` name.
    let host_exe = built(root, HOST, "tiny");
    let windows_exe = built(root, WINDOWS, "tiny.exe");
    assert!(host_exe.is_file() && windows_exe.is_file());
    let run = Command::new(&host_exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "tiny-sidecar v1\n");
    assert!(
        built(root, HOST, "tiny.input-hash").is_file(),
        "build --plan records its fingerprint"
    );

    let signing = ["--signing-key", "release.key", "--key-id", "release"];
    let mut args = vec!["package", "--plan", "plan.json", "--output", "dist"];
    args.extend(signing);
    success(component(root, &args));
    let mut args = vec![
        "package",
        "--component",
        "fast",
        "--target",
        HOST,
        "--library",
        "library.so",
        "--output",
        "dist",
    ];
    args.extend(signing);
    success(component(root, &args));
    success(component(root, &["lock", "--input", "dist", "--output", "lock.json"]));

    let lock = json(root, "lock.json");
    let mut entries = lock["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| {
            (
                artifact["identity"]["component"].as_str().unwrap().to_string(),
                artifact["identity"]["target"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        [
            ("fast".to_string(), HOST.to_string()),
            ("tiny".to_string(), WINDOWS.to_string()),
            ("tiny".to_string(), HOST.to_string()),
        ]
    );
    let tiny_windows = lock["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["identity"]["component"] == "tiny" && artifact["identity"]["target"] == WINDOWS)
        .unwrap();
    assert_eq!(tiny_windows["input_hash"], row(&planned, WINDOWS)["input_hash"]);
    for (target, payload) in [(WINDOWS, "tiny.exe"), (HOST, "tiny")] {
        let listing = Command::new("tar")
            .args(["-tzf"])
            .arg(root.join(format!("dist/dev-tiny-{target}.tar.gz")))
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&listing.stdout).to_string();
        assert!(
            listing.lines().any(|entry| entry.trim_start_matches("./") == payload),
            "{target} archive must carry `{payload}`: {listing}"
        );
    }
    success(component(root, &["verify", "--input", "dist"]));
}

#[test]
fn an_unchanged_executable_is_reused_and_a_changed_one_is_rebuilt() {
    let directory = fixture(COPY);
    let root = directory.path();
    plan(root, "plan.json", None);
    success(component(root, &["build", "--plan", "plan.json"]));
    success(component(
        root,
        &["package", "--plan", "plan.json", "--unsigned", "--output", "dist"],
    ));
    success(component(
        root,
        &[
            "lock",
            "--unsigned",
            "--component",
            "tiny",
            "--input",
            "dist",
            "--output",
            "previous.lock.json",
        ],
    ));

    let reused = plan(root, "reuse.json", Some("previous.lock.json"));
    for target in [HOST, WINDOWS] {
        assert_eq!(row(&reused, target)["action"], "reuse", "{reused}");
    }
    // `build --plan` skips reused rows, so nothing is rebuilt and no archive is made.
    fs::remove_dir_all(root.join(".alef")).unwrap();
    success(component(root, &["build", "--plan", "reuse.json"]));
    assert!(!built(root, HOST, "tiny").exists(), "a reused row must not be rebuilt");
    success(component(
        root,
        &[
            "lock",
            "--unsigned",
            "--component",
            "tiny",
            "--input",
            "absent",
            "--reuse",
            "previous.lock.json",
            "--plan",
            "reuse.json",
            "--output",
            "next.lock.json",
        ],
    ));
    assert_eq!(
        json(root, "previous.lock.json")["artifacts"],
        json(root, "next.lock.json")["artifacts"],
        "reuse carries the previous entries over verbatim"
    );

    fs::write(root.join("exe/main.sh"), "#!/bin/sh\necho tiny-sidecar v2\n").unwrap();
    let rebuilt = plan(root, "rebuild.json", Some("previous.lock.json"));
    for target in [HOST, WINDOWS] {
        assert_eq!(row(&rebuilt, target)["action"], "build", "{rebuilt}");
    }
    // The old plan no longer matches the sources: build refuses it.
    failure(
        component(root, &["build", "--plan", "reuse.json"]),
        "changed since the plan was made",
    );
}

#[test]
fn a_toolchain_command_that_fails_stops_planning() {
    let directory = fixture(COPY);
    let root = directory.path();
    fs::remove_file(root.join("tool.txt")).unwrap();
    failure(
        component(root, &["plan", "--component", "tiny", "--output", "plan.json"]),
        "toolchain_version_cmd",
    );
}

#[test]
fn a_build_that_writes_nothing_or_fails_is_an_error() {
    let directory = fixture("true # {out}");
    failure(
        component(directory.path(), &["build", "--component", "tiny", "--target", HOST]),
        "did not produce",
    );
    let directory = fixture("exit 7 # {out}");
    failure(
        component(directory.path(), &["build", "--component", "tiny", "--target", HOST]),
        "build command failed",
    );
}

#[test]
fn generate_and_verify_accept_an_executable_component_without_emitting_a_bridge() {
    let directory = fixture(COPY);
    let root = directory.path();
    let before = fs::read_dir(root.join("exe")).unwrap().count();
    success(alef(root, &["generate"]));
    assert_eq!(
        fs::read_dir(root.join("exe")).unwrap().count(),
        before,
        "no bridge in producer_dir"
    );
    assert!(
        !root.join("crates/dev-core-tiny-component").exists(),
        "no Rust producer crate for an executable component"
    );
    assert!(
        root.join("crates/dev-core-fast-component/Cargo.toml").is_file(),
        "the Rust component still generates"
    );
    success(alef(root, &["verify"]));
}
