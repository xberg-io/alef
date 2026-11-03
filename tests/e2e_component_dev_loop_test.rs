use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use alef::component::artifact::{dynamic_library_name, sha256_bytes};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use serde_json::Value;

const LOCAL_TARGET: &str = "x86_64-unknown-linux-gnu";
const OTHER_TARGET: &str = "aarch64-apple-darwin";
const HOST_LOCK: &str = "crates/dev-core-ffi/components.lock.json";
const RELEASE_KEY_SEED: [u8; 32] = [7; 32];

fn fixture(with_keys: bool) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub trait Engine { fn run(&self) -> u32; }\n").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"dev-core\"\nversion = \"1.0.0\"\nedition = \"2024\"\n\n[features]\nfast = []\n",
    )
    .unwrap();
    let keys = if with_keys {
        let key = SigningKey::from_bytes(&RELEASE_KEY_SEED);
        let encoded = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
        format!("[crates.component_distribution.public_keys]\nrelease = {encoded:?}\n")
    } else {
        String::new()
    };
    fs::write(root.join("release.key"), RELEASE_KEY_SEED).unwrap();
    fs::write(root.join("library.so"), b"explicit library bytes").unwrap();
    fs::write(root.join("alef.toml"), fixture_config(&keys)).unwrap();
    directory
}

fn fixture_config(keys: &str) -> String {
    format!(
        r#"[workspace]
languages = ["ffi"]

[[crates]]
name = "dev-core"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[[crates.component_contracts]]
name = "engine"
trait_path = "dev_core::Engine"

[[crates.components]]
name = "fast"
provides = [{{ contract = "engine", implementation = "dev_components::Fast" }}]
features = ["fast"]
targets = ["{LOCAL_TARGET}", "{OTHER_TARGET}"]

[crates.component_distribution]
url_template = "https://example.test/{{artifact}}"
artifact_name_template = "dev-{{component}}-{{target}}.tar.gz"
{keys}"#
    )
}

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_alef"))
        .current_dir(root)
        .arg("component")
        .args(args)
        .output()
        .unwrap()
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

fn package(root: &Path, target: &str, signed: bool) {
    let mut args = vec![
        "package",
        "--component",
        "fast",
        "--target",
        target,
        "--library",
        "library.so",
    ];
    if signed {
        args.extend(["--signing-key", "release.key", "--key-id", "release"]);
    } else {
        args.push("--unsigned");
    }
    success(cli(root, &args));
}

fn lock(root: &Path, path: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(path)).unwrap()).unwrap()
}

fn assert_targets(lock: &Value, expected: &[&str]) {
    let artifacts = lock["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), expected.len());
    let targets = artifacts
        .iter()
        .map(|artifact| artifact["identity"]["target"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(targets, expected);
}

#[test]
fn filtered_unsigned_lock_emits_only_requested_target_and_default_requires_complete_matrix() {
    let directory = fixture(true);
    let root = directory.path();
    package(root, LOCAL_TARGET, false);
    failure(cli(root, &["lock", "--unsigned"]), "is missing");
    success(cli(
        root,
        &[
            "lock",
            "--unsigned",
            "--target",
            LOCAL_TARGET,
            "--output",
            "local.lock.json",
        ],
    ));
    assert_targets(&lock(root, "local.lock.json"), &[LOCAL_TARGET]);
    package(root, OTHER_TARGET, false);
    success(cli(root, &["lock", "--unsigned", "--target", LOCAL_TARGET]));
    assert_targets(&lock(root, "components.lock.json"), &[LOCAL_TARGET]);
    success(cli(root, &["lock", "--unsigned"]));
    assert_targets(&lock(root, "components.lock.json"), &[OTHER_TARGET, LOCAL_TARGET]);
}

#[test]
fn filtered_lock_rejects_unknown_and_unavailable_targets_without_writing_output() {
    let directory = fixture(true);
    let root = directory.path();
    package(root, LOCAL_TARGET, false);
    failure(
        cli(root, &["lock", "--unsigned", "--target", "unknown-target"]),
        "not configured",
    );
    failure(
        cli(root, &["lock", "--unsigned", "--target", OTHER_TARGET]),
        "is missing",
    );
    assert!(!root.join("components.lock.json").exists());
}

#[test]
fn explicit_unsigned_output_preserves_existing_host_locks_and_default_still_stages() {
    let directory = fixture(true);
    let root = directory.path();
    package(root, LOCAL_TARGET, false);
    package(root, OTHER_TARGET, false);
    let host = root.join(HOST_LOCK);
    fs::create_dir_all(host.parent().unwrap()).unwrap();
    fs::write(&host, b"committed signed host lock").unwrap();
    success(cli(root, &["lock", "--unsigned", "--output", "local.lock.json"]));
    assert_eq!(fs::read(&host).unwrap(), b"committed signed host lock");
    assert_targets(&lock(root, "local.lock.json"), &[OTHER_TARGET, LOCAL_TARGET]);
    success(cli(root, &["lock", "--unsigned"]));
    assert_eq!(
        fs::read(&host).unwrap(),
        fs::read(root.join("components.lock.json")).unwrap()
    );
}

#[test]
fn signed_lock_keeps_explicit_output_staging_and_target_signature_checks() {
    let directory = fixture(true);
    let root = directory.path();
    package(root, LOCAL_TARGET, false);
    failure(cli(root, &["lock", "--target", LOCAL_TARGET]), "unsigned");
    package(root, LOCAL_TARGET, true);
    package(root, OTHER_TARGET, true);
    success(cli(root, &["lock", "--output", "release.lock.json"]));
    let release = lock(root, "release.lock.json");
    assert_targets(&release, &[OTHER_TARGET, LOCAL_TARGET]);
    assert_eq!(release["public_keys"].as_object().unwrap().len(), 1);
    assert_eq!(
        fs::read(root.join(HOST_LOCK)).unwrap(),
        fs::read(root.join("release.lock.json")).unwrap()
    );
}

fn write_built_library(root: &Path, profile: &str, shared: bool, bytes: &[u8]) {
    let target_dir = if shared {
        "shared-target"
    } else {
        ".alef/components/build/dev-core/fast"
    };
    let path = root
        .join(target_dir)
        .join(LOCAL_TARGET)
        .join(profile)
        .join(dynamic_library_name("dev-core-fast-component", LOCAL_TARGET));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn package_debug_uses_debug_library_default_uses_release_and_override_wins() {
    let directory = fixture(true);
    let root = directory.path();
    write_built_library(root, "debug", false, b"debug library");
    write_built_library(root, "release", false, b"release library");
    success(cli(
        root,
        &["package", "--unsigned", "--target", LOCAL_TARGET, "--debug"],
    ));
    let record_path = format!("dist/components/dev-fast-{LOCAL_TARGET}.tar.gz.record.json");
    let debug_record = lock(root, &record_path);
    success(cli(root, &["package", "--unsigned", "--target", LOCAL_TARGET]));
    let release_record = lock(root, &record_path);
    assert_eq!(
        debug_record["manifest"]["library"]["sha256"],
        sha256_bytes(b"debug library")
    );
    assert_eq!(
        release_record["manifest"]["library"]["sha256"],
        sha256_bytes(b"release library")
    );
    success(cli(
        root,
        &[
            "package",
            "--unsigned",
            "--target",
            LOCAL_TARGET,
            "--debug",
            "--library",
            "library.so",
        ],
    ));
    let explicit_record = lock(root, &record_path);
    assert_eq!(
        explicit_record["manifest"]["library"]["sha256"],
        sha256_bytes(b"explicit library bytes")
    );
}

#[test]
fn package_debug_respects_shared_build_directory() {
    let directory = fixture(true);
    let root = directory.path();
    write_built_library(root, "debug", true, b"shared debug library");
    success(cli(
        root,
        &[
            "package",
            "--unsigned",
            "--target",
            LOCAL_TARGET,
            "--debug",
            "--target-dir",
            "shared-target",
        ],
    ));
    assert!(
        root.join(format!("dist/components/dev-fast-{LOCAL_TARGET}.tar.gz"))
            .exists()
    );
}

#[test]
fn keyless_distribution_supports_custom_artifact_names_only_with_explicit_unsigned_mode() {
    let directory = fixture(false);
    let root = directory.path();
    failure(
        cli(root, &["package", "--target", LOCAL_TARGET, "--library", "library.so"]),
        "must be signed",
    );
    package(root, LOCAL_TARGET, false);
    assert!(
        root.join(format!("dist/components/dev-fast-{LOCAL_TARGET}.tar.gz"))
            .exists()
    );
    failure(cli(root, &["lock", "--target", LOCAL_TARGET]), "unsigned");
    success(cli(
        root,
        &[
            "lock",
            "--unsigned",
            "--target",
            LOCAL_TARGET,
            "--output",
            "dev.lock.json",
        ],
    ));
    assert_eq!(lock(root, "dev.lock.json")["public_keys"], serde_json::json!({}));
    failure(
        cli(
            root,
            &[
                "package",
                "--target",
                LOCAL_TARGET,
                "--library",
                "library.so",
                "--signing-key",
                "release.key",
                "--key-id",
                "release",
            ],
        ),
        "not configured",
    );
}

#[test]
fn target_lock_skips_profiles_without_that_target_and_rejects_mixed_unknown_targets() {
    let directory = fixture(true);
    let root = directory.path();
    let config_path = root.join("alef.toml");
    let config = fs::read_to_string(&config_path).unwrap().replace(
        &format!("targets = [\"{LOCAL_TARGET}\", \"{OTHER_TARGET}\"]"),
        &format!("targets = [\"{LOCAL_TARGET}\"]"),
    );
    fs::write(
        config_path,
        format!(
            "{config}\n[[crates.components]]\nname = \"remote\"\nfeatures = [\"fast\"]\n\
             provides = [{{ contract = \"engine\", implementation = \"dev_components::Remote\" }}]\n\
             targets = [\"{OTHER_TARGET}\"]\n"
        ),
    )
    .unwrap();
    package(root, LOCAL_TARGET, false);
    success(cli(root, &["lock", "--unsigned", "--target", LOCAL_TARGET]));
    assert_targets(&lock(root, "components.lock.json"), &[LOCAL_TARGET]);
    failure(cli(root, &["lock", "--unsigned"]), "is missing");
    failure(
        cli(
            root,
            &[
                "lock",
                "--unsigned",
                "--target",
                &format!("{LOCAL_TARGET},unknown-target"),
            ],
        ),
        "not configured",
    );
}

#[test]
fn default_signed_lock_rejects_artifacts_with_unknown_key_ids() {
    let directory = fixture(true);
    let root = directory.path();
    package(root, LOCAL_TARGET, true);
    let config_path = root.join("alef.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace("release =", "another =");
    fs::write(config_path, config).unwrap();
    failure(
        cli(root, &["lock", "--target", LOCAL_TARGET]),
        "unknown signing key `release`",
    );
    assert!(!root.join("components.lock.json").exists());
}
