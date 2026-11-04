use super::*;
use ed25519_dalek::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _};

/// Wrap DER bytes as PEM without depending on `pkcs8`'s PEM feature, which pulls in
/// its own line-ending type; a fixed-width base64 wrap is all PEM actually is.
fn pem_wrap(label: &str, der: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap());
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem
}

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
    let provides = [ProvidedContractInput {
        contract: "engine",
        interface_version: 1,
        contract_hash: &contract_hash,
        implementation: "sample_core::FastEngine",
    }];
    let input = || PackageInput {
        crate_name: "sample-core",
        component: "fast",
        version: "1.2.3",
        target: "x86_64-unknown-linux-gnu",
        provides: &provides,
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
    assert_eq!(read_archive_entry(&first_bytes, "libsample_core.so"), b"native-library");
}

fn read_archive_entry(archive_bytes: &[u8], wanted: &str) -> Vec<u8> {
    let decoder = flate2::read::GzDecoder::new(archive_bytes);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        if entry.path().unwrap().to_str().unwrap() == wanted {
            let mut buffer = Vec::new();
            entry.read_to_end(&mut buffer).unwrap();
            return buffer;
        }
    }
    panic!("archive does not contain entry `{wanted}`");
}

/// The old hand-rolled tar writer capped every entry name at 100 bytes; GNU long-name
/// support (used automatically by `tar::Builder::append_data` once a name does not fit
/// the classic header field) means a component library with a long, descriptive
/// filename now round-trips through packaging and verification instead of failing to
/// package at all.
#[test]
fn packages_and_verifies_a_library_with_a_long_file_name() {
    let temp = tempfile::tempdir().unwrap();
    let long_stem = "sample_core_with_a_very_long_and_descriptive_component_library_file_name_indeed";
    assert!(long_stem.len() > 64);
    let library = temp.path().join(format!("lib{long_stem}.so"));
    fs::write(&library, b"long-name-library").unwrap();
    let features = vec!["fast".to_string()];
    let feature_hash = feature_hash(&features, false);
    let contract_hash = "b".repeat(64);
    let provides = [ProvidedContractInput {
        contract: "engine",
        interface_version: 1,
        contract_hash: &contract_hash,
        implementation: "sample_core::FastEngine",
    }];
    let manifest = create_manifest(
        &library,
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
    .unwrap();
    let signing_key = SigningKey::from_bytes(&[10; 32]);
    let signature = sign_manifest(
        &manifest,
        &{
            let private_key = temp.path().join("release-private.der");
            fs::write(&private_key, signing_key.to_pkcs8_der().unwrap().as_bytes()).unwrap();
            private_key
        },
        "release",
    )
    .unwrap();
    let record = write_package(&library, temp.path(), manifest, Some(signature)).unwrap();
    let archive_bytes = fs::read(temp.path().join(&record.archive)).unwrap();
    assert_eq!(
        read_archive_entry(&archive_bytes, &format!("lib{long_stem}.so")),
        b"long-name-library"
    );
    assert_eq!(
        read_archive_entry(&archive_bytes, "component.json"),
        record.manifest.canonical_bytes().unwrap()
    );
    let keys = BTreeMap::from([(
        "release".into(),
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes()),
    )]);
    verify_record(&record.record_path(temp.path()), &keys).unwrap();
}

#[test]
fn signs_packages_and_verifies_the_complete_archive() {
    let temp = tempfile::tempdir().unwrap();
    let signing_key = SigningKey::from_bytes(&[3; 32]);
    let private_key = temp.path().join("release-private.pem");
    let public_key = temp.path().join("release-public.pem");
    fs::write(
        &private_key,
        pem_wrap("PRIVATE KEY", signing_key.to_pkcs8_der().unwrap().as_bytes()),
    )
    .unwrap();
    fs::write(
        &public_key,
        pem_wrap(
            "PUBLIC KEY",
            signing_key.verifying_key().to_public_key_der().unwrap().as_bytes(),
        ),
    )
    .unwrap();

    let library = temp.path().join("libsample_core.so");
    fs::write(&library, b"signed-native-library").unwrap();
    let features = vec!["fast".to_string()];
    let feature_hash = feature_hash(&features, false);
    let contract_hash = "b".repeat(64);
    let provides = [ProvidedContractInput {
        contract: "engine",
        interface_version: 1,
        contract_hash: &contract_hash,
        implementation: "sample_core::FastEngine",
    }];
    let manifest = create_manifest(
        &library,
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
    .unwrap();
    // Exercise the PKCS#8 PEM loading path (`openssl genpkey -algorithm ED25519` output).
    let signature = sign_manifest(&manifest, &private_key, "release").unwrap();
    let record = write_package(&library, temp.path(), manifest, Some(signature)).unwrap();
    let keys = BTreeMap::from([("release".into(), fs::read_to_string(public_key).unwrap())]);
    verify_record(&record.record_path(temp.path()), &keys).unwrap();
}

#[test]
fn signs_with_a_pkcs8_der_private_key() {
    let temp = tempfile::tempdir().unwrap();
    let signing_key = SigningKey::from_bytes(&[4; 32]);
    let private_key = temp.path().join("release-private.der");
    fs::write(&private_key, signing_key.to_pkcs8_der().unwrap().as_bytes()).unwrap();

    let manifest = manifest_fixture(&temp, "signed-native-library-der");
    let signature = sign_manifest(&manifest, &private_key, "release").unwrap();
    let public_key = base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());
    verify_manifest_signature(&manifest, &signature, &public_key).unwrap();
}

#[test]
fn signs_with_a_raw_seed_private_key() {
    let temp = tempfile::tempdir().unwrap();
    let signing_key = SigningKey::from_bytes(&[5; 32]);
    let private_key = temp.path().join("release-private.seed");
    fs::write(&private_key, signing_key.to_bytes()).unwrap();

    let manifest = manifest_fixture(&temp, "signed-native-library-seed");
    let signature = sign_manifest(&manifest, &private_key, "release").unwrap();
    let public_key = base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());
    verify_manifest_signature(&manifest, &signature, &public_key).unwrap();
}

fn manifest_fixture(temp: &tempfile::TempDir, library_contents: &str) -> ComponentManifest {
    let library = temp.path().join(format!("lib{library_contents}.so"));
    fs::write(&library, library_contents.as_bytes()).unwrap();
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
        &library,
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

/// Regression: config validation accepted unpadded base64 public keys
/// (`base64::STANDARD_NO_PAD`) while this module's decoder previously accepted only
/// `base64::STANDARD`, so a key config validation let through could fail verification
/// on every load. Both must now go through the same shared decoder.
#[test]
fn verifies_signature_with_unpadded_base64_public_key() {
    let temp = tempfile::tempdir().unwrap();
    let signing_key = SigningKey::from_bytes(&[6; 32]);
    let private_key = temp.path().join("release-private.pem");
    fs::write(
        &private_key,
        pem_wrap("PRIVATE KEY", signing_key.to_pkcs8_der().unwrap().as_bytes()),
    )
    .unwrap();
    let unpadded_key = base64::engine::general_purpose::STANDARD_NO_PAD.encode(signing_key.verifying_key().to_bytes());

    let library = temp.path().join("libsample_core.so");
    fs::write(&library, b"signed-native-library-unpadded").unwrap();
    let features = vec!["fast".to_string()];
    let feature_hash = feature_hash(&features, false);
    let contract_hash = "b".repeat(64);
    let provides = [ProvidedContractInput {
        contract: "engine",
        interface_version: 1,
        contract_hash: &contract_hash,
        implementation: "sample_core::FastEngine",
    }];
    let manifest = create_manifest(
        &library,
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
    .unwrap();
    let signature = sign_manifest(&manifest, &private_key, "release").unwrap();
    let record = write_package(&library, temp.path(), manifest, Some(signature)).unwrap();
    let keys = BTreeMap::from([("release".into(), unpadded_key)]);
    verify_record(&record.record_path(temp.path()), &keys).unwrap();
}

/// A manifest signed by the previous `openssl pkeyutl -rawin -sign` shell-out must keep
/// verifying under the in-process `ed25519-dalek` verifier: the signed payload (the
/// manifest's canonical bytes) and the wire format (base64-encoded raw 64-byte Ed25519
/// signature) are unchanged, only the signer/verifier implementation moved in-process.
/// Fixture generated once with a fixed Ed25519 seed:
/// `openssl pkeyutl -sign -rawin -inkey <PKCS8 DER of seed [7; 32]> -in <canonical bytes>`.
#[test]
fn verifies_a_signature_produced_by_the_legacy_openssl_signer() {
    const FIXTURE_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=\n-----END PUBLIC KEY-----\n";
    const FIXTURE_SIGNATURE_BASE64: &str =
        "LZMsYA525OsThf1/QrGg3rzSV+8SOWXGmo8WqFvywQ2mrXpJuR8RTiFSaDCgmZsgoC1u4kVlnyNx1r6VlexpDg==";

    let manifest = ComponentManifest {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        abi_version: COMPONENT_ABI_VERSION,
        identity: ComponentIdentity {
            crate_name: "sample-core".into(),
            component: "fast".into(),
            version: "1.2.3".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            feature_hash: feature_hash(&["fast".to_string()], false),
            contract_hash: "b".repeat(64),
        },
        provides: vec![ComponentProvidedContract {
            contract: "engine".into(),
            interface_version: 1,
            contract_hash: "b".repeat(64),
            implementation: "sample_core::FastEngine".into(),
        }],
        features: vec!["fast".into()],
        default_features: false,
        library: ComponentLibrary {
            file: "libsample_core.so".into(),
            sha256: sha256_bytes(b"fixture-signed-library"),
            size: b"fixture-signed-library".len() as u64,
        },
    };
    let signature = ComponentSignature {
        algorithm: "ed25519".into(),
        key_id: "release".into(),
        signature: FIXTURE_SIGNATURE_BASE64.to_string(),
    };
    verify_manifest_signature(&manifest, &signature, FIXTURE_PUBLIC_KEY_PEM).unwrap();
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
            provides: vec![ComponentProvidedContract {
                contract: "engine".into(),
                interface_version: 1,
                contract_hash: "b".repeat(64),
                implementation: "core::Engine".into(),
            }],
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
        input_hash: None,
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
    assert_eq!(lock.artifacts[0].mode, ComponentDeliveryMode::Download);
    assert_eq!(lock.artifacts[0].provides[0].contract, "engine");
}

#[test]
fn manifest_records_every_provided_contract_sorted_by_name() {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("libsample_core.so");
    fs::write(&library, b"native-library").unwrap();
    let features = vec!["fast".to_string()];
    let feature_hash = feature_hash(&features, false);
    let zeta_hash = "b".repeat(64);
    let alpha_hash = "c".repeat(64);
    let provides = [
        ProvidedContractInput {
            contract: "zeta",
            interface_version: 1,
            contract_hash: &zeta_hash,
            implementation: "sample_core::Zeta",
        },
        ProvidedContractInput {
            contract: "alpha",
            interface_version: 2,
            contract_hash: &alpha_hash,
            implementation: "sample_core::Alpha",
        },
    ];
    let manifest = create_manifest(
        &library,
        PackageInput {
            crate_name: "sample-core",
            component: "bundle",
            version: "1.2.3",
            target: "x86_64-unknown-linux-gnu",
            provides: &provides,
            features: &features,
            default_features: false,
            feature_hash: &feature_hash,
        },
    )
    .unwrap();
    assert_eq!(manifest.provides.len(), 2);
    assert_eq!(manifest.provides[0].contract, "alpha");
    assert_eq!(manifest.provides[1].contract, "zeta");
    // Identity carries the first (lexicographically) contract's real hash.
    assert_eq!(manifest.identity.contract_hash, alpha_hash);
}

#[test]
fn manifest_rejects_a_contract_provided_more_than_once() {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("libsample_core.so");
    fs::write(&library, b"native-library").unwrap();
    let features: Vec<String> = Vec::new();
    let feature_hash = feature_hash(&features, false);
    let hash = "b".repeat(64);
    let provides = [
        ProvidedContractInput {
            contract: "engine",
            interface_version: 1,
            contract_hash: &hash,
            implementation: "sample_core::One",
        },
        ProvidedContractInput {
            contract: "engine",
            interface_version: 1,
            contract_hash: &hash,
            implementation: "sample_core::Two",
        },
    ];
    let error = create_manifest(
        &library,
        PackageInput {
            crate_name: "sample-core",
            component: "bundle",
            version: "1.2.3",
            target: "x86_64-unknown-linux-gnu",
            provides: &provides,
            features: &features,
            default_features: false,
            feature_hash: &feature_hash,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("more than once"));
}

fn identity_fixture() -> ComponentIdentity {
    ComponentIdentity {
        crate_name: "poly".into(),
        component: "ruff".into(),
        version: "1.2.3".into(),
        target: "aarch64-pc-windows-msvc".into(),
        feature_hash: "a".repeat(64),
        contract_hash: "b".repeat(64),
    }
}

#[test]
fn artifact_name_template_renders_every_placeholder() {
    let identity = identity_fixture();
    assert_eq!(
        artifact_name_with(&identity, Some("poly-{component}-{version}-{target}.tar.gz")).unwrap(),
        "poly-ruff-1.2.3-aarch64-pc-windows-msvc.tar.gz"
    );
    assert_eq!(
        artifact_name_with(&identity, Some("{crate}_{component}_{fh16}_{ch16}.tgz")).unwrap(),
        "poly_ruff_aaaaaaaaaaaaaaaa_bbbbbbbbbbbbbbbb.tgz"
    );
    assert_eq!(
        artifact_name_with(&identity, None).unwrap(),
        artifact_name(&identity).unwrap()
    );
}

#[test]
fn artifact_name_template_rejects_unsafe_or_unknown_output() {
    let identity = identity_fixture();
    assert!(artifact_name_with(&identity, Some("../{component}.tar.gz")).is_err());
    assert!(artifact_name_with(&identity, Some("a/{component}.tar.gz")).is_err());
    assert!(artifact_name_with(&identity, Some("{bogus}-{component}.tar.gz")).is_err());
    assert!(artifact_name_with(&identity, Some("..")).is_err());
}

#[test]
fn custom_artifact_name_round_trips_through_package_and_verify() {
    let temp = tempfile::tempdir().unwrap();
    let signing_key = SigningKey::from_bytes(&[7; 32]);
    let manifest = manifest_fixture(&temp, "named-library");
    let library = temp.path().join("libnamed-library.so");
    let signature = sign_manifest_with_key(&manifest, &signing_key, "release").unwrap();
    let template = "poly-{component}-{version}-{target}.tar.gz";
    let record = write_package_named(&library, temp.path(), manifest, Some(signature), Some(template)).unwrap();
    assert_eq!(record.archive, "poly-fast-1.2.3-x86_64-unknown-linux-gnu.tar.gz");
    assert!(temp.path().join(&record.archive).is_file());

    let keys = BTreeMap::from([(
        "release".to_string(),
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes()),
    )]);
    let record_path = record.record_path(temp.path());
    verify_record_with(&record_path, Some(&keys), Some(template)).unwrap();
    // The default naming scheme must not accept the custom file name.
    assert!(verify_record(&record_path, &keys).is_err());
}

#[test]
fn unsigned_lock_has_no_public_keys_and_parses_as_a_runtime_lock() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = manifest_fixture(&temp, "dev-library");
    let library = temp.path().join("libdev-library.so");
    let record = write_package(&library, temp.path(), manifest, None).unwrap();
    verify_record_with(&record.record_path(temp.path()), None, None).unwrap();
    // A signature-requiring verification must still refuse the unsigned record.
    assert!(verify_record(&record.record_path(temp.path()), &BTreeMap::new()).is_err());

    let url_template = format!("file://{}/{{artifact}}", temp.path().display());
    let lock = build_unsigned_lock(std::slice::from_ref(&record), &url_template).unwrap();
    assert!(lock.public_keys.is_empty());
    assert_eq!(lock.artifacts.len(), 1);
    assert_eq!(lock.artifacts[0].key_id, "");
    assert_eq!(
        lock.artifacts[0].url,
        format!("file://{}/{}", temp.path().display(), record.archive)
    );

    // The runtime's lock type (deny_unknown_fields) accepts the serialized shape; with no keys
    // and an empty key_id its cache skips signature verification.
    let lock_path = temp.path().join("components.lock.json");
    write_lock(&lock_path, &lock).unwrap();
    let parsed: ComponentLock = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    assert_eq!(parsed, lock);
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    assert_eq!(raw["public_keys"], serde_json::json!({}));
}

#[test]
fn signed_lock_still_refuses_unsigned_records() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = manifest_fixture(&temp, "unsigned-library");
    let library = temp.path().join("libunsigned-library.so");
    let record = write_package(&library, temp.path(), manifest, None).unwrap();
    let keys = BTreeMap::from([("release".to_string(), "key".to_string())]);
    let error = build_lock(&[record], "https://example.invalid/{artifact}", &keys).unwrap_err();
    assert!(error.to_string().contains("is unsigned"), "{error}");
}

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

#[test]
fn signing_key_loads_from_an_environment_value_in_every_form() {
    let key = test_signing_key();
    let der = key.to_pkcs8_der().unwrap();
    let pem = pem_wrap("PRIVATE KEY", der.as_bytes());
    let b64_der = base64::engine::general_purpose::STANDARD.encode(der.as_bytes());
    let b64_seed = base64::engine::general_purpose::STANDARD.encode(key.to_bytes());
    for value in [pem, format!("  {b64_der}\n"), b64_seed] {
        let loaded = load_signing_key_from_env("RELEASE_KEY", |name| {
            assert_eq!(name, "RELEASE_KEY");
            Ok(value.clone())
        })
        .unwrap();
        assert_eq!(loaded.to_bytes(), key.to_bytes());
    }
}

#[test]
fn signing_key_env_errors_name_the_variable_and_never_default() {
    let missing = load_signing_key_from_env("RELEASE_KEY", |_| Err(std::env::VarError::NotPresent)).unwrap_err();
    assert!(missing.to_string().contains("`RELEASE_KEY` is not set"), "{missing}");
    let empty = load_signing_key_from_env("RELEASE_KEY", |_| Ok("  \n".into())).unwrap_err();
    assert!(empty.to_string().contains("is empty"), "{empty}");
    let secret = "c2VjcmV0LW5vdC1hLWtleQ==";
    let invalid = load_signing_key_from_env("RELEASE_KEY", |_| Ok(secret.into())).unwrap_err();
    assert!(!format!("{invalid:#}").contains(secret), "{invalid:#}");
}

#[test]
fn signing_key_loads_from_a_reader_pem_with_trailing_newline() {
    let key = test_signing_key();
    let mut pem = pem_wrap("PRIVATE KEY", key.to_pkcs8_der().unwrap().as_bytes());
    pem.push('\n');
    let loaded = load_signing_key_from_reader(pem.as_bytes(), "stdin").unwrap();
    assert_eq!(loaded.to_bytes(), key.to_bytes());
    let der = load_signing_key_from_reader(key.to_pkcs8_der().unwrap().as_bytes(), "stdin").unwrap();
    assert_eq!(der.to_bytes(), key.to_bytes());
    assert!(load_signing_key_from_reader(&b""[..], "stdin").is_err());
}

#[test]
fn signing_key_reader_rejects_oversized_input() {
    let oversized = vec![b'x'; (MAX_SIGNING_KEY_BYTES + 1) as usize];
    let error = load_signing_key_from_reader(oversized.as_slice(), "stdin").unwrap_err();
    assert!(error.to_string().contains("larger than"), "{error}");
}

#[test]
fn rotation_signature_from_either_key_verifies_against_a_multi_key_set() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = manifest_fixture(&temp, "rotated-library");
    let old = SigningKey::from_bytes(&[1; 32]);
    let new = SigningKey::from_bytes(&[2; 32]);
    let encode = |key: &SigningKey| base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());
    for (key, key_id) in [(&old, "release-2025"), (&new, "release-2026")] {
        let signature = sign_manifest_with_key(&manifest, key, key_id).unwrap();
        verify_manifest_signature(&manifest, &signature, &encode(key)).unwrap();
    }
    // A signature does not verify under the other key of the rotation pair.
    let signature = sign_manifest_with_key(&manifest, &old, "release-2025").unwrap();
    assert!(verify_manifest_signature(&manifest, &signature, &encode(&new)).is_err());
}
