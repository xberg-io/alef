use super::*;
use crate::{ComponentIdentity, ComponentLibrary};
use ed25519_dalek::{Signer as _, SigningKey};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn package(signing: Option<(&SigningKey, &str)>) -> (Vec<u8>, ComponentLockEntry) {
    let library = b"native";
    let identity = ComponentIdentity {
        crate_name: "demo-core".into(),
        component: "fast".into(),
        version: "1.0.0".into(),
        target: "test-target".into(),
        feature_hash: hex::encode([1; 32]),
        contract_hash: hex::encode([2; 32]),
    };
    let manifest = ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: 1,
        identity: identity.clone(),
        provides: vec![crate::ComponentProvidedContract {
            contract: "engine".into(),
            interface_version: 1,
            contract_hash: hex::encode([2; 32]),
            implementation: "demo::Fast".into(),
        }],
        features: vec!["fast".into()],
        default_features: false,
        library: ComponentLibrary {
            file: "libdemo.so".into(),
            sha256: hex::encode(sha256(library)),
            size: library.len() as u64,
        },
    };
    let manifest_bytes = manifest.canonical_bytes().unwrap();
    let signature = signing.map(|(key, id)| ComponentSignature {
        algorithm: "ed25519".into(),
        key_id: id.into(),
        signature: base64::engine::general_purpose::STANDARD.encode(key.sign(&manifest_bytes).to_bytes()),
    });
    let mut compressed = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append(&mut builder, "component.json", &manifest_bytes);
        if let Some(signature) = &signature {
            append(
                &mut builder,
                "component.sig",
                &crate::canonical_json(signature).unwrap(),
            );
        }
        append(&mut builder, "libdemo.so", library);
        builder.into_inner().unwrap().finish().unwrap();
    }
    let entry = ComponentLockEntry {
        identity,
        provides: manifest.provides.clone(),
        mode: crate::ComponentDeliveryMode::Download,
        url: "unused".into(),
        sha256: hex::encode(sha256(&compressed)),
        size: compressed.len() as u64,
        manifest_sha256: hex::encode(sha256(&manifest_bytes)),
        key_id: signature.map(|value| value.key_id).unwrap_or_default(),
        input_hash: None,
    };
    (compressed, entry)
}

fn append(builder: &mut tar::Builder<flate2::write::GzEncoder<&mut Vec<u8>>>, path: &str, content: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, path, Cursor::new(content)).unwrap();
}

#[test]
fn installs_and_verifies_full_trust_chain() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let (bytes, entry) = package(Some((&signing, "release")));
    let keys = BTreeMap::from([(
        "release".into(),
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes()),
    )]);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::EmbeddedKeys(keys));
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();
    assert_eq!(fs::read(installed.library).unwrap(), b"native");
    assert_eq!(installed.manifest.identity, entry.identity);
}

/// Regression for the padding disagreement between this loader, `alef`'s config
/// validation, and `alef`'s artifact verification: config accepted unpadded base64
/// keys while the loader accepted only standard-padded base64, so a key that passed
/// config validation and was baked into the lock would fail every load here.
#[test]
fn installs_and_verifies_full_trust_chain_with_unpadded_base64_key() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let (bytes, entry) = package(Some((&signing, "release")));
    let keys = BTreeMap::from([(
        "release".into(),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(signing.verifying_key().to_bytes()),
    )]);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::EmbeddedKeys(keys));
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();
    assert_eq!(fs::read(installed.library).unwrap(), b"native");
    assert_eq!(installed.manifest.identity, entry.identity);
}

#[test]
fn reuses_verified_cache_when_source_is_unavailable() {
    let (bytes, mut entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();
    entry.url = "file:///source/does/not/exist".into();

    let reused = cache.install(&entry).unwrap();
    assert_eq!(reused, installed);
}

#[test]
fn rejects_unsigned_archive_when_lock_names_a_key() {
    let (bytes, mut entry) = package(None);
    entry.key_id = "release".into();
    let keys = BTreeMap::from([(
        "release".into(),
        base64::engine::general_purpose::STANDARD.encode(SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()),
    )]);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::EmbeddedKeys(keys));
    assert!(matches!(
        cache.install_from_reader(&entry, Cursor::new(bytes)),
        Err(ComponentError::SignatureRequired)
    ));
}

#[test]
fn rejects_archive_digest_mismatch() {
    let (bytes, mut entry) = package(None);
    entry.sha256 = "00".repeat(32);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    assert!(matches!(
        cache.install_from_reader(&entry, Cursor::new(bytes)),
        Err(ComponentError::DigestMismatch { .. })
    ));
}

#[test]
fn stops_when_download_exceeds_pinned_size() {
    let (bytes, mut entry) = package(None);
    entry.size = 1;
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    assert!(matches!(
        cache.install_from_reader(&entry, Cursor::new(bytes)),
        Err(ComponentError::SizeMismatch { expected: 1, .. })
    ));
}

#[test]
fn rejects_invalid_signature_without_installing() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let wrong_key = SigningKey::from_bytes(&[8; 32]);
    let (bytes, entry) = package(Some((&signing, "release")));
    let keys = BTreeMap::from([(
        "release".into(),
        base64::engine::general_purpose::STANDARD.encode(wrong_key.verifying_key().to_bytes()),
    )]);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::EmbeddedKeys(keys));
    let destination = cache.object_path(&entry).unwrap();

    assert!(matches!(
        cache.install_from_reader(&entry, Cursor::new(bytes)),
        Err(ComponentError::SignatureVerification)
    ));
    assert!(!destination.exists());
}

#[test]
fn offline_mode_rejects_a_missing_artifact_without_a_network_attempt() {
    let (_bytes, mut entry) = package(None);
    entry.url = "https://example.invalid/component.tar.gz".into();
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).offline(true);

    let error = cache.install(&entry).unwrap_err();

    assert!(matches!(
        error,
        ComponentError::Offline { ref component, ref url }
            if component == "fast" && url == "https://example.invalid/component.tar.gz"
    ));
}

#[test]
fn offline_mode_still_reuses_an_already_verified_cache_entry() {
    let (bytes, entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();

    let offline_cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).offline(true);
    let reused = offline_cache.install(&entry).unwrap();

    assert_eq!(reused, installed);
}

#[test]
fn offline_mode_does_not_block_a_file_url() {
    let (bytes, mut entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.tar.gz");
    fs::write(&source, &bytes).unwrap();
    entry.url = format!("file://{}", source.display());

    let cache_root = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(cache_root.path(), TrustPolicy::DigestOnly).offline(true);

    assert!(cache.install(&entry).is_ok());
}

#[test]
fn rejects_parent_traversal_archive_entry() {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        header.as_mut_bytes()[..7].copy_from_slice(b"../evil");
        header.set_cksum();
        builder.append(&header, Cursor::new(b"x")).unwrap();
        builder.finish().unwrap();
    }
    let mut compressed = Vec::new();
    {
        let mut encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        extract_safely(Cursor::new(compressed), dir.path()),
        Err(ComponentError::UnsafeArchiveEntry(path)) if path == "../evil"
    ));
    assert!(!dir.path().parent().unwrap().join("evil").exists());
}

/// Yields at most `chunk` bytes per read so progress is reported several times.
struct ChunkedReader {
    data: Vec<u8>,
    position: usize,
    chunk: usize,
}

impl Read for ChunkedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let end = (self.position + self.chunk)
            .min(self.data.len())
            .min(self.position + buf.len());
        let slice = &self.data[self.position..end];
        buf[..slice.len()].copy_from_slice(slice);
        self.position = end;
        Ok(slice.len())
    }
}

#[test]
fn progress_reports_cumulative_bytes_against_the_pinned_total() {
    let (bytes, entry) = package(None);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let options = DownloadOptions::default().with_progress(move |done, total| sink.lock().unwrap().push((done, total)));
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_download_options(options);
    let reader = ChunkedReader {
        data: bytes,
        position: 0,
        chunk: 16,
    };

    cache.install_from_reader(&entry, reader).unwrap();

    let seen = seen.lock().unwrap();
    assert!(seen.len() > 1, "expected several progress calls, got {}", seen.len());
    assert!(seen.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(seen.iter().all(|&(_, total)| total == entry.size));
    assert_eq!(seen.last().copied(), Some((entry.size, entry.size)));
}

#[test]
fn cancel_flag_aborts_between_chunks_without_installing() {
    let (bytes, entry) = package(None);
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&cancel);
    let options = DownloadOptions::default()
        .with_cancel(Arc::clone(&cancel))
        .with_progress(move |_, _| trigger.store(true, std::sync::atomic::Ordering::Relaxed));
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_download_options(options);
    let destination = cache.object_path(&entry).unwrap();
    let reader = ChunkedReader {
        data: bytes,
        position: 0,
        chunk: 16,
    };

    let error = cache.install_from_reader(&entry, reader).unwrap_err();

    assert!(matches!(error, ComponentError::Cancelled));
    assert_eq!(error.code(), "cancelled");
    assert!(!destination.exists());
}

#[test]
fn already_cancelled_install_fails_before_any_network_attempt() {
    let (_bytes, mut entry) = package(None);
    entry.url = "https://example.invalid/component.tar.gz".into();
    let cancel = Arc::new(AtomicBool::new(true));
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly)
        .with_download_options(DownloadOptions::default().with_cancel(cancel));

    assert!(matches!(cache.install(&entry), Err(ComponentError::Cancelled)));
}

#[test]
fn url_override_replaces_the_lock_url_and_integrity_still_applies() {
    let (bytes, mut entry) = package(None);
    entry.url = "https://origin.invalid/component.tar.gz".into();
    let source = tempfile::tempdir().unwrap();
    let mirror = source.path().join("mirror.tar.gz");
    fs::write(&mirror, &bytes).unwrap();
    let target = format!("file://{}", mirror.display());

    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_url_override(move |_| target.clone());
    assert!(cache.install(&entry).is_ok());

    fs::write(&mirror, b"substituted content").unwrap();
    let other = tempfile::tempdir().unwrap();
    let target = format!("file://{}", mirror.display());
    let cache = ArtifactCache::new(other.path(), TrustPolicy::DigestOnly).with_url_override(move |_| target.clone());
    assert!(matches!(
        cache.install(&entry),
        Err(ComponentError::SizeMismatch { .. } | ComponentError::DigestMismatch { .. })
    ));
}

#[test]
fn install_stamps_last_use() {
    let (bytes, entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();

    let objects = cache.list_objects().unwrap();
    assert_eq!(objects.len(), 1);
    assert!(
        dir.path()
            .join("sha256")
            .join(format!("{}.used", entry.sha256))
            .is_file()
    );
}

fn set_mtime(path: &Path, offset_secs: u64) {
    let later = std::time::SystemTime::now() + Duration::from_secs(offset_secs);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(later)
        .unwrap();
}

#[test]
fn verified_stamp_is_off_by_default() {
    let (bytes, entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly);
    cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();
    assert!(!cache.stamp_path(&entry.sha256).exists());
}

#[test]
fn verified_stamp_lets_a_matching_library_skip_rehashing() {
    let (bytes, entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_verified_stamp(true);
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();
    assert!(cache.stamp_path(&entry.sha256).is_file());

    // A same-size edit that is re-stamped (what a local attacker with cache write access
    // could do) is trusted: documents the limit of the threat model.
    fs::write(&installed.library, b"nativf").unwrap();
    let stamp = stamp::capture(&entry.sha256, &installed.library).unwrap();
    stamp::write(&cache.stamp_path(&entry.sha256), &stamp);

    let reused = cache.install(&entry).unwrap();
    assert_eq!(fs::read(reused.library).unwrap(), b"nativf");
}

#[test]
fn verified_stamp_mismatch_forces_full_verification_and_repair() {
    let (bytes, mut entry) = package(None);
    let source = tempfile::tempdir().unwrap();
    let archive = source.path().join("component.tar.gz");
    fs::write(&archive, &bytes).unwrap();
    entry.url = format!("file://{}", archive.display());
    let dir = tempfile::tempdir().unwrap();
    let cache = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_verified_stamp(true);
    let installed = cache.install_from_reader(&entry, Cursor::new(bytes)).unwrap();

    fs::write(&installed.library, b"nativf").unwrap();
    set_mtime(&installed.library, 5);

    let repaired = cache.install(&entry).unwrap();
    assert_eq!(fs::read(repaired.library).unwrap(), b"native");
}

#[test]
fn full_verification_writes_the_stamp_for_a_pre_existing_install() {
    let (bytes, entry) = package(None);
    let dir = tempfile::tempdir().unwrap();
    ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly)
        .install_from_reader(&entry, Cursor::new(bytes))
        .unwrap();
    let stamping = ArtifactCache::new(dir.path(), TrustPolicy::DigestOnly).with_verified_stamp(true);
    assert!(!stamping.stamp_path(&entry.sha256).exists());

    stamping.install(&entry).unwrap();

    assert!(stamping.stamp_path(&entry.sha256).is_file());
}

fn manifest_declaring_library_size(size: u64) -> ComponentManifest {
    ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: 1,
        identity: ComponentIdentity {
            crate_name: "demo-core".into(),
            component: "fast".into(),
            version: "1.0.0".into(),
            target: "test-target".into(),
            feature_hash: hex::encode([1; 32]),
            contract_hash: hex::encode([2; 32]),
        },
        provides: vec![crate::ComponentProvidedContract {
            contract: "engine".into(),
            interface_version: 1,
            contract_hash: hex::encode([2; 32]),
            implementation: "demo::Fast".into(),
        }],
        features: vec!["fast".into()],
        default_features: false,
        library: ComponentLibrary {
            file: "libdemo.so".into(),
            sha256: hex::encode(sha256(b"native")),
            size,
        },
    }
}

/// A component archive can only ever grow to the size the manifest itself declares (plus
/// a small fixed headroom): a library entry that decompresses to far more than that is
/// rejected before the whole thing is buffered or written to disk, bounding how much a
/// crafted or corrupted archive can force this loader to decompress.
#[test]
fn rejects_library_entry_exceeding_the_manifest_declared_size() {
    let manifest = manifest_declaring_library_size(6);
    let manifest_bytes = manifest.canonical_bytes().unwrap();
    let oversized_library = vec![b'x'; (LIBRARY_SIZE_HEADROOM + 4096) as usize];

    let mut compressed = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append(&mut builder, "component.json", &manifest_bytes);
        append(&mut builder, "libdemo.so", &oversized_library);
        builder.into_inner().unwrap().finish().unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        extract_safely(Cursor::new(compressed), dir.path()),
        Err(ComponentError::ExtractedSizeExceeded { .. })
    ));
    assert!(!dir.path().join("libdemo.so").exists());
}

/// The library entry must be a regular file: a symlink (which could point anywhere on
/// the filesystem once `dlopen`ed) is rejected outright, never followed or written.
#[test]
fn rejects_symlink_library_entry() {
    let manifest = manifest_declaring_library_size(6);
    let manifest_bytes = manifest.canonical_bytes().unwrap();

    let mut compressed = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append(&mut builder, "component.json", &manifest_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_path("libdemo.so").unwrap();
        header.set_cksum();
        builder.append(&header, Cursor::new(&[][..])).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        extract_safely(Cursor::new(compressed), dir.path()),
        Err(ComponentError::UnsafeArchiveEntry(path)) if path == "libdemo.so"
    ));
    assert!(!dir.path().join("libdemo.so").exists());
}

/// A packaged component archive never has more than three entries (manifest, optional
/// signature, library); a fourth is not an archive this loader made.
#[test]
fn rejects_archive_with_too_many_entries() {
    let manifest = manifest_declaring_library_size(6);
    let manifest_bytes = manifest.canonical_bytes().unwrap();

    let mut compressed = Vec::new();
    {
        let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        append(&mut builder, "component.json", &manifest_bytes);
        append(&mut builder, "component.sig", b"{}");
        append(&mut builder, "libdemo.so", b"native");
        append(&mut builder, "unexpected.txt", b"extra");
        builder.into_inner().unwrap().finish().unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        extract_safely(Cursor::new(compressed), dir.path()),
        Err(ComponentError::TooManyArchiveEntries {
            max: MAX_ARCHIVE_ENTRIES,
            ..
        })
    ));
}
