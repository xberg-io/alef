use super::*;
use crate::ComponentIdentity;
use std::collections::BTreeMap;

fn host() -> AlefHostApiV1 {
    AlefHostApiV1 {
        struct_size: 0,
        abi_major: 0,
        abi_minor: 0,
        ..AlefHostApiV1::EMPTY
    }
}

fn lock(target: &str) -> ComponentLock {
    lock_with_mode(target, ComponentDeliveryMode::Download)
}

fn lock_with_mode(target: &str, mode: ComponentDeliveryMode) -> ComponentLock {
    ComponentLock {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        public_keys: BTreeMap::new(),
        artifacts: vec![ComponentLockEntry {
            identity: ComponentIdentity {
                crate_name: "demo-core".into(),
                component: "demo".into(),
                version: "1.0.0".into(),
                target: target.into(),
                feature_hash: hex::encode([2; 32]),
                contract_hash: hex::encode([1; 32]),
            },
            provides: vec![crate::ComponentProvidedContract {
                contract: "engine".into(),
                interface_version: 1,
                contract_hash: hex::encode([1; 32]),
                implementation: "demo::Demo".into(),
            }],
            mode,
            url: "file:///does/not/exist".into(),
            sha256: hex::encode([3; 32]),
            size: 0,
            manifest_sha256: hex::encode([4; 32]),
            key_id: String::new(),
            input_hash: None,
        }],
    }
}

#[test]
fn bootstrap_empty_lock_reports_components_not_locked_instead_of_artifact_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let empty = ComponentLock {
        schema_version: COMPONENT_MANIFEST_SCHEMA,
        public_keys: BTreeMap::new(),
        artifacts: Vec::new(),
    };
    let manager = ComponentManager::from_lock(empty, dir.path(), "target-a", host()).unwrap();

    assert!(matches!(
        manager.status("demo"),
        Err(ComponentError::ComponentsNotLocked { .. })
    ));
    assert!(matches!(
        manager.ensure_contract("demo", "engine", b"irrelevant\0"),
        Err(ComponentError::ComponentsNotLocked { .. })
    ));
    assert!(matches!(
        manager.prefetch(&["demo"]),
        Err(ComponentError::ComponentsNotLocked { .. })
    ));
    assert!(matches!(
        manager.cache_path("demo"),
        Err(ComponentError::ComponentsNotLocked { .. })
    ));
}

#[test]
fn manager_filters_lock_to_exact_target() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-b", host()).unwrap();
    assert!(matches!(
        manager.status("demo"),
        Err(ComponentError::ArtifactNotFound { .. })
    ));
}

#[test]
fn manager_reports_content_address_before_download() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::NotDownloaded);
    assert!(
        manager
            .cache_path("demo")
            .unwrap()
            .ends_with(format!("sha256/{}", hex::encode([3; 32])))
    );
}

#[test]
fn manager_names_objects_by_lower_case_digest() {
    let dir = tempfile::tempdir().unwrap();
    let mut upper = lock("target-a");
    upper.artifacts[0].sha256.make_ascii_uppercase();
    let manager = ComponentManager::from_lock(upper, dir.path(), "target-a", host()).unwrap();
    assert!(
        manager
            .cache_path("demo")
            .unwrap()
            .ends_with(format!("sha256/{}", hex::encode([3; 32])))
    );
    assert_eq!(manager.lock_entry("demo").unwrap().sha256, hex::encode([3; 32]));
}

#[test]
fn duplicate_target_entry_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut lock = lock("target-a");
    lock.artifacts.push(lock.artifacts[0].clone());
    assert!(matches!(
        ComponentManager::from_lock(lock, dir.path(), "target-a", host()),
        Err(ComponentError::DuplicateLockEntry { .. })
    ));
}

#[test]
fn ensure_contract_rejects_a_contract_the_component_does_not_provide() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
    assert!(matches!(
        manager.ensure_contract("demo", "missing-contract", b"irrelevant\0"),
        Err(ComponentError::ContractNotProvided { .. })
    ));
}

#[test]
fn ensure_contract_reaches_the_download_step_for_a_provided_contract() {
    let dir = tempfile::tempdir().unwrap();
    let mut two_contracts = lock("target-a");
    two_contracts.artifacts[0]
        .provides
        .push(crate::ComponentProvidedContract {
            contract: "second".into(),
            interface_version: 1,
            contract_hash: hex::encode([9; 32]),
            implementation: "demo::Second".into(),
        });
    // A real missing path rather than `lock()`'s `file:///does/not/exist`, which has no drive
    // letter and so is not a local file URL on Windows.
    two_contracts.artifacts[0].url = url::Url::from_file_path(dir.path().join("missing.tar.gz"))
        .unwrap()
        .to_string();
    let manager = ComponentManager::from_lock(two_contracts, dir.path(), "target-a", host()).unwrap();
    // Fails to find the library at the fake `file://` URL -- proving contract resolution
    // for "second" succeeded and the manager reached the download step, not that the
    // download itself succeeded.
    assert!(matches!(
        manager.ensure_contract("demo", "second", b"alef_component_entry_v1_second\0"),
        Err(ComponentError::Io(_))
    ));
}

#[test]
fn bundled_component_is_rejected_before_touching_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(
        lock_with_mode("target-a", ComponentDeliveryMode::Bundled),
        dir.path(),
        "target-a",
        host(),
    )
    .unwrap();

    assert!(matches!(
        manager.ensure_contract("demo", "engine", b"alef_component_entry_v1_engine\0"),
        Err(ComponentError::BundledComponentNotLoadable { .. })
    ));
    // Unlike `ensure_contract`/`prefetch`/`cache_path`, `status` reports `Bundled` as a
    // legitimate answer rather than failing: this manager genuinely cannot download or
    // load the component, and that is exactly what the status says.
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Bundled);
    assert!(matches!(
        manager.prefetch(&["demo"]),
        Err(ComponentError::BundledComponentNotLoadable { .. })
    ));
    assert!(matches!(
        manager.cache_path("demo"),
        Err(ComponentError::BundledComponentNotLoadable { .. })
    ));
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "a bundled component must never touch the cache directory"
    );
}

#[test]
fn prefetch_reuses_an_already_verified_artifact_without_reinstalling() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
    let entry = manager.entries.get("demo").unwrap().clone();

    let fake = CachedArtifact {
        root: dir.path().join("fake-root"),
        library: dir.path().join("fake-root/libdemo.so"),
        manifest: crate::ComponentManifest {
            schema_version: COMPONENT_MANIFEST_SCHEMA,
            abi_version: 1,
            identity: entry.identity.clone(),
            provides: entry.provides.clone(),
            features: Vec::new(),
            default_features: false,
            library: crate::ComponentLibrary {
                file: "libdemo.so".into(),
                sha256: hex::encode([0; 32]),
                size: 0,
            },
        },
    };
    std::fs::create_dir_all(&fake.root).unwrap();
    std::fs::write(&fake.library, b"").unwrap();
    let file = std::fs::File::open(&fake.library).unwrap();
    let metadata = file.metadata().unwrap();
    let verified = VerifiedFile::new(fake.library.clone(), file, &metadata);
    manager
        .verified_artifacts
        .lock()
        .unwrap()
        .insert(entry.sha256.clone(), (fake.clone(), verified));

    // The lock entry's URL points nowhere real (`file:///does/not/exist`, see `lock()`),
    // so this call would fail if it attempted to actually install/verify the artifact;
    // succeeding proves it reused the in-process cache instead.
    let prefetched = manager.prefetch(&["demo"]).unwrap();
    assert_eq!(prefetched, vec![fake]);
}

#[test]
fn lock_entry_exposes_version_digest_and_size() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
    let entry = manager.lock_entry("demo").unwrap();
    assert_eq!(entry.identity.version, "1.0.0");
    assert_eq!(entry.sha256, hex::encode([3; 32]));
    assert_eq!(manager.lock_entries().len(), 1);
    assert!(matches!(
        manager.lock_entry("missing"),
        Err(ComponentError::ArtifactNotFound { .. })
    ));
}

#[test]
fn list_remove_and_prune_operate_on_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
    let ours = dir.path().join("sha256").join(hex::encode([3; 32]));
    let foreign = dir.path().join("sha256").join(hex::encode([7; 32]));
    for path in [&ours, &foreign] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("component.json"), b"{}").unwrap();
    }

    let listed = manager.list_cached().unwrap();
    assert_eq!(listed.len(), 2);
    let named: Vec<_> = listed.iter().filter_map(|object| object.component.as_deref()).collect();
    assert_eq!(named, vec!["demo"]);

    let report = manager.prune(&Retain::new().referenced_by_lock()).unwrap();
    assert_eq!(report.removed, vec![foreign.clone()]);
    assert!(ours.exists() && !foreign.exists());

    let report = manager.remove("demo").unwrap();
    assert_eq!(report.removed, vec![ours.clone()]);
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::NotDownloaded);
}

#[test]
fn mirror_base_keeps_only_the_archive_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut remote = lock("target-a");
    remote.artifacts[0].url = "https://origin.example/dl/demo-1.0.0.tar.gz?token=1".into();
    let manager = ComponentManager::from_lock(remote, dir.path(), "target-a", host())
        .unwrap()
        .offline(true)
        .with_mirror_base("https://mirror.example/components/");

    let error = manager.prefetch(&["demo"]).unwrap_err();

    assert!(matches!(
        error,
        ComponentError::Offline { ref url, .. }
            if url == "https://mirror.example/components/demo-1.0.0.tar.gz"
    ));
}

#[test]
fn contract_lock_coalesces_the_same_key_and_separates_different_keys() {
    let dir = tempfile::tempdir().unwrap();
    let manager = ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();

    let first = manager.contract_lock("demo::engine".to_string());
    let second = manager.contract_lock("demo::engine".to_string());
    assert!(
        Arc::ptr_eq(&first, &second),
        "the same (component, contract) key must coalesce onto one lock"
    );

    let unrelated = manager.contract_lock("other::engine".to_string());
    assert!(
        !Arc::ptr_eq(&first, &unrelated),
        "an unrelated key must get its own lock so it is never blocked by a different key"
    );
}

#[test]
fn concurrent_calls_for_the_same_key_serialize_on_the_coalescing_lock() {
    use std::sync::Barrier;
    use std::thread;

    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap());
    let order = Arc::new(Mutex::new(Vec::new()));
    let ready = Arc::new(Barrier::new(2));

    // Hold the coalescing lock as a stand-in for an in-progress download, then prove a
    // concurrent request for the *same* key really does wait for it: `order` only ever
    // records the second entry-point-symbol lookup after the first thread has both run
    // and released the lock, never interleaved with it.
    let held = manager.contract_lock("demo::engine".to_string());
    let guard = held.lock().unwrap();

    let waiter = {
        let manager = Arc::clone(&manager);
        let order = Arc::clone(&order);
        let ready = Arc::clone(&ready);
        thread::spawn(move || {
            ready.wait();
            let _ = manager.ensure_contract("demo", "engine", b"alef_component_entry_v1_engine\0");
            order.lock().unwrap().push("waiter");
        })
    };

    ready.wait();
    thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        order.lock().unwrap().is_empty(),
        "a concurrent call for the same key must not proceed while the lock is held"
    );
    order.lock().unwrap().push("holder");
    drop(guard);

    waiter.join().unwrap();
    assert_eq!(*order.lock().unwrap(), vec!["holder", "waiter"]);
}

fn signed(mut lock: ComponentLock) -> ComponentLock {
    lock.public_keys
        .insert("release".into(), "unused-in-these-tests".into());
    for artifact in &mut lock.artifacts {
        artifact.key_id = "release".into();
    }
    lock
}

fn strict(lock: ComponentLock) -> Result<ComponentManager, ComponentError> {
    let dir = tempfile::tempdir().unwrap();
    ComponentManager::from_lock_with_signature_policy(lock, dir.path(), "target-a", host(), SignaturePolicy::Require)
}

#[test]
fn the_default_policy_still_accepts_an_unsigned_lock() {
    let dir = tempfile::tempdir().unwrap();
    ComponentManager::from_lock(lock("target-a"), dir.path(), "target-a", host()).unwrap();
}

#[test]
fn requiring_signatures_rejects_a_lock_without_public_keys() {
    let error = strict(lock("target-a")).err().unwrap();
    assert!(
        matches!(error, ComponentError::UnsignedLock(ref reason) if reason.contains("no public keys")),
        "{error}"
    );
    assert_eq!(error.code(), "unsigned_lock");
}

#[test]
fn requiring_signatures_rejects_an_artifact_without_a_key_id_on_any_target() {
    let mut lock = signed(lock("target-a"));
    let mut other = lock.artifacts[0].clone();
    other.identity.target = "target-b".into();
    other.key_id = String::new();
    lock.artifacts.push(other);
    assert!(matches!(
        strict(lock),
        Err(ComponentError::UnsignedLock(reason)) if reason.contains("target-b") && reason.contains("no key_id")
    ));
}

#[test]
fn requiring_signatures_rejects_a_key_id_the_lock_does_not_embed() {
    let mut lock = signed(lock("target-a"));
    lock.artifacts[0].key_id = "rotated".into();
    assert!(matches!(
        strict(lock),
        Err(ComponentError::UnsignedLock(reason)) if reason.contains("rotated")
    ));
}

#[test]
fn requiring_signatures_accepts_a_signed_lock_and_ignores_bundled_entries() {
    let mut lock = signed(lock("target-a"));
    let mut bundled = lock.artifacts[0].clone();
    bundled.identity.target = "target-b".into();
    bundled.mode = ComponentDeliveryMode::Bundled;
    bundled.key_id = String::new();
    lock.artifacts.push(bundled);
    strict(lock).unwrap();
}
