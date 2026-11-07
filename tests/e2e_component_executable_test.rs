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
    populate(directory.path(), build);
    directory
}

fn populate(root: &Path, build: &str) {
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

#[test]
fn an_out_path_with_a_space_and_a_quote_reaches_the_command_as_one_word() {
    let directory = tempfile::Builder::new().prefix("it's a dir ").tempdir().unwrap();
    populate(directory.path(), COPY);
    let root = directory.path();
    success(component(root, &["build", "--component", "tiny", "--target", HOST]));
    assert!(built(root, HOST, "tiny").is_file());
}

#[test]
fn a_failed_build_deletes_its_partial_output_and_reports_the_exit_status() {
    let directory = fixture("printf partial > {out}; exit 7");
    let root = directory.path();
    failure(
        component(root, &["build", "--component", "tiny", "--target", HOST]),
        "exit status: 7",
    );
    assert!(
        !built(root, HOST, "tiny").exists(),
        "a partial payload must not survive"
    );
}

#[test]
fn a_built_payload_is_made_executable() {
    use std::os::unix::fs::PermissionsExt;
    let directory = fixture("cp main.sh {out}");
    let root = directory.path();
    fs::set_permissions(root.join("exe/main.sh"), fs::Permissions::from_mode(0o644)).unwrap();
    success(component(root, &["build", "--component", "tiny", "--target", HOST]));
    let mode = fs::metadata(built(root, HOST, "tiny")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn a_missing_shell_stops_the_build_and_the_plan_early() {
    let directory = fixture(COPY);
    let root = directory.path();
    let empty = tempfile::tempdir().unwrap();
    for args in [
        vec!["build", "--component", "tiny", "--target", HOST],
        vec!["plan", "--component", "tiny", "--output", "plan.json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_alef"))
            .current_dir(root)
            .env("PATH", empty.path())
            .arg("component")
            .args(&args)
            .output()
            .unwrap();
        failure(output, "no `sh` was found on PATH");
    }
}

#[test]
fn the_fingerprint_does_not_depend_on_the_working_directory() {
    let directory = fixture(COPY);
    let root = directory.path();
    // Core crate paths stay relative to the working directory; only component paths and
    // `inputs` follow the config directory.
    fs::create_dir_all(root.join("work/src")).unwrap();
    fs::copy(root.join("src/lib.rs"), root.join("work/src/lib.rs")).unwrap();
    fs::copy(root.join("Cargo.toml"), root.join("work/Cargo.toml")).unwrap();
    let from_root = plan(root, "plan.json", None);
    let from_work = alef(
        &root.join("work"),
        &[
            "--config",
            "../alef.toml",
            "component",
            "plan",
            "--component",
            "tiny",
            "--output",
            "plan.json",
        ],
    );
    success(from_work);
    let from_work = json(&root.join("work"), "plan.json");
    assert_eq!(
        row(&from_root, HOST)["input_hash"],
        row(&from_work, HOST)["input_hash"],
        "labels must be rooted at the config directory, not the working directory"
    );
}

#[test]
fn a_producer_dir_containing_the_alef_build_directory_is_rejected() {
    let directory = fixture(COPY);
    let root = directory.path();
    let output = alef(
        &root.join("exe"),
        &["--config", "../alef.toml", "component", "plan", "--output", "plan.json"],
    );
    failure(output, "must not contain the alef build directory");
}

#[test]
fn the_alef_build_directory_is_not_an_input_of_a_broad_inputs_glob() {
    let directory = fixture(COPY);
    let root = directory.path();
    let alef_toml = fs::read_to_string(root.join("alef.toml")).unwrap();
    fs::write(
        root.join("alef.toml"),
        alef_toml.replace(
            "toolchain_version_cmd = \"cat ../tool.txt\" }",
            "toolchain_version_cmd = \"cat ../tool.txt\", inputs = [\"*\"] }",
        ),
    )
    .unwrap();
    let plans = tempfile::tempdir().unwrap();
    let first_path = plans.path().join("first.json");
    let second_path = plans.path().join("second.json");
    let plan_to = |path: &Path| {
        success(component(
            root,
            &["plan", "--component", "tiny", "--output", path.to_str().unwrap()],
        ));
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()
    };
    let before = plan_to(&first_path);
    success(component(root, &["build", "--component", "tiny", "--target", HOST]));
    assert!(root.join(".alef").is_dir());
    let after = plan_to(&second_path);
    assert_eq!(
        row(&before, HOST)["input_hash"],
        row(&after, HOST)["input_hash"],
        "a build must not change its own fingerprint"
    );
}

#[test]
fn a_toolchain_command_is_run_once_per_producer_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub trait Engine { fn run(&self) -> u32; }\n").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"dev-core\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let component = |name: &str| {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(root.join(name).join("main.sh"), "#!/bin/sh\n").unwrap();
        fs::write(root.join(name).join("toolchain.txt"), "go 1\n").unwrap();
        format!(
            r#"
[[crates.components]]
name = "{name}"
producer_language = "executable"
producer_dir = "{name}"
provides = [{{ contract = "engine", implementation = "New" }}]
targets = ["{HOST}"]
executable = {{ build = "cp main.sh {{out}}", toolchain_version_cmd = "cat toolchain.txt", inputs = ["{name}/main.sh"] }}
"#
        )
    };
    let components = format!("{}{}", component("one"), component("two"));
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
{components}
[crates.component_distribution]
url_template = "https://example.test/{{artifact}}"
"#
        ),
    )
    .unwrap();
    let hash_of_two = |output: &str| {
        success(component_cmd(root, output));
        let planned = json(root, output);
        planned["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["component"] == "two")
            .unwrap()["input_hash"]
            .clone()
    };
    let before = hash_of_two("plan1.json");
    fs::write(root.join("two/toolchain.txt"), "go 2\n").unwrap();
    let after = hash_of_two("plan2.json");
    assert_ne!(
        before, after,
        "`two` must see its own toolchain, not `one`'s cached answer"
    );
}

fn component_cmd(root: &Path, output: &str) -> Output {
    component(root, &["plan", "--output", output])
}
