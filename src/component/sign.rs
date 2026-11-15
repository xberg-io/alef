//! Sign unsigned component archives on a host that never builds.
//!
//! `alef component package --unsigned` runs where dependency build scripts run; `sign` runs where
//! the release key lives and reads nothing but the packaged archives and their records. Every
//! archive is re-validated under the runtime's extraction rules and its library re-hashed against the
//! manifest, then rewritten through the writer `package` uses, so a signed archive is byte-identical
//! to what a signed `package` of the same library writes and `lock` accepts it the same way.

use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use super::artifact::{
    COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentArtifactRecord, ComponentLibrary, ComponentManifest,
    MAX_ARCHIVE_ENTRIES, METADATA_ENTRY_SIZE_CEILING, canonical_json, read_record, sha256_bytes, sha256_file,
    sign_manifest_with_key, validate_name, verify_manifest_signature, write_archive,
};
use super::plan::{ComponentPlan, PlanAction};

/// What `alef component sign` signs with and checks against.
pub struct SignOptions<'a> {
    /// The release key. `SigningKey` zeroizes itself on drop.
    pub signing_key: &'a SigningKey,
    /// The key ID written into each signature; `lock` requires it in `public_keys`.
    pub key_id: &'a str,
    /// The public key configured for `key_id`; when given, the signing key must match it.
    pub expected_public_key: Option<&'a str>,
    /// A plan whose `build` rows the archives must match (identity, version, `input_hash`).
    pub plan: Option<&'a ComponentPlan>,
    /// Replace an existing signature instead of refusing an already-signed archive.
    pub resign: bool,
}

/// An archive that passed every check, with its library extracted for rewriting.
struct Prepared {
    record: ComponentArtifactRecord,
    library: PathBuf,
    _staging: tempfile::TempDir,
}

/// Sign every `*.record.json` in `input` and its archive into `output`.
///
/// Every archive is validated before anything is written, so a bad input leaves `output`
/// untouched. Records are processed in file-name order and returned in that order.
///
/// # Errors
///
/// Fails when `input` holds no records, `output` is the same directory as `input`, the key does
/// not match `expected_public_key`, or any archive fails a check.
pub fn sign_directory(input: &Path, output: &Path, options: &SignOptions<'_>) -> Result<Vec<ComponentArtifactRecord>> {
    check_key(options)?;
    ensure!(
        input.is_dir(),
        "component sign input is not a directory: {}",
        input.display()
    );
    let record_paths = record_paths(input)?;
    ensure!(
        !record_paths.is_empty(),
        "no component artifact records (*.record.json) found in {}",
        input.display()
    );
    let prepared = record_paths
        .iter()
        .map(|path| prepare(path, options))
        .collect::<Result<Vec<_>>>()?;

    fs::create_dir_all(output)
        .with_context(|| format!("failed to create component output directory {}", output.display()))?;
    let same_directory = fs::canonicalize(input).with_context(|| format!("failed to resolve {}", input.display()))?
        == fs::canonicalize(output).with_context(|| format!("failed to resolve {}", output.display()))?;
    ensure!(
        !same_directory,
        "component sign --output must differ from --input; signing in place would overwrite the unsigned inputs"
    );
    prepared
        .into_iter()
        .map(|prepared| write_signed(prepared, output, options))
        .collect()
}

fn check_key(options: &SignOptions<'_>) -> Result<()> {
    ensure!(!options.key_id.trim().is_empty(), "signing key ID must not be empty");
    if let Some(expected) = options.expected_public_key {
        let expected = alef_component_runtime::decode_public_key(expected).map_err(|_| {
            anyhow!("--public-key must be PEM or base64-encoded (standard or unpadded) DER/raw Ed25519 bytes")
        })?;
        ensure!(
            expected == options.signing_key.verifying_key(),
            "the signing key does not match the public key given for key ID `{}`",
            options.key_id
        );
    }
    Ok(())
}

fn record_paths(input: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(input).with_context(|| format!("failed to read {}", input.display()))? {
        let entry = entry.with_context(|| format!("failed to read {}", input.display()))?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".record.json"))
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

/// Check one record and its archive without trusting either.
///
/// The record must name its own archive, and its manifest hash, archive size and archive hash
/// must match. The archive must hold `component.json` first, then at most a signature and the
/// declared library, every entry a regular file with a plain file name and within its size cap.
/// The embedded manifest must be the record's, byte for byte, and the library must hash to the
/// manifest's digest. An already-signed archive needs `resign`; with a plan, the archive must
/// match a `build` row.
fn prepare(record_path: &Path, options: &SignOptions<'_>) -> Result<Prepared> {
    ensure_regular_file(record_path)?;
    let record = read_record(record_path)?;
    let archive = record.archive.clone();
    validate_name("artifact name", &archive)?;
    ensure!(
        record_path.file_name().and_then(|name| name.to_str()) == Some(format!("{archive}.record.json").as_str()),
        "component record {} does not belong to archive `{archive}`",
        record_path.display()
    );
    let manifest = &record.manifest;
    ensure!(
        manifest.schema_version == COMPONENT_MANIFEST_SCHEMA,
        "`{archive}`: unsupported component manifest schema"
    );
    ensure!(
        manifest.abi_version == COMPONENT_ABI_VERSION,
        "`{archive}`: unsupported component ABI version"
    );
    let manifest_bytes = manifest.canonical_bytes()?;
    ensure!(
        record.manifest_sha256 == sha256_bytes(&manifest_bytes),
        "`{archive}`: manifest hash mismatch"
    );

    let archive_path = record_path.parent().unwrap_or_else(|| Path::new(".")).join(&archive);
    ensure_regular_file(&archive_path)?;
    let archive_size = fs::metadata(&archive_path)
        .with_context(|| format!("failed to stat component archive {}", archive_path.display()))?
        .len();
    ensure!(
        archive_size == record.archive_size,
        "`{archive}`: component archive size mismatch"
    );
    ensure!(
        sha256_file(&archive_path)? == record.archive_sha256,
        "`{archive}`: component archive hash mismatch"
    );

    let staging = tempfile::Builder::new()
        .prefix(".alef-sign-")
        .tempdir()
        .context("failed to create a staging directory")?;
    let extracted = extract(&archive_path, staging.path())
        .with_context(|| format!("`{archive}` is not a valid component archive"))?;
    ensure!(
        extracted.manifest_bytes == manifest_bytes,
        "`{archive}`: archive component.json differs from the record's manifest"
    );
    ensure!(
        options.resign || !(extracted.signed || record.signature.is_some()),
        "`{archive}` is already signed; pass --resign to replace its signature"
    );
    if let Some(plan) = options.plan {
        check_plan_row(plan, &record)?;
    }
    Ok(Prepared {
        record,
        library: extracted.library,
        _staging: staging,
    })
}

/// A record or archive must be a regular file, not a symlink to one: the signer reads only what
/// the build job wrote into the input directory.
fn ensure_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    ensure!(metadata.is_file(), "{} is not a regular file", path.display());
    Ok(())
}

fn check_plan_row(plan: &ComponentPlan, record: &ComponentArtifactRecord) -> Result<()> {
    let identity = &record.manifest.identity;
    let described = format!("{}/{} for {}", identity.crate_name, identity.component, identity.target);
    let row = plan
        .row(&identity.crate_name, &identity.component, &identity.target)
        .with_context(|| format!("{described} is not in the plan"))?;
    ensure!(
        row.action == PlanAction::Build,
        "the plan reuses {described}, so it has no fresh archive to sign"
    );
    ensure!(
        row.input_hash == record.input_hash,
        "{described} was packaged from inputs `{}` but the plan expects `{}`",
        record.input_hash.as_deref().unwrap_or("<none>"),
        row.input_hash.as_deref().unwrap_or("<none>")
    );
    if let Some(version) = &row.version {
        ensure!(
            version == &identity.version,
            "{described} has version `{}` but the plan releases `{version}`",
            identity.version
        );
    }
    Ok(())
}

struct Extracted {
    manifest_bytes: Vec<u8>,
    signed: bool,
    library: PathBuf,
}

/// Read an archive under the rules the runtime's extractor applies: `component.json` first, at
/// most [`MAX_ARCHIVE_ENTRIES`] entries, regular files only, plain file names, metadata entries
/// capped at [`METADATA_ENTRY_SIZE_CEILING`] and the library at its declared size. The library is
/// streamed to `staging` and hashed on the way.
fn extract(archive_path: &Path, staging: &Path) -> Result<Extracted> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open component archive {}", archive_path.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut entries = tar.entries().context("failed to read component archive")?;

    let mut first = entries
        .next()
        .context("component archive is empty")?
        .context("failed to read a component archive entry")?;
    let first_name = entry_name(&first)?;
    ensure!(
        first_name == "component.json" && first.header().entry_type().is_file(),
        "the first archive entry must be the regular file component.json, found `{first_name}`"
    );
    let manifest_bytes = read_metadata_entry(&mut first, &first_name)?;
    let manifest: ComponentManifest =
        serde_json::from_slice(&manifest_bytes).context("component.json is not a component manifest")?;
    validate_name("library filename", &manifest.library.file)?;

    let mut signed = false;
    let mut library = None;
    let mut entry_count = 1_usize;
    for entry in entries {
        entry_count += 1;
        ensure!(
            entry_count <= MAX_ARCHIVE_ENTRIES,
            "component archive has more than {MAX_ARCHIVE_ENTRIES} entries"
        );
        let mut entry = entry.context("failed to read a component archive entry")?;
        let name = entry_name(&entry)?;
        ensure!(
            entry.header().entry_type().is_file(),
            "component archive entry `{name}` is not a regular file"
        );
        if name == "component.sig" && !signed {
            read_metadata_entry(&mut entry, &name)?;
            signed = true;
        } else if name == manifest.library.file && library.is_none() {
            library = Some(extract_library(&mut entry, staging, &manifest.library)?);
        } else {
            bail!("component archive contains an unexpected or duplicate entry `{name}`");
        }
    }
    let library = library.with_context(|| format!("component archive does not contain {}", manifest.library.file))?;
    Ok(Extracted {
        manifest_bytes,
        signed,
        library,
    })
}

/// An entry's name, which must be one plain file name: no directory, `..`, absolute path or
/// character outside `[A-Za-z0-9._-]`.
fn entry_name<R: Read>(entry: &tar::Entry<'_, R>) -> Result<String> {
    let path = entry.path().context("component archive entry has an invalid path")?;
    let name = path
        .to_str()
        .context("component archive entry name is not valid UTF-8")?
        .to_string();
    validate_name("component archive entry", &name)?;
    Ok(name)
}

fn read_metadata_entry<R: Read>(entry: &mut tar::Entry<'_, R>, name: &str) -> Result<Vec<u8>> {
    let declared = entry
        .header()
        .size()
        .with_context(|| format!("component archive entry `{name}` has an invalid size header"))?;
    ensure!(
        declared <= METADATA_ENTRY_SIZE_CEILING,
        "component archive entry `{name}` declares {declared} bytes, exceeding the {METADATA_ENTRY_SIZE_CEILING}-byte limit"
    );
    let mut bytes = Vec::new();
    entry
        .take(METADATA_ENTRY_SIZE_CEILING)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read component archive entry `{name}`"))?;
    Ok(bytes)
}

fn extract_library<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    staging: &Path,
    library: &ComponentLibrary,
) -> Result<PathBuf> {
    let declared = entry
        .header()
        .size()
        .with_context(|| format!("component archive entry `{}` has an invalid size header", library.file))?;
    ensure!(
        declared == library.size,
        "component library `{}` declares {declared} bytes but the manifest records {}",
        library.file,
        library.size
    );
    let path = staging.join(&library.file);
    let mut file = fs::File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let mut limited = entry.take(library.size.saturating_add(1));
    loop {
        let count = limited
            .read(&mut buffer)
            .with_context(|| format!("failed to read component library `{}`", library.file))?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        hasher.update(&buffer[..count]);
        file.write_all(&buffer[..count])
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    ensure!(
        copied == library.size,
        "component library `{}` holds {copied} bytes but the manifest records {}",
        library.file,
        library.size
    );
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ensure!(
        digest == library.sha256,
        "component library `{}` does not match the manifest's SHA-256",
        library.file
    );
    Ok(path)
}

/// Sign the manifest, check the signature, and write the archive and record. The archive goes to
/// a temporary file in `output` first and is renamed into place, so a failed write never leaves a
/// partial archive under the final name.
fn write_signed(prepared: Prepared, output: &Path, options: &SignOptions<'_>) -> Result<ComponentArtifactRecord> {
    let Prepared {
        record,
        library,
        _staging,
    } = prepared;
    let manifest = &record.manifest;
    let signature = sign_manifest_with_key(manifest, options.signing_key, options.key_id)?;
    let public_key = base64::engine::general_purpose::STANDARD.encode(options.signing_key.verifying_key().to_bytes());
    verify_manifest_signature(manifest, &signature, &public_key)?;

    let partial = tempfile::Builder::new()
        .prefix(".alef-sign-")
        .suffix(".partial")
        .tempfile_in(output)
        .with_context(|| format!("failed to create a temporary archive in {}", output.display()))?
        .into_temp_path();
    let manifest_bytes = manifest.canonical_bytes()?;
    let signature_bytes = canonical_json(&signature)?;
    let (archive_size, archive_sha256) = write_archive(
        &partial,
        &manifest_bytes,
        Some(signature_bytes.as_slice()),
        &library,
        &manifest.library.file,
        &manifest.library,
    )?;
    let archive_path = output.join(&record.archive);
    partial
        .persist(&archive_path)
        .with_context(|| format!("failed to write component archive {}", archive_path.display()))?;

    let signed = ComponentArtifactRecord {
        archive_sha256,
        archive_size,
        signature: Some(signature),
        ..record
    };
    let record_path = signed.record_path(output);
    fs::write(&record_path, canonical_json(&signed)?)
        .with_context(|| format!("failed to write component record {}", record_path.display()))?;
    Ok(signed)
}

#[cfg(test)]
#[path = "sign/tests.rs"]
mod tests;
