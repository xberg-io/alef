//! Signed-artifact scaffolding shared by the stdio sidecar suites: packages a scripted sidecar into
//! a signed, locked component and returns a manager that loads it over stdio.

use alef_component_abi::AlefHostApiV1;
use alef_component_runtime::{ComponentLock, ComponentManager, ComponentManifest};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::io::Cursor;
use std::time::Duration;

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, bytes: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o700);
    header.set_cksum();
    builder.append_data(&mut header, name, Cursor::new(bytes)).unwrap();
}

/// A manager for component `demo`, whose sidecar is `template` with `__DESCRIPTOR__` replaced by
/// the protocol-1 descriptor matching its manifest and `__SCENARIO__` by `scenario`.
pub fn manager(template: &str, scenario: &str) -> (tempfile::TempDir, ComponentManager) {
    let dir = tempfile::tempdir().unwrap();
    let provides = json!([
        {"contract":"engine","interface_version":1,"contract_hash":"11".repeat(32),"implementation":"Engine"},
        {"contract":"second","interface_version":1,"contract_hash":"22".repeat(32),"implementation":"Second"}
    ]);
    let description = json!({
        "protocol_version":1,"abi_major":1,"abi_minor":0,"component_id":"demo","component_version":"1.0.0",
        "feature_set_hash":"33".repeat(32),"provides":provides
    });
    let executable = template
        .replace("__DESCRIPTOR__", &description.to_string())
        .replace("__SCENARIO__", scenario);
    let identity = json!({
        "crate_name":"demo","component":"demo","version":"1.0.0","target":"test-target",
        "feature_hash":"33".repeat(32),"contract_hash":"11".repeat(32)
    });
    let manifest = json!({
        "schema_version":2,"abi_version":1,"identity":identity,"provides":provides,
        "features":[],"default_features":false,
        "library":{"file":"sidecar","sha256":digest(executable.as_bytes()),"size":executable.len()}
    });
    let manifest: ComponentManifest = serde_json::from_value(manifest).unwrap();
    let manifest_bytes = manifest.canonical_bytes().unwrap();
    let signing = SigningKey::from_bytes(&[7; 32]);
    let signature = json!({"algorithm":"ed25519","key_id":"test",
        "signature":base64::engine::general_purpose::STANDARD.encode(signing.sign(&manifest_bytes).to_bytes())});
    let mut builder = tar::Builder::new(Vec::new());
    append(&mut builder, "component.json", &manifest_bytes);
    append(&mut builder, "component.sig", &serde_json::to_vec(&signature).unwrap());
    append(&mut builder, "sidecar", executable.as_bytes());
    let tar = builder.into_inner().unwrap();
    let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut compressed, &tar).unwrap();
    let archive = compressed.finish().unwrap();
    let path = dir.path().join("component.tar.gz");
    std::fs::write(&path, &archive).unwrap();
    let lock: ComponentLock = serde_json::from_value(json!({"schema_version":2,
        "public_keys":{"test":base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes())},
        "artifacts":[{"identity":identity,"provides":provides,"mode":"download",
            "url":format!("file://{}",path.display()),"sha256":digest(&archive),"size":archive.len(),
            "manifest_sha256":digest(&manifest_bytes),"key_id":"test"}]}))
    .unwrap();
    let manager = ComponentManager::from_lock(lock, dir.path().join("cache"), "test-target", AlefHostApiV1::EMPTY)
        .unwrap()
        .with_stdio_component("demo")
        // Generous enough for the describe handshake to survive the python startups of a fully
        // parallel test run; tests that exercise the deadline themselves chain a tighter
        // `with_sidecar_timeout`.
        .with_sidecar_timeout(Duration::from_secs(2));
    (dir, manager)
}

pub fn is_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}
