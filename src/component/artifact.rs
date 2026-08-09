use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use sha2::{Digest, Sha256};

pub use alef_component_runtime::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentIdentity, ComponentLibrary,
    ComponentLock, ComponentLockEntry, ComponentManifest, ComponentSignature, canonical_json,
};

#[derive(Debug, Clone)]
pub struct PackageInput<'a> {
    pub crate_name: &'a str,
    pub component: &'a str,
    pub version: &'a str,
    pub target: &'a str,
    pub contract: &'a str,
    pub contract_version: u32,
    pub contract_hash: &'a str,
    pub implementation: &'a str,
    pub features: &'a [String],
    pub default_features: bool,
    pub feature_hash: &'a str,
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(sha256_bytes(&bytes))
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
    let library_bytes = fs::read(library_path)
        .with_context(|| format!("failed to read component library {}", library_path.display()))?;
    let library_file = library_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("component library filename is not valid UTF-8")?
        .to_string();
    validate_name("library filename", &library_file)?;

    let mut features = input.features.to_vec();
    features.sort();
    features.dedup();
    let identity = ComponentIdentity {
        crate_name: input.crate_name.to_string(),
        component: input.component.to_string(),
        version: input.version.to_string(),
        target: input.target.to_string(),
        feature_hash: input.feature_hash.to_string(),
        contract_hash: input.contract_hash.to_string(),
    };
    Ok(ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: COMPONENT_ABI_VERSION,
        identity,
        contract: input.contract.to_string(),
        contract_version: input.contract_version,
        implementation: input.implementation.to_string(),
        features,
        default_features: input.default_features,
        library: ComponentLibrary {
            file: library_file.clone(),
            sha256: sha256_bytes(&library_bytes),
            size: library_bytes.len() as u64,
        },
    })
}

pub fn write_package(
    library_path: &Path,
    output_dir: &Path,
    manifest: ComponentManifest,
    signature: Option<ComponentSignature>,
) -> Result<ComponentArtifactRecord> {
    let library_bytes = fs::read(library_path)
        .with_context(|| format!("failed to read component library {}", library_path.display()))?;
    ensure!(
        manifest.library.size == library_bytes.len() as u64,
        "component library size changed while packaging"
    );
    ensure!(
        manifest.library.sha256 == sha256_bytes(&library_bytes),
        "component library changed while packaging"
    );
    let library_file = library_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("component library filename is not valid UTF-8")?;
    ensure!(
        manifest.library.file == library_file,
        "component library filename changed while packaging"
    );
    let archive = artifact_name(&manifest.identity)?;
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    if let Some(signature) = &signature {
        ensure!(
            signature.algorithm == "ed25519",
            "unsupported signature algorithm `{}`",
            signature.algorithm
        );
    }
    let signature_bytes = signature.as_ref().map(canonical_json).transpose()?;

    let mut tar = Vec::new();
    append_tar_file(&mut tar, "component.json", &manifest_bytes, 0o644)?;
    if let Some(bytes) = &signature_bytes {
        append_tar_file(&mut tar, "component.sig", bytes, 0o644)?;
    }
    append_tar_file(&mut tar, library_file, &library_bytes, 0o755)?;
    tar.extend_from_slice(&[0_u8; 1024]);
    let archive_bytes = gzip_stored(&tar);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create component output directory {}", output_dir.display()))?;
    let archive_path = output_dir.join(&archive);
    fs::write(&archive_path, &archive_bytes)
        .with_context(|| format!("failed to write component archive {}", archive_path.display()))?;

    let record = ComponentArtifactRecord {
        manifest,
        manifest_sha256,
        archive,
        archive_sha256: sha256_bytes(&archive_bytes),
        archive_size: archive_bytes.len() as u64,
        signature,
    };
    let record_path = record.record_path(output_dir);
    fs::write(&record_path, canonical_json(&record)?)
        .with_context(|| format!("failed to write component record {}", record_path.display()))?;
    Ok(record)
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
    let archive_bytes = fs::read(&archive_path)
        .with_context(|| format!("failed to read component archive {}", archive_path.display()))?;
    ensure!(
        archive_bytes.len() as u64 == record.archive_size,
        "component archive size mismatch"
    );
    ensure!(
        sha256_bytes(&archive_bytes) == record.archive_sha256,
        "component archive hash mismatch"
    );

    let signature = record.signature.as_ref().context("component artifact is unsigned")?;
    let public_key = public_keys
        .get(&signature.key_id)
        .with_context(|| format!("component artifact uses unknown signing key `{}`", signature.key_id))?;
    verify_manifest_signature(&record.manifest, signature, public_key)?;

    let tar = decode_stored_gzip(&archive_bytes)?;
    let embedded_manifest =
        find_tar_entry(&tar, "component.json")?.context("archive does not contain component.json")?;
    ensure!(
        embedded_manifest == record.manifest.canonical_bytes()?,
        "archive manifest differs from record manifest"
    );
    let embedded_signature =
        find_tar_entry(&tar, "component.sig")?.context("archive does not contain component.sig")?;
    ensure!(
        embedded_signature == canonical_json(signature)?,
        "archive signature differs from record signature"
    );
    let library = find_tar_entry(&tar, &record.manifest.library.file)?
        .with_context(|| format!("archive does not contain {}", record.manifest.library.file))?;
    ensure!(
        library.len() as u64 == record.manifest.library.size,
        "component library size mismatch"
    );
    ensure!(
        sha256_bytes(&library) == record.manifest.library.sha256,
        "component library hash mismatch"
    );
    Ok(())
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

pub fn sign_manifest(manifest: &ComponentManifest, private_key: &Path, key_id: &str) -> Result<ComponentSignature> {
    ensure!(!key_id.trim().is_empty(), "signing key ID must not be empty");
    let payload = manifest.canonical_bytes()?;
    let temp_dir = tempfile::tempdir().context("failed to create signing workspace")?;
    let payload_path = temp_dir.path().join("component.json");
    let signature_path = temp_dir.path().join("component.sig");
    fs::write(&payload_path, payload)?;
    let private_key_bytes = fs::read(private_key)
        .with_context(|| format!("failed to read Ed25519 private key {}", private_key.display()))?;
    let mut command = Command::new("openssl");
    command.args(["pkeyutl", "-sign", "-rawin", "-inkey"]).arg(private_key);
    if !private_key_bytes.starts_with(b"-----BEGIN") {
        command.args(["-keyform", "DER"]);
    }
    let output = command
        .arg("-in")
        .arg(&payload_path)
        .arg("-out")
        .arg(&signature_path)
        .output()
        .context("failed to execute openssl for Ed25519 signing")?;
    if !output.status.success() {
        bail!(
            "openssl Ed25519 signing failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let signature = fs::read(&signature_path).context("openssl did not produce a signature")?;
    Ok(ComponentSignature {
        algorithm: "ed25519".to_string(),
        key_id: key_id.to_string(),
        signature: base64::engine::general_purpose::STANDARD.encode(signature),
    })
}

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
    let (key_bytes, key_format) = decode_public_key(public_key)?;
    let temp_dir = tempfile::tempdir().context("failed to create verification workspace")?;
    let payload_path = temp_dir.path().join("component.json");
    let signature_path = temp_dir.path().join("component.sig");
    let key_path = temp_dir.path().join("component.pub");
    fs::write(&payload_path, manifest.canonical_bytes()?)?;
    fs::write(&signature_path, signature_bytes)?;
    fs::write(&key_path, key_bytes)?;

    let mut command = Command::new("openssl");
    command.args(["pkeyutl", "-verify", "-rawin", "-pubin", "-inkey"]);
    command.arg(&key_path);
    if let Some(format) = key_format {
        command.args(["-keyform", format]);
    }
    let output = command
        .arg("-in")
        .arg(&payload_path)
        .arg("-sigfile")
        .arg(&signature_path)
        .output()
        .context("failed to execute openssl for Ed25519 verification")?;
    if !output.status.success() {
        bail!(
            "component signature verification failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn decode_public_key(value: &str) -> Result<(Vec<u8>, Option<&'static str>)> {
    if value.trim_start().starts_with("-----BEGIN") {
        return Ok((value.as_bytes().to_vec(), None));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .context("public key must be PEM or base64-encoded DER/raw Ed25519 bytes")?;
    if decoded.len() == 32 {
        // RFC 8410 SubjectPublicKeyInfo prefix for a raw 32-byte Ed25519 key.
        let mut der = vec![0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
        der.extend_from_slice(&decoded);
        return Ok((der, Some("DER")));
    }
    Ok((decoded, Some("DER")))
}

fn append_tar_file(tar: &mut Vec<u8>, name: &str, content: &[u8], mode: u32) -> Result<()> {
    ensure!(name.len() <= 100, "tar entry name `{name}` is longer than 100 bytes");
    validate_name("tar entry", name)?;
    let mut header = [0_u8; 512];
    write_field(&mut header[0..100], name.as_bytes())?;
    write_octal(&mut header[100..108], u64::from(mode))?;
    write_octal(&mut header[108..116], 0)?;
    write_octal(&mut header[116..124], 0)?;
    write_octal(&mut header[124..136], content.len() as u64)?;
    write_octal(&mut header[136..148], 0)?;
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    write_checksum(&mut header[148..156], checksum)?;
    tar.extend_from_slice(&header);
    tar.extend_from_slice(content);
    let padding = (512 - (content.len() % 512)) % 512;
    tar.resize(tar.len() + padding, 0);
    Ok(())
}

fn write_field(field: &mut [u8], value: &[u8]) -> Result<()> {
    ensure!(value.len() < field.len(), "tar field is too long");
    field[..value.len()].copy_from_slice(value);
    Ok(())
}

fn write_octal(field: &mut [u8], value: u64) -> Result<()> {
    let rendered = format!("{:0width$o}\0", value, width = field.len() - 1);
    ensure!(rendered.len() == field.len(), "tar numeric field overflow");
    field.copy_from_slice(rendered.as_bytes());
    Ok(())
}

fn write_checksum(field: &mut [u8], value: u64) -> Result<()> {
    let rendered = format!("{:06o}\0 ", value);
    ensure!(rendered.len() == field.len(), "tar checksum overflow");
    field.copy_from_slice(rendered.as_bytes());
    Ok(())
}

fn gzip_stored(input: &[u8]) -> Vec<u8> {
    let mut output = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff];
    if input.is_empty() {
        output.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    } else {
        let chunks = input.chunks(u16::MAX as usize);
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            output.push(u8::from(index + 1 == chunk_count));
            let len = chunk.len() as u16;
            output.extend_from_slice(&len.to_le_bytes());
            output.extend_from_slice(&(!len).to_le_bytes());
            output.extend_from_slice(chunk);
        }
    }
    output.extend_from_slice(&crc32(input).to_le_bytes());
    output.extend_from_slice(&(input.len() as u32).to_le_bytes());
    output
}

fn decode_stored_gzip(input: &[u8]) -> Result<Vec<u8>> {
    ensure!(input.len() >= 18, "component archive is not a valid gzip stream");
    ensure!(
        input[..4] == [0x1f, 0x8b, 0x08, 0x00],
        "component archive has an unsupported gzip header"
    );
    let trailer_at = input.len() - 8;
    let mut cursor = 10;
    let mut output = Vec::new();
    loop {
        ensure!(cursor + 5 <= trailer_at, "truncated deflate block");
        let header = input[cursor];
        cursor += 1;
        ensure!(
            header & 0b110 == 0,
            "component archive uses unsupported compressed deflate blocks"
        );
        let final_block = header & 1 == 1;
        let len = u16::from_le_bytes([input[cursor], input[cursor + 1]]);
        let inverse = u16::from_le_bytes([input[cursor + 2], input[cursor + 3]]);
        cursor += 4;
        ensure!(len == !inverse, "invalid deflate stored-block length");
        let end = cursor + usize::from(len);
        ensure!(end <= trailer_at, "truncated deflate stored block");
        output.extend_from_slice(&input[cursor..end]);
        cursor = end;
        if final_block {
            break;
        }
    }
    ensure!(cursor == trailer_at, "unexpected bytes after final deflate block");
    let expected_crc = u32::from_le_bytes(input[trailer_at..trailer_at + 4].try_into().unwrap());
    let expected_size = u32::from_le_bytes(input[trailer_at + 4..].try_into().unwrap());
    ensure!(crc32(&output) == expected_crc, "gzip checksum mismatch");
    ensure!(output.len() as u32 == expected_size, "gzip size mismatch");
    Ok(output)
}

fn find_tar_entry(tar: &[u8], wanted: &str) -> Result<Option<Vec<u8>>> {
    let mut cursor = 0;
    while cursor + 512 <= tar.len() {
        let header = &tar[cursor..cursor + 512];
        if header.iter().all(|byte| *byte == 0) {
            return Ok(None);
        }
        let name_end = header[..100].iter().position(|byte| *byte == 0).unwrap_or(100);
        let name = std::str::from_utf8(&header[..name_end]).context("tar entry name is not valid UTF-8")?;
        let size_field = std::str::from_utf8(&header[124..136]).context("tar size is not valid ASCII")?;
        let size = usize::from_str_radix(size_field.trim_matches(['\0', ' ']), 8)
            .context("tar entry size is not valid octal")?;
        let content_start = cursor + 512;
        let content_end = content_start.checked_add(size).context("tar entry size overflow")?;
        ensure!(content_end <= tar.len(), "truncated tar entry `{name}`");
        if name == wanted {
            return Ok(Some(tar[content_start..content_end].to_vec()));
        }
        cursor = content_start + size.div_ceil(512) * 512;
    }
    bail!("truncated tar archive")
}

fn crc32(input: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in input {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_feature_sets_independent_of_order_and_duplicates() {
        assert_eq!(
            feature_hash(&["b".into(), "a".into(), "a".into()], false),
            feature_hash(&["a".into(), "b".into()], false)
        );
        assert_ne!(feature_hash(&["a".into()], false), feature_hash(&["a".into()], true));
    }

    #[test]
    fn artifact_name_contains_both_short_hashes() {
        let identity = ComponentIdentity {
            crate_name: "sample-core".into(),
            component: "fast".into(),
            version: "1.2.3".into(),
            target: "aarch64-apple-darwin".into(),
            feature_hash: "a".repeat(64),
            contract_hash: "b".repeat(64),
        };
        assert_eq!(
            artifact_name(&identity).unwrap(),
            "sample-core-fast-1.2.3-aaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn deterministic_package_bytes_and_record() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("libsample_core.so");
        fs::write(&library, b"native-library").unwrap();
        let features = vec!["simd".to_string(), "fast".to_string()];
        let feature_hash = "a".repeat(64);
        let contract_hash = "b".repeat(64);
        let input = || PackageInput {
            crate_name: "sample-core",
            component: "fast",
            version: "1.2.3",
            target: "x86_64-unknown-linux-gnu",
            contract: "engine",
            contract_version: 1,
            contract_hash: &contract_hash,
            implementation: "sample_core::FastEngine",
            features: &features,
            default_features: false,
            feature_hash: &feature_hash,
        };
        let first_manifest = create_manifest(&library, input()).unwrap();
        let first = write_package(&library, temp.path(), first_manifest, None).unwrap();
        let first_bytes = fs::read(temp.path().join(&first.archive)).unwrap();
        let second_manifest = create_manifest(&library, input()).unwrap();
        let second = write_package(&library, temp.path(), second_manifest, None).unwrap();
        let second_bytes = fs::read(temp.path().join(&second.archive)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(&first_bytes[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
        let tar = decode_stored_gzip(&first_bytes).unwrap();
        assert_eq!(
            find_tar_entry(&tar, "libsample_core.so").unwrap().unwrap(),
            b"native-library"
        );
    }

    #[test]
    fn signs_packages_and_verifies_the_complete_archive() {
        if Command::new("openssl").arg("version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let private_key = temp.path().join("release-private.pem");
        let public_key = temp.path().join("release-public.pem");
        let generated = Command::new("openssl")
            .args(["genpkey", "-algorithm", "ED25519", "-out"])
            .arg(&private_key)
            .status()
            .unwrap();
        assert!(generated.success());
        let exported = Command::new("openssl")
            .args(["pkey", "-in"])
            .arg(&private_key)
            .args(["-pubout", "-out"])
            .arg(&public_key)
            .status()
            .unwrap();
        assert!(exported.success());

        let library = temp.path().join("libsample_core.so");
        fs::write(&library, b"signed-native-library").unwrap();
        let features = vec!["fast".to_string()];
        let feature_hash = feature_hash(&features, false);
        let contract_hash = "b".repeat(64);
        let manifest = create_manifest(
            &library,
            PackageInput {
                crate_name: "sample-core",
                component: "fast",
                version: "1.2.3",
                target: "x86_64-unknown-linux-gnu",
                contract: "engine",
                contract_version: 1,
                contract_hash: &contract_hash,
                implementation: "sample_core::FastEngine",
                features: &features,
                default_features: false,
                feature_hash: &feature_hash,
            },
        )
        .unwrap();
        let signature = sign_manifest(&manifest, &private_key, "release").unwrap();
        let record = write_package(&library, temp.path(), manifest, Some(signature)).unwrap();
        let keys = BTreeMap::from([("release".into(), fs::read_to_string(public_key).unwrap())]);
        verify_record(&record.record_path(temp.path()), &keys).unwrap();
    }

    #[test]
    fn lock_is_sorted_and_expands_urls() {
        let make_record = |component: &str| ComponentArtifactRecord {
            manifest: ComponentManifest {
                schema_version: 1,
                abi_version: 1,
                identity: ComponentIdentity {
                    crate_name: "core".into(),
                    component: component.into(),
                    version: "1.0.0".into(),
                    target: "x86_64-unknown-linux-gnu".into(),
                    feature_hash: "a".repeat(64),
                    contract_hash: "b".repeat(64),
                },
                contract: "engine".into(),
                contract_version: 1,
                implementation: "core::Engine".into(),
                features: vec![],
                default_features: false,
                library: ComponentLibrary {
                    file: "libcore.so".into(),
                    sha256: "c".repeat(64),
                    size: 1,
                },
            },
            manifest_sha256: "d".repeat(64),
            archive: format!("{component}.tar.gz"),
            archive_sha256: "e".repeat(64),
            archive_size: 2,
            signature: Some(ComponentSignature {
                algorithm: "ed25519".into(),
                key_id: "release".into(),
                signature: "AA==".into(),
            }),
        };
        let keys = BTreeMap::from([("release".into(), "key".into())]);
        let lock = build_lock(
            &[make_record("zeta"), make_record("alpha")],
            "https://example.invalid/{version}/{target}/{artifact}",
            &keys,
        )
        .unwrap();
        assert_eq!(lock.artifacts[0].identity.component, "alpha");
        assert!(lock.artifacts[0].url.ends_with("/alpha.tar.gz"));
    }
}
