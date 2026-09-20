use crate::loader::{decode_hash, validate_manifest};
use crate::manifest::COMPONENT_MANIFEST_SCHEMA;
use crate::{
    ArtifactCache, CachedArtifact, ComponentDeliveryMode, ComponentError, ComponentLock, ComponentLockEntry,
    ComponentRequirements, LoadedComponent, TrustPolicy,
};
use alef_component_abi::AlefHostApiV1;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// Where a configured component currently stands, independent of its on-disk location (see
/// [`ComponentManager::cache_path`] for that). Generated bindings expose this as both a
/// stable string tag and a numeric code (`alef_component_status`/`alef_component_status_code`
/// in `alef::backends::native_components`) so callers in any host language can branch on it
/// without parsing free-form text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentStatus {
    /// Loaded and its contracts activated in this process.
    Ready,
    /// Downloaded and verified on disk, but not yet loaded in this process.
    Cached,
    /// Not yet present in the on-disk cache.
    NotDownloaded,
    /// Linked into the binding ahead of time for this target; never downloaded or loaded
    /// through this manager.
    Bundled,
    /// This host/target does not support downloading native components at all.
    Unsupported { reason: String },
}

pub struct ComponentManager {
    cache: ArtifactCache,
    target: String,
    entries: HashMap<String, ComponentLockEntry>,
    loaded: Mutex<HashMap<String, Arc<LoadedComponent>>>,
    /// One lock per (component, contract) key, used to coalesce concurrent `ensure_contract`
    /// calls for the same key onto a single download-and-load instead of racing duplicates;
    /// a call for a different key gets its own lock and proceeds independently. See
    /// [`Self::contract_lock`].
    key_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Artifacts this process has already installed and verified, keyed by the lock entry's
    /// own content-addressed `sha256` digest. A component that provides more than one
    /// contract shares one artifact across several `ensure_contract` calls; without this, each
    /// call would re-read and re-hash the same (potentially large) dylib. On-disk tampering
    /// with an artifact after this process has verified it once is out of scope: entries here
    /// are trusted for the rest of the process, not re-checked against the disk.
    verified_artifacts: Mutex<HashMap<String, CachedArtifact>>,
    host: AlefHostApiV1,
}

// SAFETY: `LoadedComponent` is thread-safe metadata and the host callback
// context is required by the ABI to remain valid for calls from component
// threads. Mutation is protected by the manager mutex. ~keep
unsafe impl Send for ComponentManager {}
unsafe impl Sync for ComponentManager {}

impl ComponentManager {
    pub fn from_lock(
        lock: ComponentLock,
        cache_root: impl Into<PathBuf>,
        target: impl Into<String>,
        host: AlefHostApiV1,
    ) -> Result<Self, ComponentError> {
        if lock.schema_version != COMPONENT_MANIFEST_SCHEMA {
            return Err(ComponentError::UnsupportedLockSchema(lock.schema_version));
        }
        let target = target.into();
        let mut entries = HashMap::new();
        for artifact in lock
            .artifacts
            .into_iter()
            .filter(|artifact| artifact.identity.target == target)
        {
            let id = artifact.identity.component.clone();
            if entries.insert(id.clone(), artifact).is_some() {
                return Err(ComponentError::DuplicateLockEntry {
                    component_id: id,
                    target,
                });
            }
        }
        Ok(Self {
            cache: ArtifactCache::new(cache_root, TrustPolicy::EmbeddedKeys(lock.public_keys)),
            target,
            entries,
            loaded: Mutex::new(HashMap::new()),
            key_locks: Mutex::new(HashMap::new()),
            verified_artifacts: Mutex::new(HashMap::new()),
            host,
        })
    }

    /// Disable network downloads: [`Self::ensure_contract`] and [`Self::prefetch`] fail a
    /// missing artifact with [`ComponentError::Offline`] instead of attempting a request. An
    /// artifact already cached on disk is unaffected. Generated bindings call this when the
    /// crate-specific `{CRATE}_COMPONENT_OFFLINE` environment variable is set.
    #[must_use]
    pub fn offline(mut self, offline: bool) -> Self {
        self.cache = self.cache.offline(offline);
        self
    }

    /// Load one contract a component provides.
    ///
    /// `entry_symbol` is that contract's NUL-terminated producer entry-point
    /// symbol (`alef::codegen::component::entry_point_symbol`); the caller
    /// supplies it because computing it requires the same identifier-casing
    /// logic the producer used, which this crate does not depend on.
    ///
    /// Concurrent calls for the *same* `(component_id, contract_name)` coalesce onto one
    /// download-and-load: a second caller waits behind the first instead of racing a
    /// duplicate download or `dlopen`. A call for a different key is unaffected and proceeds
    /// immediately -- the mutex guarding the loaded-component map is only ever held for a
    /// map lookup or insert, never across I/O.
    pub fn ensure_contract(
        &self,
        component_id: &str,
        contract_name: &str,
        entry_symbol: &[u8],
    ) -> Result<Arc<LoadedComponent>, ComponentError> {
        let cache_key = loaded_cache_key(component_id, contract_name);
        if let Some(component) = self.loaded_component(&cache_key) {
            return Ok(component);
        }

        let key_lock = self.contract_lock(cache_key.clone());
        let _guard = key_lock.lock().unwrap_or_else(PoisonError::into_inner);
        // Re-check now that this call holds the per-key lock: a concurrent caller for the
        // same key may have finished the work while this call was waiting for it.
        if let Some(component) = self.loaded_component(&cache_key) {
            return Ok(component);
        }

        let entry = self.entry(component_id)?;
        let requirements = contract_requirements(entry, contract_name)?;
        let cached = self.cached_artifact(entry)?;
        validate_manifest(&cached.manifest, &requirements)?;
        let component = Arc::new(LoadedComponent::load(
            cached.library,
            entry_symbol,
            &requirements,
            self.host,
        )?);
        self.insert_loaded(cache_key, Arc::clone(&component));
        Ok(component)
    }

    pub fn prefetch(&self, component_ids: &[&str]) -> Result<Vec<CachedArtifact>, ComponentError> {
        component_ids
            .iter()
            .map(|component_id| self.cached_artifact(self.entry(component_id)?))
            .collect()
    }

    fn loaded_component(&self, cache_key: &str) -> Option<Arc<LoadedComponent>> {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(cache_key)
            .cloned()
    }

    fn insert_loaded(&self, cache_key: String, component: Arc<LoadedComponent>) {
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key, component);
    }

    /// The per-`cache_key` lock used to coalesce concurrent [`Self::ensure_contract`] calls.
    /// The outer `key_locks` mutex is only ever held for the lookup-or-insert below, never
    /// across the download/load work the returned lock guards.
    fn contract_lock(&self, cache_key: String) -> Arc<Mutex<()>> {
        Arc::clone(
            self.key_locks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(cache_key)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Install `entry`'s artifact, reusing an in-process-verified copy when this process has
    /// already installed it (see [`Self::verified_artifacts`]).
    fn cached_artifact(&self, entry: &ComponentLockEntry) -> Result<CachedArtifact, ComponentError> {
        if let Some(cached) = self
            .verified_artifacts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&entry.sha256)
            .cloned()
        {
            return Ok(cached);
        }
        let cached = self.cache.install(entry)?;
        self.verified_artifacts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(entry.sha256.clone(), cached.clone());
        Ok(cached)
    }

    /// Report where `component_id` currently stands. Unlike [`Self::ensure_contract`],
    /// [`Self::prefetch`], and [`Self::cache_path`], a `Bundled` entry is a legitimate answer
    /// here rather than a failure: it is this manager's way of saying "linked in ahead of
    /// time, never downloaded or loaded through me", not an error.
    pub fn status(&self, component_id: &str) -> Result<ComponentStatus, ComponentError> {
        let entry = self.lookup(component_id)?;
        if entry.mode == ComponentDeliveryMode::Bundled {
            return Ok(ComponentStatus::Bundled);
        }
        let loaded_prefix = format!("{component_id}::");
        if self
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .any(|key| key.starts_with(&loaded_prefix))
        {
            return Ok(ComponentStatus::Ready);
        }
        let root = self.cache.object_path(entry)?;
        if root.join("component.json").is_file() {
            Ok(ComponentStatus::Cached)
        } else {
            Ok(ComponentStatus::NotDownloaded)
        }
    }

    pub fn cache_path(&self, component_id: &str) -> Result<PathBuf, ComponentError> {
        self.cache.object_path(self.entry(component_id)?)
    }

    #[must_use]
    pub fn cache_root(&self) -> &Path {
        self.cache.root()
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Look up a component's lock entry for this manager's target, with no restriction on
    /// its delivery mode. [`Self::status`] uses this directly since a `Bundled` entry is a
    /// legitimate status to report, not a failure.
    fn lookup(&self, component_id: &str) -> Result<&ComponentLockEntry, ComponentError> {
        self.entries
            .get(component_id)
            .ok_or_else(|| ComponentError::ArtifactNotFound {
                component_id: component_id.to_owned(),
                target: self.target.clone(),
            })
    }

    /// Like [`Self::lookup`], but additionally rejects a `Bundled` entry -- one this manager
    /// cannot download -- before any cache I/O runs. Every caller that actually needs to
    /// touch the cache (`ensure_contract`, `prefetch`, `cache_path`) reaches it only through
    /// this method.
    fn entry(&self, component_id: &str) -> Result<&ComponentLockEntry, ComponentError> {
        let entry = self.lookup(component_id)?;
        if entry.mode == ComponentDeliveryMode::Bundled {
            return Err(ComponentError::BundledComponentNotLoadable {
                component: component_id.to_owned(),
            });
        }
        Ok(entry)
    }
}

fn loaded_cache_key(component_id: &str, contract_name: &str) -> String {
    format!("{component_id}::{contract_name}")
}

fn contract_requirements(
    entry: &ComponentLockEntry,
    contract_name: &str,
) -> Result<ComponentRequirements, ComponentError> {
    let provided = entry
        .provides
        .iter()
        .find(|provided| provided.contract == contract_name)
        .ok_or_else(|| ComponentError::ContractNotProvided {
            component_id: entry.identity.component.clone(),
            contract: contract_name.to_owned(),
        })?;
    Ok(ComponentRequirements {
        component_id: entry.identity.component.clone(),
        contract_name: contract_name.to_owned(),
        contract_hash: decode_hash(&provided.contract_hash)?,
        feature_set_hash: Some(decode_hash(&entry.identity.feature_hash)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ComponentIdentity;
    use std::collections::BTreeMap;
    use std::ffi::c_void;

    fn host() -> AlefHostApiV1 {
        AlefHostApiV1 {
            struct_size: 0,
            abi_major: 0,
            abi_minor: 0,
            context: core::ptr::null_mut::<c_void>(),
            log: None,
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
            }],
        }
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
        manager
            .verified_artifacts
            .lock()
            .unwrap()
            .insert(entry.sha256.clone(), fake.clone());

        // The lock entry's URL points nowhere real (`file:///does/not/exist`, see `lock()`),
        // so this call would fail if it attempted to actually install/verify the artifact;
        // succeeding proves it reused the in-process cache instead.
        let prefetched = manager.prefetch(&["demo"]).unwrap();
        assert_eq!(prefetched, vec![fake]);
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
}
