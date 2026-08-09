use crate::manifest::{COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA};
use crate::{ComponentError, ComponentLockEntry, ComponentManifest, ComponentSignature};
use base64::Engine as _;
use ed25519_dalek::pkcs8::DecodePublicKey as _;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use flate2::read::GzDecoder;
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek as _, SeekFrom, Write as _};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub enum TrustPolicy {
    DigestOnly,
    EmbeddedKeys(BTreeMap<String, String>),
}

#[derive(Clone, Debug)]
pub struct ArtifactCache {
    root: PathBuf,
    trust: TrustPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedArtifact {
    pub root: PathBuf,
    pub library: PathBuf,
    pub manifest: ComponentManifest,
}

impl ArtifactCache {
    pub fn new(root: impl Into<PathBuf>, trust: TrustPolicy) -> Self {
        Self {
            root: root.into(),
            trust,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn object_path(&self, entry: &ComponentLockEntry) -> Result<PathBuf, ComponentError> {
        decode_digest(&entry.sha256)?;
        Ok(self.root.join("sha256").join(&entry.sha256))
    }

    pub fn install(&self, entry: &ComponentLockEntry) -> Result<CachedArtifact, ComponentError> {
        let destination = self.object_path(entry)?;
        if destination.is_dir()
            && let Ok(cached) = self.verify_installation(entry, &destination)
        {
            return Ok(cached);
        }
        if let Some(path) = entry.url.strip_prefix("file://") {
            return self.install_from_reader(entry, File::open(path)?);
        }
        let response = ureq::get(&entry.url)
            .header(
                "User-Agent",
                concat!("alef-component-runtime/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| ComponentError::Download {
                url: entry.url.clone(),
                message: error.to_string(),
            })?;
        self.install_from_reader(entry, response.into_body().into_reader())
    }

    pub fn install_from_reader(
        &self,
        entry: &ComponentLockEntry,
        mut reader: impl Read,
    ) -> Result<CachedArtifact, ComponentError> {
        let expected = decode_digest(&entry.sha256)?;
        let objects = self.root.join("sha256");
        fs::create_dir_all(&objects)?;
        let lock_path = objects.join(format!("{}.lock", entry.sha256));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;

        let destination = self.object_path(entry)?;
        if destination.is_dir()
            && let Ok(cached) = self.verify_installation(entry, &destination)
        {
            return Ok(cached);
        }

        let mut archive = tempfile::NamedTempFile::new_in(&objects)?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            archive.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            copied += count as u64;
            if copied > entry.size {
                return Err(ComponentError::SizeMismatch {
                    expected: entry.size,
                    actual: copied,
                });
            }
        }
        if entry.size != copied {
            return Err(ComponentError::SizeMismatch {
                expected: entry.size,
                actual: copied,
            });
        }
        let actual: [u8; 32] = hasher.finalize().into();
        if actual != expected {
            return Err(ComponentError::DigestMismatch {
                expected: entry.sha256.clone(),
                actual: hex::encode(actual),
            });
        }

        archive.as_file_mut().seek(SeekFrom::Start(0))?;
        let staging = tempfile::Builder::new().prefix(".install-").tempdir_in(&objects)?;
        extract_safely(archive.as_file_mut(), staging.path())?;
        let cached = self.verify_installation(entry, staging.path())?;

        if destination.exists() {
            fs::remove_dir_all(&destination)?;
        }
        let staging_path = staging.keep();
        if let Err(error) = fs::rename(&staging_path, &destination) {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(ComponentError::Io(error));
        }
        Ok(CachedArtifact {
            root: destination.clone(),
            library: destination.join(&cached.manifest.library.file),
            manifest: cached.manifest,
        })
    }

    fn verify_installation(&self, entry: &ComponentLockEntry, root: &Path) -> Result<CachedArtifact, ComponentError> {
        let manifest_bytes = fs::read(root.join("component.json"))?;
        if sha256(&manifest_bytes) != decode_digest(&entry.manifest_sha256)? {
            return Err(ComponentError::ManifestDigestMismatch);
        }
        let manifest: ComponentManifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.canonical_bytes()? != manifest_bytes {
            return Err(ComponentError::NonCanonicalManifest);
        }
        if manifest.schema_version != COMPONENT_MANIFEST_SCHEMA || manifest.abi_version != COMPONENT_ABI_VERSION {
            return Err(ComponentError::UnsupportedManifest);
        }
        if manifest.identity != entry.identity {
            return Err(ComponentError::ManifestIdentityMismatch);
        }
        let library_relative = safe_relative_path(&manifest.library.file)?;
        let library = root.join(&library_relative);
        let library_bytes = fs::read(&library).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ComponentError::MissingLibrary(library.clone())
            } else {
                ComponentError::Io(error)
            }
        })?;
        if library_bytes.len() as u64 != manifest.library.size {
            return Err(ComponentError::SizeMismatch {
                expected: manifest.library.size,
                actual: library_bytes.len() as u64,
            });
        }
        let expected_library = decode_digest(&manifest.library.sha256)?;
        let actual_library = sha256(&library_bytes);
        if expected_library != actual_library {
            return Err(ComponentError::DigestMismatch {
                expected: manifest.library.sha256.clone(),
                actual: hex::encode(actual_library),
            });
        }
        self.verify_manifest_signature(entry, root, &manifest_bytes)?;
        Ok(CachedArtifact {
            root: root.to_owned(),
            library,
            manifest,
        })
    }

    fn verify_manifest_signature(
        &self,
        entry: &ComponentLockEntry,
        root: &Path,
        manifest_bytes: &[u8],
    ) -> Result<(), ComponentError> {
        let TrustPolicy::EmbeddedKeys(keys) = &self.trust else {
            return Ok(());
        };
        if entry.key_id.is_empty() && keys.is_empty() {
            return Ok(());
        }
        let signature_bytes = fs::read(root.join("component.sig")).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ComponentError::SignatureRequired
            } else {
                ComponentError::Io(error)
            }
        })?;
        let signature: ComponentSignature = serde_json::from_slice(&signature_bytes)?;
        if signature.algorithm != "ed25519" {
            return Err(ComponentError::UnsupportedSignatureAlgorithm(signature.algorithm));
        }
        if signature.key_id != entry.key_id {
            return Err(ComponentError::SignatureKeyMismatch);
        }
        let encoded_key = keys
            .get(&signature.key_id)
            .ok_or_else(|| ComponentError::UnknownSignatureKey(signature.key_id.clone()))?;
        let key = decode_public_key(encoded_key)?;
        let raw_signature = base64::engine::general_purpose::STANDARD
            .decode(&signature.signature)
            .map_err(|_| ComponentError::InvalidSignatureEncoding)?;
        let signature = Signature::from_slice(&raw_signature).map_err(|_| ComponentError::InvalidSignatureEncoding)?;
        key.verify(manifest_bytes, &signature)
            .map_err(|_| ComponentError::SignatureVerification)
    }
}

fn decode_public_key(value: &str) -> Result<VerifyingKey, ComponentError> {
    if value.trim_start().starts_with("-----BEGIN") {
        return VerifyingKey::from_public_key_pem(value).map_err(|_| ComponentError::InvalidPublicKey);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|_| ComponentError::InvalidPublicKey)?;
    if let Ok(raw) = <[u8; 32]>::try_from(bytes.as_slice()) {
        return VerifyingKey::from_bytes(&raw).map_err(|_| ComponentError::InvalidPublicKey);
    }
    VerifyingKey::from_public_key_der(&bytes).map_err(|_| ComponentError::InvalidPublicKey)
}

fn decode_digest(value: &str) -> Result<[u8; 32], ComponentError> {
    let bytes = hex::decode(value).map_err(|_| ComponentError::InvalidDigest(value.to_owned()))?;
    bytes
        .try_into()
        .map_err(|_| ComponentError::InvalidDigest(value.to_owned()))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ComponentError> {
    let path = Path::new(value);
    let safe = !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if safe {
        Ok(path.to_owned())
    } else {
        Err(ComponentError::UnsafeLibraryPath(value.to_owned()))
    }
}

fn extract_safely(reader: impl Read, destination: &Path) -> Result<(), ComponentError> {
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.as_os_str().is_empty()
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            || !(entry.header().entry_type().is_file() || entry.header().entry_type().is_dir())
        {
            return Err(ComponentError::UnsafeArchiveEntry(path.display().to_string()));
        }
        if !entry.unpack_in(destination)? {
            return Err(ComponentError::UnsafeArchiveEntry(path.display().to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentIdentity, ComponentLibrary};
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::io::Cursor;

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
            schema_version: 1,
            abi_version: 1,
            identity: identity.clone(),
            contract: "engine".into(),
            contract_version: 1,
            implementation: "demo::Fast".into(),
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
            url: "unused".into(),
            sha256: hex::encode(sha256(&compressed)),
            size: compressed.len() as u64,
            manifest_sha256: hex::encode(sha256(&manifest_bytes)),
            key_id: signature.map(|value| value.key_id).unwrap_or_default(),
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
            base64::engine::general_purpose::STANDARD
                .encode(SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()),
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
}
