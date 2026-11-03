#![cfg(unix)]

use alef_component_abi::{AlefContract, AlefHostApiV1};
use alef_component_runtime::{ComponentLock, ComponentManager, ComponentManifest, ComponentStatus};
use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Debug)]
struct TypedTable {
    method: unsafe extern "C" fn(),
}

// SAFETY: a repr(C) function table has a stable plain C layout for this test hash. ~keep
unsafe impl AlefContract for TypedTable {
    const CONTRACT_HASH: [u8; 32] = [0x11; 32];
}

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

fn manager(scenario: &str) -> (tempfile::TempDir, ComponentManager) {
    let dir = tempfile::tempdir().unwrap();
    let provides = json!([
        {"contract":"engine","interface_version":1,"contract_hash":"11".repeat(32),"implementation":"Engine"},
        {"contract":"second","interface_version":1,"contract_hash":"22".repeat(32),"implementation":"Second"}
    ]);
    let description = json!({
        "protocol_version":1,"abi_major":1,"abi_minor":0,"component_id":"demo","component_version":"1.0.0",
        "feature_set_hash":"33".repeat(32),"provides":provides
    });
    let executable = include_str!("stdio_sidecar/fixture.py")
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
        .with_sidecar_timeout(Duration::from_millis(300));
    (dir, manager)
}

#[test]
fn should_load_signed_sidecar_and_reuse_one_process_across_contracts() {
    let (_dir, manager) = manager("valid");
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::NotDownloaded);
    manager.prefetch(&["demo"]).unwrap();
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached);
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let second = manager.ensure_contract("demo", "second", b"unused\0").unwrap();
    assert_eq!(
        engine.contract_table::<TypedTable>().unwrap_err().code(),
        "unsupported_transport"
    );
    assert_eq!(
        engine.instantiate::<TypedTable>().err().unwrap().code(),
        "unsupported_transport"
    );
    assert!(engine.contract().is_null());
    assert_eq!(engine.contract_size(), 0);
    assert_eq!(engine.component_id(), "demo");
    assert_eq!(engine.component_version(), "1.0.0");
    assert_eq!(engine.descriptor().contract_hash, [0x11; 32]);
    assert_eq!(second.descriptor().contract_hash, [0x22; 32]);
    assert_eq!(engine.invoke_bytes("echo", b"\0hello\xff").unwrap(), b"\0hello\xff");
    assert_eq!(second.invoke_bytes("count", b"").unwrap(), b"2");
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Ready);
}

#[test]
fn should_reject_mismatching_handshake_before_becoming_ready() {
    for scenario in ["bad_hash", "bad_version", "bad_identity", "bad_feature"] {
        let (_dir, manager) = manager(scenario);
        assert!(
            manager.ensure_contract("demo", "engine", b"unused\0").is_err(),
            "{scenario}"
        );
        assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached);
    }
}

#[test]
fn should_return_status_error_without_losing_process() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let error = engine.invoke_bytes("failure", b"").unwrap_err().to_string();
    assert!(error.contains("3"), "{error}");
    assert!(error.contains("producer failed"), "{error}");
    let oversized = vec![0; alef_component_runtime::stdio::MAX_FRAME_SIZE + 1];
    assert_eq!(
        engine.invoke_bytes("echo", &oversized).unwrap_err().code(),
        "sidecar_protocol"
    );
    assert_eq!(engine.invoke_bytes("echo", b"still alive").unwrap(), b"still alive");
}

#[test]
fn should_bound_hanging_calls_and_reject_broken_frames() {
    for method in ["hang", "exit", "truncated", "oversized", "malformed"] {
        let (_dir, manager) = manager("valid");
        let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
        let start = Instant::now();
        assert!(engine.invoke_bytes(method, b"").is_err(), "{method}");
        assert!(start.elapsed() < Duration::from_secs(3), "{method}");
        assert!(engine.invoke_bytes("echo", b"after failure").is_err(), "{method}");
    }
}

#[test]
fn should_serialize_concurrent_calls_without_crossing_responses() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let threads: Vec<_> = (0..12)
        .map(|value| {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || {
                let payload = vec![value; 4096];
                assert_eq!(engine.invoke_bytes("echo", &payload).unwrap(), payload);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn should_reap_child_after_manager_and_last_component_are_dropped() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let pid = String::from_utf8(engine.invoke_bytes("pid", b"").unwrap()).unwrap();
    drop(manager);
    assert_eq!(engine.invoke_bytes("echo", b"retained").unwrap(), b"retained");
    drop(engine);
    let status = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "sidecar {pid} survived its final owner");
}
