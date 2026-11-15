use std::collections::BTreeMap;
use std::io::Cursor;

use alef_component_runtime::{ArtifactCache, TrustPolicy};

use super::*;
use crate::component::artifact::{
    PackageInput, ProvidedContractInput, SigningKeySource, build_lock, create_manifest, feature_hash,
    load_signing_key_from_env, load_signing_key_from_source, verify_record_with, write_package_recorded,
};
use crate::component::plan::PlanRow;

const LIBRARY: &[u8] = b"native-component-library";
const INPUT_HASH: &str = "1f0e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
const URL_TEMPLATE: &str = "https://example.invalid/{artifact}";

type Builder<'a> = tar::Builder<flate2::write::GzEncoder<&'a mut Vec<u8>>>;

/// An unsigned `package --plan` output in `<root>/unsigned`, built from `<root>/build`.
struct Fixture {
    root: tempfile::TempDir,
    input: PathBuf,
    output: PathBuf,
    library: PathBuf,
    record: ComponentArtifactRecord,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let build = root.path().join("build");
        fs::create_dir(&build).unwrap();
        let library = build.join("libsample_core.so");
        fs::write(&library, LIBRARY).unwrap();
        let input = root.path().join("unsigned");
        let record = write_package_recorded(
            &library,
            &input,
            manifest_for(&library),
            None,
            None,
            Some(INPUT_HASH.into()),
        )
        .unwrap();
        let output = root.path().join("signed");
        Self {
            root,
            input,
            output,
            library,
            record,
        }
    }

    fn sign(&self, options: &SignOptions<'_>) -> Result<Vec<ComponentArtifactRecord>> {
        sign_directory(&self.input, &self.output, options)
    }

    /// Sign, expect a refusal, check nothing was written, and return the full error chain.
    fn refusal(&self, options: &SignOptions<'_>) -> String {
        let error = format!("{:#}", self.sign(options).unwrap_err());
        assert!(
            !self.output.exists() || fs::read_dir(&self.output).unwrap().next().is_none(),
            "a refused sign must write nothing"
        );
        error
    }

    fn archive_bytes(&self, dir: &Path) -> Vec<u8> {
        fs::read(dir.join(&self.record.archive)).unwrap()
    }

    fn manifest_bytes(&self) -> Vec<u8> {
        self.record.manifest.canonical_bytes().unwrap()
    }

    /// Replace the unsigned archive with one built by `fill`, updating the record's archive hash
    /// and size so only the archive's contents are under test.
    fn rewrite_archive(&mut self, fill: impl FnOnce(&mut Builder<'_>)) {
        let mut compressed = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            fill(&mut builder);
            builder.into_inner().unwrap().finish().unwrap();
        }
        fs::write(self.input.join(&self.record.archive), &compressed).unwrap();
        self.record.archive_size = compressed.len() as u64;
        self.record.archive_sha256 = sha256_bytes(&compressed);
        fs::write(
            self.record.record_path(&self.input),
            canonical_json(&self.record).unwrap(),
        )
        .unwrap();
    }
}

fn manifest_for(library: &Path) -> ComponentManifest {
    let features = vec!["fast".to_string()];
    let feature_hash = feature_hash(&features, false);
    let contract_hash = "b".repeat(64);
    let provides = [ProvidedContractInput {
        contract: "engine",
        interface_version: 1,
        contract_hash: &contract_hash,
        implementation: "sample_core::FastEngine",
    }];
    create_manifest(
        library,
        PackageInput {
            crate_name: "sample-core",
            component: "fast",
            version: "1.2.3",
            target: "x86_64-unknown-linux-gnu",
            provides: &provides,
            features: &features,
            default_features: false,
            feature_hash: &feature_hash,
        },
    )
    .unwrap()
}

fn release_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[8; 32])
}

fn public_key(key: &SigningKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes())
}

fn trusted(key_id: &str, key: &SigningKey) -> BTreeMap<String, String> {
    BTreeMap::from([(key_id.to_string(), public_key(key))])
}

fn options(key: &SigningKey) -> SignOptions<'_> {
    SignOptions {
        signing_key: key,
        key_id: "release",
        expected_public_key: None,
        plan: None,
        resign: false,
    }
}

fn append_file(builder: &mut Builder<'_>, name: &str, content: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, name, Cursor::new(content)).unwrap();
}

/// Append an entry whose raw name bypasses `tar`'s own path checks, as a hostile archive would.
fn append_raw_name(builder: &mut Builder<'_>, name: &[u8], content: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder.append(&header, Cursor::new(content)).unwrap();
}

fn install(archive: &Path, record: &ComponentArtifactRecord, keys: BTreeMap<String, String>) -> Result<Vec<u8>> {
    let lock = build_lock(std::slice::from_ref(record), URL_TEMPLATE, &keys)?;
    let cache = tempfile::tempdir().unwrap();
    let installed = ArtifactCache::new(cache.path(), TrustPolicy::EmbeddedKeys(keys))
        .install_from_reader(&lock.artifacts[0], fs::File::open(archive).unwrap())?;
    Ok(fs::read(installed.library).unwrap())
}

#[test]
fn signs_the_same_archive_a_signed_package_writes() {
    let fixture = Fixture::new();
    let key = release_key();
    let signed = fixture.sign(&options(&key)).unwrap();
    assert_eq!(signed.len(), 1);

    let packaged_dir = fixture.root.path().join("packaged");
    let manifest = manifest_for(&fixture.library);
    let signature = sign_manifest_with_key(&manifest, &key, "release").unwrap();
    let packaged = write_package_recorded(
        &fixture.library,
        &packaged_dir,
        manifest,
        Some(signature),
        None,
        Some(INPUT_HASH.into()),
    )
    .unwrap();

    assert_eq!(signed[0], packaged);
    assert_eq!(
        fixture.archive_bytes(&fixture.output),
        fixture.archive_bytes(&packaged_dir)
    );
    assert_eq!(
        fs::read(signed[0].record_path(&fixture.output)).unwrap(),
        fs::read(packaged.record_path(&packaged_dir)).unwrap()
    );
}

/// `component lock` verifies each record with `verify_record_with` and builds entries with
/// `build_lock`; the runtime then installs the archive against the lock's embedded keys.
#[test]
fn signed_archive_passes_lock_verification_and_runtime_install() {
    let fixture = Fixture::new();
    let key = release_key();
    let signed = fixture.sign(&options(&key)).unwrap();
    let record_path = signed[0].record_path(&fixture.output);
    let keys = trusted("release", &key);

    verify_record_with(&record_path, Some(&keys), None).unwrap();
    let lock = build_lock(&signed, URL_TEMPLATE, &keys).unwrap();
    assert_eq!(lock.artifacts[0].key_id, "release");
    assert_eq!(lock.artifacts[0].input_hash.as_deref(), Some(INPUT_HASH));
    let library = install(&fixture.output.join(&signed[0].archive), &signed[0], keys).unwrap();
    assert_eq!(library, LIBRARY);
}

#[test]
fn signing_is_deterministic() {
    let fixture = Fixture::new();
    let key = release_key();
    fixture.sign(&options(&key)).unwrap();
    let again = fixture.root.path().join("signed-again");
    sign_directory(&fixture.input, &again, &options(&key)).unwrap();
    assert_eq!(fixture.archive_bytes(&fixture.output), fixture.archive_bytes(&again));
    assert_eq!(
        fs::read(fixture.record.record_path(&fixture.output)).unwrap(),
        fs::read(fixture.record.record_path(&again)).unwrap()
    );
}

#[test]
fn keys_from_a_file_and_an_environment_value_sign_identically() {
    let fixture = Fixture::new();
    let seed = release_key().to_bytes();
    let key_file = fixture.root.path().join("release.seed");
    fs::write(&key_file, seed).unwrap();
    let from_file = load_signing_key_from_source(&SigningKeySource::File(key_file)).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(seed);
    let from_env = load_signing_key_from_env("RELEASE_SIGNING_KEY", |_| Ok(encoded)).unwrap();

    fixture.sign(&options(&from_file)).unwrap();
    let env_output = fixture.root.path().join("signed-env");
    sign_directory(&fixture.input, &env_output, &options(&from_env)).unwrap();
    assert_eq!(
        fixture.archive_bytes(&fixture.output),
        fixture.archive_bytes(&env_output)
    );
}

#[test]
fn refuses_a_key_that_does_not_match_the_given_public_key() {
    let fixture = Fixture::new();
    let key = release_key();
    let expected = public_key(&other_key());
    let error = fixture.refusal(&SignOptions {
        expected_public_key: Some(expected.as_str()),
        ..options(&key)
    });
    assert!(error.contains("does not match the public key"), "{error}");

    let matching = public_key(&key);
    fixture
        .sign(&SignOptions {
            expected_public_key: Some(matching.as_str()),
            ..options(&key)
        })
        .unwrap();
}

#[test]
fn lock_and_runtime_refuse_a_signature_under_the_wrong_key_id() {
    let fixture = Fixture::new();
    let key = release_key();
    let signed = fixture.sign(&options(&key)).unwrap();
    let record_path = signed[0].record_path(&fixture.output);

    let unknown_id = trusted("release-2027", &key);
    let error = format!("{:#}", build_lock(&signed, URL_TEMPLATE, &unknown_id).unwrap_err());
    assert!(error.contains("unknown signing key `release`"), "{error}");

    let other = trusted("release", &other_key());
    let error = format!(
        "{:#}",
        verify_record_with(&record_path, Some(&other), None).unwrap_err()
    );
    assert!(error.contains("signature verification failed"), "{error}");
    assert!(install(&fixture.output.join(&signed[0].archive), &signed[0], other).is_err());
}

#[test]
fn refuses_an_entry_that_escapes_the_archive() {
    let mut fixture = Fixture::new();
    let manifest = fixture.manifest_bytes();
    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_raw_name(builder, b"../libsample_core.so", LIBRARY);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("`../libsample_core.so`"), "{error}");
}

#[test]
fn refuses_a_symlink_entry() {
    let mut fixture = Fixture::new();
    let manifest = fixture.manifest_bytes();
    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_path("libsample_core.so").unwrap();
        header.set_cksum();
        builder.append(&header, Cursor::new(&[][..])).unwrap();
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("is not a regular file"), "{error}");
}

#[test]
fn refuses_unexpected_and_surplus_entries() {
    let mut fixture = Fixture::new();
    let manifest = fixture.manifest_bytes();
    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_file(builder, "README", b"extra");
        append_file(builder, "libsample_core.so", LIBRARY);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("unexpected or duplicate entry `README`"), "{error}");

    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_file(builder, "libsample_core.so", LIBRARY);
        append_file(builder, "libsample_core.so", LIBRARY);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("duplicate entry `libsample_core.so`"), "{error}");

    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_file(builder, "component.sig", b"{}");
        append_file(builder, "libsample_core.so", LIBRARY);
        append_file(builder, "extra.so", LIBRARY);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("more than 3 entries"), "{error}");
}

#[test]
fn refuses_a_manifest_that_is_not_the_first_entry() {
    let mut fixture = Fixture::new();
    let manifest = fixture.manifest_bytes();
    fixture.rewrite_archive(|builder| {
        append_file(builder, "libsample_core.so", LIBRARY);
        append_file(builder, "component.json", &manifest);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("must be the regular file component.json"), "{error}");
}

#[test]
fn refuses_a_library_that_does_not_match_the_manifest() {
    let mut fixture = Fixture::new();
    let manifest = fixture.manifest_bytes();
    let mut swapped = LIBRARY.to_vec();
    swapped[0] ^= 1;
    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_file(builder, "libsample_core.so", &swapped);
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("does not match the manifest's SHA-256"), "{error}");

    fixture.rewrite_archive(|builder| {
        append_file(builder, "component.json", &manifest);
        append_file(builder, "libsample_core.so", b"short");
    });
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("declares 5 bytes"), "{error}");
}

#[test]
fn refuses_an_archive_changed_after_packaging() {
    let fixture = Fixture::new();
    let archive = fixture.input.join(&fixture.record.archive);
    let mut bytes = fs::read(&archive).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&archive, bytes).unwrap();
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("component archive hash mismatch"), "{error}");
}

#[test]
fn refuses_a_record_whose_manifest_differs_from_the_archive() {
    let mut fixture = Fixture::new();
    fixture.record.manifest.identity.version = "9.9.9".into();
    fixture.record.manifest_sha256 = sha256_bytes(&fixture.manifest_bytes());
    fs::write(
        fixture.record.record_path(&fixture.input),
        canonical_json(&fixture.record).unwrap(),
    )
    .unwrap();
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("differs from the record's manifest"), "{error}");
}

#[cfg(unix)]
#[test]
fn refuses_a_symlinked_archive_file() {
    let fixture = Fixture::new();
    let archive = fixture.input.join(&fixture.record.archive);
    let elsewhere = fixture.root.path().join("elsewhere.tar.gz");
    fs::rename(&archive, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &archive).unwrap();
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("is not a regular file"), "{error}");
}

#[test]
fn refuses_already_signed_input_unless_resigning() {
    let fixture = Fixture::new();
    let key = release_key();
    fixture.sign(&options(&key)).unwrap();
    let resigned = fixture.root.path().join("resigned");

    let error = format!(
        "{:#}",
        sign_directory(&fixture.output, &resigned, &options(&key)).unwrap_err()
    );
    assert!(error.contains("already signed; pass --resign"), "{error}");
    assert!(!resigned.exists());

    let rotated = other_key();
    let records = sign_directory(
        &fixture.output,
        &resigned,
        &SignOptions {
            key_id: "release-2027",
            resign: true,
            ..options(&rotated)
        },
    )
    .unwrap();
    assert_eq!(records[0].signature.as_ref().unwrap().key_id, "release-2027");
    verify_record_with(
        &records[0].record_path(&resigned),
        Some(&trusted("release-2027", &rotated)),
        None,
    )
    .unwrap();
}

#[test]
fn refuses_to_sign_in_place() {
    let fixture = Fixture::new();
    let error = format!(
        "{:#}",
        sign_directory(&fixture.input, &fixture.input, &options(&release_key())).unwrap_err()
    );
    assert!(error.contains("--output must differ from --input"), "{error}");
}

#[test]
fn validates_every_archive_before_writing_any() {
    let fixture = Fixture::new();
    // Sorts after the valid record and names an archive it does not belong to.
    fs::copy(
        fixture.record.record_path(&fixture.input),
        fixture.input.join("zz.record.json"),
    )
    .unwrap();
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("does not belong to archive"), "{error}");
}

#[test]
fn refuses_an_input_without_records() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.record.record_path(&fixture.input)).unwrap();
    let error = fixture.refusal(&options(&release_key()));
    assert!(error.contains("no component artifact records"), "{error}");
}

fn plan_row(action: PlanAction, input_hash: &str, version: &str) -> PlanRow {
    PlanRow {
        crate_name: "sample-core".into(),
        component: "fast".into(),
        target: "x86_64-unknown-linux-gnu".into(),
        input_hash: Some(input_hash.into()),
        action,
        version: Some(version.into()),
        source_hash: None,
        host_family: None,
        previous: None,
    }
}

#[test]
fn signs_archives_that_match_their_plan_row() {
    let fixture = Fixture::new();
    let key = release_key();
    let plan = ComponentPlan::new(false, vec![plan_row(PlanAction::Build, INPUT_HASH, "1.2.3")]);
    fixture
        .sign(&SignOptions {
            plan: Some(&plan),
            ..options(&key)
        })
        .unwrap();
}

#[test]
fn refuses_archives_the_plan_does_not_expect() {
    let fixture = Fixture::new();
    let key = release_key();
    let cases = [
        (
            ComponentPlan::new(false, vec![plan_row(PlanAction::Build, &"0".repeat(64), "1.2.3")]),
            "but the plan expects",
        ),
        (
            ComponentPlan::new(false, vec![plan_row(PlanAction::Build, INPUT_HASH, "1.2.4")]),
            "but the plan releases `1.2.4`",
        ),
        (ComponentPlan::new(false, Vec::new()), "is not in the plan"),
    ];
    for (plan, expected) in &cases {
        let error = fixture.refusal(&SignOptions {
            plan: Some(plan),
            ..options(&key)
        });
        assert!(error.contains(*expected), "{error}");
    }

    let mut reuse = plan_row(PlanAction::Reuse, INPUT_HASH, "1.2.3");
    reuse.previous = Some(
        crate::component::artifact::build_unsigned_lock(std::slice::from_ref(&fixture.record), URL_TEMPLATE)
            .unwrap()
            .artifacts
            .remove(0),
    );
    let plan = ComponentPlan::new(false, vec![reuse]);
    let error = fixture.refusal(&SignOptions {
        plan: Some(&plan),
        ..options(&key)
    });
    assert!(error.contains("the plan reuses"), "{error}");
}
