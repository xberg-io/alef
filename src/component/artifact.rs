use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine;
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey};
use flate2::{Compression, GzBuilder};
use sha2::{Digest, Sha256};

pub use alef_component_runtime::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentDeliveryMode,
    ComponentIdentity, ComponentLibrary, ComponentLock, ComponentLockEntry, ComponentManifest,
    ComponentProvidedContract, ComponentSignature, canonical_json,
};

/// One contract a packaged component provides, as supplied to [`create_manifest`].
#[derive(Debug, Clone)]
pub struct ProvidedContractInput<'a> {
    pub contract: &'a str,
    pub interface_version: u32,
    pub contract_hash: &'a str,
    pub implementation: &'a str,
}

#[derive(Debug, Clone)]
pub struct PackageInput<'a> {
    pub crate_name: &'a str,
    pub component: &'a str,
    pub version: &'a str,
    pub target: &'a str,
    pub provides: &'a [ProvidedContractInput<'a>],
    pub features: &'a [String],
    pub default_features: bool,
    pub feature_hash: &'a str,
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// Hash a file in fixed-size chunks instead of reading it whole, so a large component
/// library never needs to be fully resident in memory just to be digested.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn feature_hash(features: &[String], default_features: bool) -> String {
    let mut normalized = features.to_vec();
    normalized.sort();
    normalized.dedup();
    let input = serde_json::json!({
        "default_features": default_features,
        "features": normalized,
    });
    sha256_bytes(&serde_json::to_vec(&input).expect("feature hash input is serializable"))
}

pub fn artifact_name(identity: &ComponentIdentity) -> Result<String> {
    for (kind, value) in [
        ("crate", identity.crate_name.as_str()),
        ("component", identity.component.as_str()),
        ("version", identity.version.as_str()),
        ("target", identity.target.as_str()),
    ] {
        validate_name(kind, value)?;
    }
    ensure!(
        identity.feature_hash.len() >= 16,
        "feature hash must contain at least 16 hexadecimal characters"
    );
    ensure!(
        identity.contract_hash.len() >= 16,
        "contract hash must contain at least 16 hexadecimal characters"
    );
    Ok(format!(
        "{}-{}-{}-{}-{}-{}.tar.gz",
        identity.crate_name,
        identity.component,
        identity.version,
        &identity.feature_hash[..16],
        &identity.contract_hash[..16],
        identity.target
    ))
}

fn validate_name(kind: &str, value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{kind} must not be empty");
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{kind} `{value}` contains characters that are unsafe in an artifact name"
    );
    // The charset check above allows a run of only `.` characters through, so `.` and `..`
    // -- otherwise-valid-looking path traversal components -- need an explicit reject.
    ensure!(
        value != "." && value != "..",
        "{kind} `{value}` is not a safe relative path component"
    );
    Ok(())
}

pub fn dynamic_library_name(crate_name: &str, target: &str) -> String {
    let stem = crate_name.replace('-', "_");
    if target.contains("windows") {
        format!("{stem}.dll")
    } else if target.contains("apple") || target.contains("darwin") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

pub fn create_manifest(library_path: &Path, input: PackageInput<'_>) -> Result<ComponentManifest> {
    ensure!(
        !input.provides.is_empty(),
        "component manifest must provide at least one contract"
    );
    let library_size = fs::metadata(library_path)
        .with_context(|| format!("failed to stat component library {}", library_path.display()))?
        .len();
    let library_sha256 = sha256_file(library_path)?;
    let library_file = library_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("component library filename is not valid UTF-8")?
        .to_string();
    validate_name("library filename", &library_file)?;

    let mut features = input.features.to_vec();
    features.sort();
    features.dedup();

    let mut provides = Vec::with_capacity(input.provides.len());
    let mut seen_contracts = std::collections::HashSet::new();
    for entry in input.provides {
        validate_name("contract", entry.contract)?;
        ensure!(
            seen_contracts.insert(entry.contract),
            "component manifest provides contract `{}` more than once",
            entry.contract
        );
        provides.push(ComponentProvidedContract {
            contract: entry.contract.to_string(),
            interface_version: entry.interface_version,
            contract_hash: entry.contract_hash.to_string(),
            implementation: entry.implementation.to_string(),
        });
    }
    provides.sort_by(|left, right| left.contract.cmp(&right.contract));
    let primary_contract_hash = provides[0].contract_hash.clone();

    let identity = ComponentIdentity {
        crate_name: input.crate_name.to_string(),
        component: input.component.to_string(),
        version: input.version.to_string(),
        target: input.target.to_string(),
        feature_hash: input.feature_hash.to_string(),
        contract_hash: primary_contract_hash,
    };
    Ok(ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: COMPONENT_ABI_VERSION,
        identity,
        provides,
        features,
        default_features: input.default_features,
        library: ComponentLibrary {
            file: library_file.clone(),
            sha256: library_sha256,
            size: library_size,
        },
    })
}

pub fn write_package(
    library_path: &Path,
    output_dir: &Path,
    manifest: ComponentManifest,
    signature: Option<ComponentSignature>,
) -> Result<ComponentArtifactRecord> {
    let library_file = library_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("component library filename is not valid UTF-8")?;
    ensure!(
        manifest.library.file == library_file,
        "component library filename changed while packaging"
    );
    if let Some(signature) = &signature {
        ensure!(
            signature.algorithm == "ed25519",
            "unsupported signature algorithm `{}`",
            signature.algorithm
        );
    }
    let archive = artifact_name(&manifest.identity)?;
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let signature_bytes = signature.as_ref().map(canonical_json).transpose()?;

    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create component output directory {}", output_dir.display()))?;
    let archive_path = output_dir.join(&archive);
    let (archive_size, archive_sha256) = write_archive(
        &archive_path,
        &manifest_bytes,
        signature_bytes.as_deref(),
        library_path,
        library_file,
        &manifest.library,
    )?;

    let record = ComponentArtifactRecord {
        manifest,
        manifest_sha256,
        archive,
        archive_sha256,
        archive_size,
        signature,
    };
    let record_path = record.record_path(output_dir);
    fs::write(&record_path, canonical_json(&record)?)
        .with_context(|| format!("failed to write component record {}", record_path.display()))?;
    Ok(record)
}

/// Write `component.json` [+ `component.sig`] and the component library as a gzip-compressed
/// tar archive, streaming the (potentially large) library through the writer in fixed-size
/// chunks instead of buffering it in memory, and hashing the compressed output as it is
/// written instead of reading the archive back afterward. The gzip header pins `mtime` to 0
/// (and lets the encoder's already-deterministic default `OS = unknown` stand) so that
/// packaging the same inputs twice produces byte-identical archives.
fn write_archive(
    archive_path: &Path,
    manifest_bytes: &[u8],
    signature_bytes: Option<&[u8]>,
    library_path: &Path,
    library_file: &str,
    library: &ComponentLibrary,
) -> Result<(u64, String)> {
    let archive_file = fs::File::create(archive_path)
        .with_context(|| format!("failed to create component archive {}", archive_path.display()))?;
    let hashing_output = HashingWriter::new(archive_file);
    let gzip = GzBuilder::new().mtime(0).write(hashing_output, Compression::default());
    let mut tar = tar::Builder::new(gzip);

    append_tar_bytes(&mut tar, "component.json", manifest_bytes, 0o644)?;
    if let Some(bytes) = signature_bytes {
        append_tar_bytes(&mut tar, "component.sig", bytes, 0o644)?;
    }

    let mut library_handle = fs::File::open(library_path)
        .with_context(|| format!("failed to open component library {}", library_path.display()))?;
    let library_len = library_handle
        .metadata()
        .with_context(|| format!("failed to stat component library {}", library_path.display()))?
        .len();
    ensure!(
        library.size == library_len,
        "component library size changed while packaging"
    );
    let mut hashing_library = HashingReader::new(&mut library_handle);
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o755);
    header.set_mtime(0);
    header.set_size(library_len);
    tar.append_data(&mut header, library_file, &mut hashing_library)
        .with_context(|| format!("failed to append component library `{library_file}` to the archive"))?;
    let (library_bytes_streamed, library_sha256) = hashing_library.finish();
    ensure!(
        library_bytes_streamed == library.size,
        "component library changed size while streaming into the archive"
    );
    ensure!(
        library_sha256 == library.sha256,
        "component library changed while streaming into the archive"
    );

    let gzip = tar
        .into_inner()
        .context("failed to finalize the component archive's tar stream")?;
    let hashing_output = gzip
        .finish()
        .context("failed to finalize the component archive's gzip stream")?;
    Ok(hashing_output.finish())
}

fn append_tar_bytes(builder: &mut tar::Builder<impl Write>, name: &str, content: &[u8], mode: u32) -> Result<()> {
    validate_name("tar entry", name)?;
    let mut header = tar::Header::new_gnu();
    header.set_mode(mode);
    header.set_mtime(0);
    header.set_size(content.len() as u64);
    builder
        .append_data(&mut header, name, content)
        .with_context(|| format!("failed to append `{name}` to the component archive"))
}

/// A [`Read`] wrapper that hashes bytes as they pass through, so a source (here, a component
/// library on disk) only needs to be streamed once to be both archived and digest-verified.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R: Read> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes_read: 0,
        }
    }

    fn finish(self) -> (u64, String) {
        (self.bytes_read, hex_encode(&self.hasher.finalize()))
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..count]);
        self.bytes_read += count as u64;
        Ok(count)
    }
}

/// A [`Write`] wrapper that hashes and counts bytes as they are written, so the archive this
/// module produces is digested once while it is written instead of being read back afterward.
struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    bytes_written: u64,
}

impl<W> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes_written: 0,
        }
    }

    fn finish(self) -> (u64, String) {
        (self.bytes_written, hex_encode(&self.hasher.finalize()))
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..count]);
        self.bytes_written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub fn read_record(path: &Path) -> Result<ComponentArtifactRecord> {
    let bytes = fs::read(path).with_context(|| format!("failed to read component record {}", path.display()))?;
    let record: ComponentArtifactRecord = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse component record {}", path.display()))?;
    Ok(record)
}

pub fn build_lock(
    records: &[ComponentArtifactRecord],
    url_template: &str,
    public_keys: &BTreeMap<String, String>,
) -> Result<ComponentLock> {
    let mut artifacts = Vec::with_capacity(records.len());
    for record in records {
        let signature = record
            .signature
            .as_ref()
            .with_context(|| format!("component artifact `{}` is unsigned", record.archive))?;
        ensure!(
            public_keys.contains_key(&signature.key_id),
            "component artifact `{}` uses unknown signing key `{}`",
            record.archive,
            signature.key_id
        );
        let identity = &record.manifest.identity;
        let url = expand_url(url_template, identity, &record.archive)?;
        artifacts.push(ComponentLockEntry {
            identity: identity.clone(),
            provides: record.manifest.provides.clone(),
            // No backend builds or locks a bundled artifact yet -- every artifact
            // this command packages and locks today is downloaded on demand.
            mode: ComponentDeliveryMode::Download,
            url,
            sha256: record.archive_sha256.clone(),
            size: record.archive_size,
            manifest_sha256: record.manifest_sha256.clone(),
            key_id: signature.key_id.clone(),
        });
    }
    artifacts.sort_by(|left, right| {
        (
            &left.identity.crate_name,
            &left.identity.component,
            &left.identity.version,
            &left.identity.target,
            &left.identity.feature_hash,
        )
            .cmp(&(
                &right.identity.crate_name,
                &right.identity.component,
                &right.identity.version,
                &right.identity.target,
                &right.identity.feature_hash,
            ))
    });
    for pair in artifacts.windows(2) {
        ensure!(
            pair[0].identity != pair[1].identity,
            "duplicate component artifact identity in lock input"
        );
    }
    Ok(ComponentLock {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        public_keys: public_keys.clone(),
        artifacts,
    })
}

pub fn write_lock(path: &Path, lock: &ComponentLock) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, canonical_json(lock)?).with_context(|| format!("failed to write component lock {}", path.display()))
}

/// `write_package` always produces at most three archive entries: the manifest, an optional
/// signature, and the library. Anything else is not an archive this crate wrote.
const MAX_ARCHIVE_ENTRIES: usize = 3;
/// Generous ceiling for the manifest/signature entries -- both are small JSON documents,
/// orders of magnitude smaller than any native component library, so a declared size above
/// this is already a sign the archive is not what it claims to be.
const METADATA_ENTRY_SIZE_CEILING: u64 = 64 * 1024;

pub fn verify_record(record_path: &Path, public_keys: &BTreeMap<String, String>) -> Result<()> {
    let record = read_record(record_path)?;
    ensure!(
        record.manifest.schema_version == COMPONENT_MANIFEST_SCHEMA,
        "unsupported component manifest schema"
    );
    ensure!(
        record.manifest.abi_version == COMPONENT_ABI_VERSION,
        "unsupported component ABI version"
    );
    ensure!(
        record.manifest_sha256 == sha256_bytes(&record.manifest.canonical_bytes()?),
        "manifest hash mismatch"
    );
    ensure!(
        record.archive == artifact_name(&record.manifest.identity)?,
        "artifact filename does not match its identity"
    );

    let archive_path = record_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&record.archive);
    let archive_size = fs::metadata(&archive_path)
        .with_context(|| format!("failed to stat component archive {}", archive_path.display()))?
        .len();
    ensure!(archive_size == record.archive_size, "component archive size mismatch");
    ensure!(
        sha256_file(&archive_path)? == record.archive_sha256,
        "component archive hash mismatch"
    );

    let signature = record.signature.as_ref().context("component artifact is unsigned")?;
    let public_key = public_keys
        .get(&signature.key_id)
        .with_context(|| format!("component artifact uses unknown signing key `{}`", signature.key_id))?;
    verify_manifest_signature(&record.manifest, signature, public_key)?;

    let (embedded_manifest, embedded_signature, embedded_library) =
        read_component_archive(&archive_path, &record.manifest)?;
    ensure!(
        embedded_manifest == record.manifest.canonical_bytes()?,
        "archive manifest differs from record manifest"
    );
    ensure!(
        embedded_signature == canonical_json(signature)?,
        "archive signature differs from record signature"
    );
    ensure!(
        embedded_library.len() as u64 == record.manifest.library.size,
        "component library size mismatch"
    );
    ensure!(
        sha256_bytes(&embedded_library) == record.manifest.library.sha256,
        "component library hash mismatch"
    );
    Ok(())
}

/// Read `component.json`, `component.sig`, and the declared library out of a packaged
/// archive, rejecting anything that does not look like an archive `write_package` made:
/// more than [`MAX_ARCHIVE_ENTRIES`] entries, an entry that is not a regular file, an
/// unexpected entry name, or an entry whose declared size exceeds the expected ceiling for
/// its kind (bounding how much a corrupted or hand-crafted archive can force this to
/// decompress and hold in memory).
fn read_component_archive(archive_path: &Path, manifest: &ComponentManifest) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let archive_file = fs::File::open(archive_path)
        .with_context(|| format!("failed to read component archive {}", archive_path.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive_file));

    let mut manifest_bytes = None;
    let mut signature_bytes = None;
    let mut library_bytes = None;
    let mut entry_count = 0_usize;
    for entry in tar.entries().context("failed to read component archive")? {
        entry_count += 1;
        ensure!(
            entry_count <= MAX_ARCHIVE_ENTRIES,
            "component archive has more than {MAX_ARCHIVE_ENTRIES} entries"
        );
        let entry = entry.context("failed to read a component archive entry")?;
        let path = entry
            .path()
            .context("component archive entry has an invalid path")?
            .into_owned();
        ensure!(
            entry.header().entry_type().is_file(),
            "component archive entry `{}` is not a regular file",
            path.display()
        );
        let name = path
            .to_str()
            .context("component archive entry name is not valid UTF-8")?
            .to_string();
        let ceiling = if name == manifest.library.file {
            manifest.library.size
        } else {
            METADATA_ENTRY_SIZE_CEILING
        };
        let declared_size = entry
            .header()
            .size()
            .with_context(|| format!("component archive entry `{name}` has an invalid size header"))?;
        ensure!(
            declared_size <= ceiling,
            "component archive entry `{name}` declares {declared_size} bytes, exceeding the {ceiling}-byte limit"
        );
        let mut buffer = Vec::new();
        entry
            .take(ceiling)
            .read_to_end(&mut buffer)
            .with_context(|| format!("failed to read component archive entry `{name}`"))?;
        let slot = if name == "component.json" {
            &mut manifest_bytes
        } else if name == "component.sig" {
            &mut signature_bytes
        } else if name == manifest.library.file {
            &mut library_bytes
        } else {
            bail!("component archive contains an unexpected entry `{name}`");
        };
        ensure!(slot.is_none(), "component archive contains a duplicate entry `{name}`");
        *slot = Some(buffer);
    }

    Ok((
        manifest_bytes.context("archive does not contain component.json")?,
        signature_bytes.context("archive does not contain component.sig")?,
        library_bytes.with_context(|| format!("archive does not contain {}", manifest.library.file))?,
    ))
}

fn expand_url(template: &str, identity: &ComponentIdentity, artifact: &str) -> Result<String> {
    let url = template
        .replace("{crate}", &identity.crate_name)
        .replace("{component}", &identity.component)
        .replace("{version}", &identity.version)
        .replace("{target}", &identity.target)
        .replace("{feature_hash}", &identity.feature_hash)
        .replace("{contract_hash}", &identity.contract_hash)
        .replace("{artifact}", artifact);
    ensure!(
        !url.contains('{') && !url.contains('}'),
        "component URL template contains an unknown placeholder"
    );
    Ok(url)
}

/// Sign a component manifest's canonical bytes with an in-process Ed25519 key, replacing
/// the previous `openssl pkeyutl -rawin` shell-out so signing no longer depends on the
/// system OpenSSL build supporting Ed25519.
pub fn sign_manifest(manifest: &ComponentManifest, private_key: &Path, key_id: &str) -> Result<ComponentSignature> {
    ensure!(!key_id.trim().is_empty(), "signing key ID must not be empty");
    let payload = manifest.canonical_bytes()?;
    let signing_key = load_signing_key(private_key)?;
    let signature = signing_key.sign(&payload);
    Ok(ComponentSignature {
        algorithm: "ed25519".to_string(),
        key_id: key_id.to_string(),
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
    })
}

/// Load an Ed25519 signing key from every form `openssl genpkey -algorithm ED25519`
/// (and `-outform DER`) produces -- PKCS#8 PEM or DER -- plus a raw 32-byte seed, so keys
/// minted for the previous `openssl`-based signer keep working unchanged.
fn load_signing_key(path: &Path) -> Result<SigningKey> {
    let bytes = fs::read(path).with_context(|| format!("failed to read Ed25519 private key {}", path.display()))?;
    if let Ok(text) = std::str::from_utf8(&bytes)
        && text.trim_start().starts_with("-----BEGIN")
    {
        return SigningKey::from_pkcs8_pem(text).map_err(|error| {
            anyhow!(
                "failed to parse Ed25519 private key {} as PKCS#8 PEM: {error}",
                path.display()
            )
        });
    }
    if let Ok(key) = SigningKey::from_pkcs8_der(&bytes) {
        return Ok(key);
    }
    let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        anyhow!(
            "Ed25519 private key {} is not PKCS#8 PEM/DER or a raw 32-byte seed",
            path.display()
        )
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Verify a component manifest's Ed25519 signature in-process via [`VerifyingKey::verify_strict`],
/// replacing the previous `openssl pkeyutl -rawin -pubin` shell-out.
pub fn verify_manifest_signature(
    manifest: &ComponentManifest,
    signature: &ComponentSignature,
    public_key: &str,
) -> Result<()> {
    ensure!(
        signature.algorithm == "ed25519",
        "unsupported signature algorithm `{}`",
        signature.algorithm
    );
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(&signature.signature)
        .context("component signature is not valid base64")?;
    let raw_signature =
        Signature::from_slice(&signature_bytes).context("component signature is not a valid Ed25519 signature")?;
    // Shared with `alef-component-runtime`'s loader and `alef`'s config validation so all
    // three agree on accepted key forms (PEM, or base64 standard/unpadded DER or raw
    // 32-byte bytes) -- see `alef_component_runtime::decode_public_key`.
    let key = alef_component_runtime::decode_public_key(public_key).map_err(|_| {
        anyhow!("public key must be PEM or base64-encoded (standard or unpadded) DER/raw Ed25519 bytes")
    })?;
    key.verify_strict(&manifest.canonical_bytes()?, &raw_signature)
        .map_err(|_| anyhow!("component signature verification failed"))
}

#[cfg(test)]
mod tests;
