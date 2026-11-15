//! Manifest and signature checks on install: each failure names the check that failed.

use super::tests::{append, manifest_declaring_library_size, package, package_signed_by};
use super::*;
use ed25519_dalek::{Signer as _, SigningKey};
use std::io::Cursor;

fn signing() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn keys(id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(
        id.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(signing().verifying_key().to_bytes()),
    )])
}

fn signature(algorithm: &str, key_id: &str, manifest: &[u8]) -> ComponentSignature {
    ComponentSignature {
        algorithm: algorithm.into(),
        key_id: key_id.into(),
        signature: base64::engine::general_purpose::STANDARD.encode(signing().sign(manifest).to_bytes()),
    }
}

fn install(
    trust: TrustPolicy,
    (bytes, entry): (Vec<u8>, ComponentLockEntry),
) -> Result<CachedArtifact, ComponentError> {
    let dir = tempfile::tempdir().unwrap();
    ArtifactCache::new(dir.path(), trust).install_from_reader(&entry, Cursor::new(bytes))
}

#[test]
fn rejects_a_manifest_that_is_not_canonical_json() {
    let (_, entry) = package(None);
    let manifest = manifest_declaring_library_size(6);
    // The same document pretty-printed: valid JSON, and the lock pins exactly these bytes, but it
    // is not the canonical form the signature covers.
    let pretty = serde_json::to_vec_pretty(&manifest).unwrap();
    assert_ne!(pretty, manifest.canonical_bytes().unwrap());
    let mut compressed = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append(&mut builder, "component.json", &pretty);
        append(&mut builder, "libdemo.so", b"native");
        builder.into_inner().unwrap().finish().unwrap();
    }
    let entry = ComponentLockEntry {
        sha256: hex::encode(sha256(&compressed)),
        size: compressed.len() as u64,
        manifest_sha256: hex::encode(sha256(&pretty)),
        ..entry
    };
    assert!(matches!(
        install(TrustPolicy::DigestOnly, (compressed, entry)),
        Err(ComponentError::NonCanonicalManifest)
    ));
}

#[test]
fn rejects_a_signature_by_a_key_the_lock_does_not_embed() {
    let package = package_signed_by(b"native", |manifest| Some(signature("ed25519", "other", manifest)));
    assert!(matches!(
        install(TrustPolicy::EmbeddedKeys(keys("release")), package),
        Err(ComponentError::UnknownSignatureKey(key)) if key == "other"
    ));
}

#[test]
fn rejects_a_signature_whose_key_differs_from_the_lock_entry() {
    let (bytes, mut entry) = package_signed_by(b"native", |manifest| Some(signature("ed25519", "release", manifest)));
    entry.key_id = "rotated".into();
    let mut trusted = keys("release");
    trusted.extend(keys("rotated"));
    assert!(matches!(
        install(TrustPolicy::EmbeddedKeys(trusted), (bytes, entry)),
        Err(ComponentError::SignatureKeyMismatch)
    ));
}

#[test]
fn rejects_a_signature_algorithm_other_than_ed25519() {
    let package = package_signed_by(b"native", |manifest| Some(signature("rsa-pss", "release", manifest)));
    assert!(matches!(
        install(TrustPolicy::EmbeddedKeys(keys("release")), package),
        Err(ComponentError::UnsupportedSignatureAlgorithm(algorithm)) if algorithm == "rsa-pss"
    ));
}

/// With no keys in the lock and no `key_id` on the entry, the signature check is skipped
/// entirely (the default, digest-only policy): even an unparseable `component.sig` is ignored.
#[test]
fn an_unsigned_lock_skips_the_signature_check() {
    install(TrustPolicy::EmbeddedKeys(BTreeMap::new()), package(None)).unwrap();
    let garbage = package_signed_by(b"native", |_| {
        Some(ComponentSignature {
            algorithm: "garbage".into(),
            key_id: String::new(),
            signature: "not base64!".into(),
        })
    });
    install(TrustPolicy::EmbeddedKeys(BTreeMap::new()), garbage).unwrap();
}

#[test]
fn a_lock_with_keys_requires_a_signature_even_when_the_entry_names_none() {
    assert!(matches!(
        install(TrustPolicy::EmbeddedKeys(keys("release")), package(None)),
        Err(ComponentError::SignatureRequired)
    ));
}
