use crate::host_services::ServicesHandle;
use crate::identity::{FileIdentity, VerifiedFile};
use crate::stdio::SidecarProcess;
use crate::{COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentError, ComponentManifest, HostServices};
use alef_component_abi::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AlefComponentEntryV1, AlefComponentV1, AlefContract, AlefHostApiV1, AlefOwnedBuffer,
};
use libloading::Library;
use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

const MAX_DESCRIPTOR_STRING: usize = 16 * 1024;

/// What a loaded component's descriptor must match.
///
/// `contract_name` identifies which of a component's `provides` entries this
/// is for; `contract_hash` is that specific contract's hash, not the
/// component-wide legacy `ComponentIdentity::contract_hash`. A component that
/// provides several contracts is loaded once per contract, each through its
/// own entry point (see [`crate::ComponentManager::ensure_contract`]), so each
/// load only ever validates one contract's requirements.
#[derive(Clone, Debug)]
pub struct ComponentRequirements {
    pub component_id: String,
    pub contract_name: String,
    pub contract_hash: [u8; 32],
    pub feature_set_hash: Option<[u8; 32]>,
}

pub struct LoadedComponent {
    library: Option<Arc<Library>>,
    sidecar: Option<(Arc<SidecarProcess>, String)>,
    _sidecar_descriptor: Option<Box<AlefComponentV1>>,
    descriptor: NonNull<AlefComponentV1>,
    host: Box<AlefHostApiV1>,
    /// Keeps the context behind the host table's service callbacks alive with the component.
    _services: Option<ServicesHandle>,
    component_id: String,
    component_version: String,
}

/// A live instance created by a component, paired with its typed C function table.
///
/// Dropping the instance invokes the component's own destructor while the
/// originating dynamic library remains pinned.
pub struct ComponentInstance<T: AlefContract> {
    component: Arc<LoadedComponent>,
    handle: NonNull<c_void>,
    _contract: PhantomData<T>,
}

// SAFETY: native metadata is static and pinned; sidecar metadata is owned and immutable,
// and its process synchronizes concurrent calls internally. Host callback context thread-safety is part of the
// `AlefHostApiV1` contract supplied by the caller. ~keep
unsafe impl Send for LoadedComponent {}
// SAFETY: as for `Send` above; every shared access is a read of immutable metadata or goes
// through the sidecar process's own synchronization.
unsafe impl Sync for LoadedComponent {}

// SAFETY: the C ABI requires every contract method to take `&self` (see
// `component::map_method`) and calls it through a plain function pointer, so the
// component's own implementation is the only thing that can make concurrent access
// unsound; the ABI async wrapper already calls back into an instance from a spawned
// worker thread, so components are contractually required to tolerate this. The
// instance handle itself is owned exclusively by this `ComponentInstance` for its
// lifetime, with no interior aliasing on the host side. ~keep
unsafe impl<T: AlefContract> Send for ComponentInstance<T> {}
// SAFETY: as for `Send` above; `&self` methods only read the handle and the validated table,
// and concurrent calls through the table are the component's contractual obligation.
unsafe impl<T: AlefContract> Sync for ComponentInstance<T> {}

impl LoadedComponent {
    /// Load one contract's descriptor from a component's shared library.
    ///
    /// A component that provides several contracts exports one entry point
    /// per contract (`alef_component_entry_v1_<contract>`, see
    /// `alef::codegen::component::entry_point_symbol`); `entry_symbol` is that
    /// NUL-terminated symbol name, so this loads exactly one contract's
    /// descriptor per call even when the underlying `.so`/`.dylib`/`.dll` is
    /// shared across several `LoadedComponent`s.
    ///
    /// `services` is published through the host table's `service_call` / `service_notify`
    /// callbacks. With `None` the service fields of `host` are passed through unchanged (null by
    /// default, so components see no service table).
    ///
    /// Crate-private: loading runs the library's initializers and entry point, so the only
    /// caller is [`crate::ComponentManager`], which loads nothing it has not verified against
    /// the lock first.
    pub(crate) fn load_with_services(
        verified: &VerifiedFile,
        entry_symbol: &[u8],
        requirements: &ComponentRequirements,
        mut host: AlefHostApiV1,
        services: Option<Arc<dyn HostServices>>,
    ) -> Result<Self, ComponentError> {
        host.struct_size = core::mem::size_of::<AlefHostApiV1>();
        host.abi_major = ABI_MAJOR_V1;
        host.abi_minor = ABI_MINOR_V1;
        let services = services.map(ServicesHandle::new);
        if let Some(handle) = &services {
            handle.install(&mut host);
        }
        let host = Box::new(host);

        // The verified descriptor's path on Linux, else the checked path (see `identity`).
        let load_path = verified.library_load_path()?;
        let library = Arc::new(
            // SAFETY: loading runs the library's initializers. `load_path` names the file the
            // cache verified against the signed lock (its descriptor on Linux, a re-checked path
            // elsewhere), which is the trust decision this crate makes for native code.
            unsafe { Library::new(&load_path) }.map_err(|error| ComponentError::LibraryLoad {
                path: verified.path().to_owned(),
                message: error.to_string(),
            })?,
        );
        // SAFETY: `entry_symbol` names an `alef_component_entry_v1*` export, whose signature the
        // ABI fixes as `AlefComponentEntryV1`; `library` outlives the symbol, and `host` is a live
        // `AlefHostApiV1` the call only borrows. The returned pointer is checked for null here and
        // for size and contents below before any reference is formed over it.
        let descriptor = unsafe {
            let entry: libloading::Symbol<'_, AlefComponentEntryV1> = library
                .get(entry_symbol)
                .map_err(|error| ComponentError::MissingEntrypoint(error.to_string()))?;
            let mut raw = core::ptr::null();
            let status = entry(ABI_MAJOR_V1, host.as_ref(), &mut raw);
            if !status.is_ok() {
                return Err(ComponentError::EntrypointFailed(status.0));
            }
            NonNull::new(raw.cast_mut()).ok_or(ComponentError::InvalidDescriptor("null descriptor"))?
        };

        // Reject a too-small descriptor before a `&AlefComponentV1` reference is ever formed
        // over it: forming that reference asserts the pointee is valid for
        // `size_of::<AlefComponentV1>()` bytes, which is undefined behavior when the
        // component actually returned a shorter, older-ABI struct.
        // SAFETY: the entry point returned this non-null pointer as its descriptor, which the ABI
        // requires to start with a readable `struct_size`, and no reference to it exists yet.
        unsafe { reject_undersized_descriptor(descriptor)? };
        // SAFETY: the descriptor is at least `size_of::<AlefComponentV1>()` bytes (checked above)
        // and static in the pinned library; its string fields are the component's own static
        // metadata, as `validate_descriptor` requires.
        let validated = unsafe { validate_descriptor(descriptor.as_ref(), requirements)? };
        pin_for_process(verified.identity(), Arc::clone(&library));
        Ok(Self {
            library: Some(library),
            sidecar: None,
            _sidecar_descriptor: None,
            descriptor,
            host,
            _services: services,
            component_id: validated.0,
            component_version: validated.1,
        })
    }

    pub(crate) fn from_sidecar(
        process: Arc<SidecarProcess>,
        manifest: &ComponentManifest,
        requirements: &ComponentRequirements,
        host: AlefHostApiV1,
    ) -> Result<Self, ComponentError> {
        let component_id = manifest.identity.component.clone();
        let component_version = manifest.identity.version.clone();
        let mut descriptor = Box::new(AlefComponentV1 {
            struct_size: core::mem::size_of::<AlefComponentV1>(),
            abi_major: ABI_MAJOR_V1,
            abi_minor: ABI_MINOR_V1,
            component_id: alef_component_abi::AlefStr {
                ptr: component_id.as_ptr().cast(),
                len: component_id.len(),
            },
            component_version: alef_component_abi::AlefStr {
                ptr: component_version.as_ptr().cast(),
                len: component_version.len(),
            },
            contract_hash: requirements.contract_hash,
            feature_set_hash: decode_hash(&manifest.identity.feature_hash)?,
            contract: core::ptr::null(),
            contract_size: 0,
            create: None,
            destroy: None,
        });
        Ok(Self {
            library: None,
            sidecar: Some((process, requirements.contract_name.clone())),
            descriptor: NonNull::from(descriptor.as_mut()),
            _sidecar_descriptor: Some(descriptor),
            host: Box::new(host),
            _services: None,
            component_id,
            component_version,
        })
    }

    pub(crate) fn ensure_active(&self) -> Result<(), ComponentError> {
        if let Some((process, _)) = &self.sidecar {
            process.ensure_running()?;
        }
        Ok(())
    }

    /// Passive liveness probe: true when a sidecar connection is currently running, with no
    /// healing side effect — unlike [`Self::ensure_active`], it never respawns. Status reporting
    /// observes; only a real call may spend the restart budget.
    pub(crate) fn is_active(&self) -> bool {
        match &self.sidecar {
            Some((process, _)) => process.is_running(),
            None => true,
        }
    }

    /// Invoke a named bytes-only method on this contract's stdio component.
    ///
    /// Native typed function tables require their generated bindings and do not implement
    /// this operation. Stdio components likewise cannot expose typed C function tables.
    pub fn invoke_bytes(&self, method: &str, payload: &[u8]) -> Result<Vec<u8>, ComponentError> {
        let (process, contract) = self.sidecar.as_ref().ok_or(ComponentError::UnsupportedTransport(
            "bytes invocation requires a stdio component",
        ))?;
        process.invoke(contract, method, payload)
    }

    #[must_use]
    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    #[must_use]
    pub fn component_version(&self) -> &str {
        &self.component_version
    }

    #[must_use]
    pub fn descriptor(&self) -> &AlefComponentV1 {
        // SAFETY: a native descriptor was size-checked at load and lives in a library pinned for
        // the process; a sidecar descriptor is owned by `self._sidecar_descriptor`.
        unsafe { self.descriptor.as_ref() }
    }

    #[must_use]
    pub fn contract(&self) -> *const c_void {
        self.descriptor().contract
    }

    #[must_use]
    pub fn contract_size(&self) -> usize {
        self.descriptor().contract_size
    }

    pub fn contract_table<T: AlefContract>(&self) -> Result<&T, ComponentError> {
        if self.sidecar.is_some() {
            return Err(ComponentError::UnsupportedTransport(
                "stdio components support bytes invocation, not typed C function tables",
            ));
        }
        contract_table_from_descriptor(self.descriptor())
    }

    /// Create an instance and bind it to a generated contract-table type.
    pub fn instantiate<T: AlefContract>(self: &Arc<Self>) -> Result<ComponentInstance<T>, ComponentError> {
        self.contract_table::<T>()?;
        let create = self.descriptor().create.ok_or(ComponentError::MissingInstanceFactory)?;
        self.descriptor()
            .destroy
            .ok_or(ComponentError::MissingInstanceDestructor)?;
        let mut raw = core::ptr::null_mut();
        let mut error = AlefOwnedBuffer::EMPTY;
        // SAFETY: `create` comes from the validated descriptor of a pinned library and takes the
        // host table this component was loaded with plus two valid out-pointers.
        let status = unsafe { create(self.host_api(), &mut raw, &mut error) };
        if !status.is_ok() {
            return Err(ComponentError::InstanceCreation {
                status: status.0,
                // SAFETY: the component filled `error` (or left it `EMPTY`) as the ABI's owned
                // error buffer for this failed call.
                message: unsafe { take_owned_buffer(error) },
            });
        }
        let handle = NonNull::new(raw).ok_or(ComponentError::NullInstance)?;
        Ok(ComponentInstance {
            component: Arc::clone(self),
            handle,
            _contract: PhantomData,
        })
    }

    #[must_use]
    pub fn host_api(&self) -> &AlefHostApiV1 {
        &self.host
    }

    #[must_use]
    pub fn strong_library_references(&self) -> usize {
        self.library.as_ref().map_or(0, Arc::strong_count)
    }
}

impl<T: AlefContract> ComponentInstance<T> {
    #[must_use]
    pub fn table(&self) -> &T {
        // The table was validated before this instance was constructed.
        self.component
            .contract_table::<T>()
            .expect("validated component contract table changed")
    }

    #[must_use]
    pub fn handle(&self) -> *mut c_void {
        self.handle.as_ptr()
    }

    #[must_use]
    pub fn component(&self) -> &Arc<LoadedComponent> {
        &self.component
    }
}

impl<T: AlefContract> Drop for ComponentInstance<T> {
    fn drop(&mut self) {
        if let Some(destroy) = self.component.descriptor().destroy {
            // SAFETY: `handle` came from this component's `create` and is destroyed exactly once,
            // here; the library stays loaded through `self.component`.
            unsafe { destroy(self.handle.as_ptr()) };
        }
    }
}

/// Copy an owned buffer's bytes into a `Vec<u8>` and release the original
/// allocation through its own `free` callback.
///
/// Generated contract proxies use this to convert a component's
/// `AlefOwnedBuffer` results into owned Rust values without re-implementing
/// buffer ownership handling in every generated snippet.
///
/// # Safety
///
/// `buffer` must be a valid `AlefOwnedBuffer`: `ptr` must be non-null and
/// valid for `len` bytes whenever `len > 0`, and `free` (if present) must be
/// safe to call with this buffer's own `context`, `ptr`, `len`, and
/// `capacity`.
#[must_use]
pub unsafe fn take_owned_bytes(buffer: AlefOwnedBuffer) -> Vec<u8> {
    let bytes = if buffer.ptr.is_null() || buffer.len == 0 {
        Vec::new()
    } else {
        // SAFETY: upheld by this function's caller contract.
        unsafe { core::slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec()
    };
    if let Some(free) = buffer.free {
        // SAFETY: upheld by this function's caller contract.
        unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
    }
    bytes
}

/// Like [`take_owned_bytes`], but lossily decodes the bytes as UTF-8.
///
/// Generated proxies use this to turn a component's error buffer into a
/// `String` for constructing the contract's error type; a component that
/// returns non-UTF-8 error text gets a replacement-character message instead
/// of a panic or a dropped allocation.
///
/// # Safety
///
/// Same obligations as [`take_owned_bytes`].
#[must_use]
pub unsafe fn take_owned_text(buffer: AlefOwnedBuffer) -> String {
    // SAFETY: upheld by this function's caller contract.
    String::from_utf8_lossy(&unsafe { take_owned_bytes(buffer) }).into_owned()
}

/// Decode a component's error buffer as text and release it, tolerating a null pointer.
///
/// # Safety
///
/// When `buffer.ptr` is non-null it must be valid for reads of `buffer.len` bytes, and `free`
/// (if present) must be safe to call once with this buffer's own `context`, `ptr`, `len` and
/// `capacity`.
unsafe fn take_owned_buffer(buffer: AlefOwnedBuffer) -> String {
    let message = if buffer.ptr.is_null() {
        if buffer.len == 0 {
            String::new()
        } else {
            "component returned an invalid error buffer".to_string()
        }
    } else {
        // SAFETY: non-null and valid for `len` bytes per this function's contract.
        String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(buffer.ptr, buffer.len) }).into_owned()
    };
    if let Some(free) = buffer.free {
        // SAFETY: called once with the buffer's own fields, per this function's contract.
        unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
    }
    message
}

fn contract_table_from_descriptor<T: AlefContract>(descriptor: &AlefComponentV1) -> Result<&T, ComponentError> {
    if descriptor.contract_hash != T::CONTRACT_HASH
        || descriptor.contract_size < core::mem::size_of::<T>()
        || descriptor.contract.is_null()
        || !(descriptor.contract as usize).is_multiple_of(core::mem::align_of::<T>())
    {
        return Err(ComponentError::InvalidContractTable);
    }
    // SAFETY: the table is non-null, aligned for `T`, at least `size_of::<T>()` bytes, and carries
    // `T`'s contract hash, so it is the `repr(C)` table `T` describes; it lives in a pinned
    // library for as long as `descriptor` is borrowed.
    Ok(unsafe { &*descriptor.contract.cast::<T>() })
}

/// Libraries kept loaded for the rest of the process, one per verified file: function pointers
/// and registered providers may outlive every `LoadedComponent` that handed them out.
fn process_pins() -> &'static Mutex<HashMap<FileIdentity, Arc<Library>>> {
    static PINS: OnceLock<Mutex<HashMap<FileIdentity, Arc<Library>>>> = OnceLock::new();
    PINS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pin_for_process(identity: FileIdentity, library: Arc<Library>) {
    pin_into(
        &mut process_pins().lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
        identity,
        library,
    );
}

/// Pin `library` unless the same file is already pinned; returns whether it was added. Loading
/// another contract from an already-loaded library returns the same OS handle, so one pin per file
/// is enough, and a long-lived host that re-ensures components no longer grows the list per load.
fn pin_into(pins: &mut HashMap<FileIdentity, Arc<Library>>, identity: FileIdentity, library: Arc<Library>) -> bool {
    match pins.entry(identity) {
        std::collections::hash_map::Entry::Occupied(_) => false,
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(library);
            true
        }
    }
}

pub(crate) fn validate_manifest(
    manifest: &ComponentManifest,
    requirements: &ComponentRequirements,
) -> Result<(), ComponentError> {
    if manifest.schema_version != COMPONENT_MANIFEST_SCHEMA || manifest.abi_version != COMPONENT_ABI_VERSION {
        return Err(ComponentError::UnsupportedManifest);
    }
    if manifest.identity.component != requirements.component_id {
        return Err(ComponentError::IdentityMismatch {
            expected: requirements.component_id.clone(),
            actual: manifest.identity.component.clone(),
        });
    }
    let provided = manifest
        .provides
        .iter()
        .find(|provided| provided.contract == requirements.contract_name)
        .ok_or_else(|| ComponentError::ContractNotProvided {
            component_id: requirements.component_id.clone(),
            contract: requirements.contract_name.clone(),
        })?;
    if decode_hash(&provided.contract_hash)? != requirements.contract_hash {
        return Err(ComponentError::ContractHashMismatch);
    }
    if let Some(expected) = requirements.feature_set_hash
        && decode_hash(&manifest.identity.feature_hash)? != expected
    {
        return Err(ComponentError::FeatureSetHashMismatch);
    }
    Ok(())
}

// `struct_size` must be a descriptor's first field: only that guarantees a raw read of it
// never touches memory beyond what the component actually allocated, even when the
// allocation is shorter than `size_of::<AlefComponentV1>()`. If this ever fails, the raw
// read in `reject_undersized_descriptor` is no longer sound and must be revisited.
const _: () = assert!(core::mem::offset_of!(AlefComponentV1, struct_size) == 0);

/// Read a just-returned descriptor's `struct_size` and reject it while it is still smaller
/// than the current ABI, before a `&AlefComponentV1` reference is formed over it.
///
/// # Safety
///
/// `descriptor` must be non-null and point to memory valid for reads of at least
/// `size_of::<usize>()` bytes at `AlefComponentV1`'s alignment; no wider reference to it may
/// have been formed yet. Both are upheld by [`LoadedComponent::load`]'s caller contract: a
/// component's entry point must return a pointer to at least a valid `struct_size` field.
unsafe fn reject_undersized_descriptor(descriptor: NonNull<AlefComponentV1>) -> Result<(), ComponentError> {
    // SAFETY: upheld by this function's caller contract; `struct_size` sits at offset 0
    // (asserted above), so this read never reaches past the caller-guaranteed
    // `size_of::<usize>()` bytes regardless of how large the real descriptor is.
    let struct_size = unsafe { descriptor.as_ptr().cast::<usize>().read_unaligned() };
    if struct_size < core::mem::size_of::<AlefComponentV1>() {
        return Err(ComponentError::InvalidDescriptor("descriptor is smaller than ABI v1"));
    }
    Ok(())
}

/// Check a loaded descriptor against `requirements` and copy out its id and version.
///
/// # Safety
///
/// Unsafe despite taking a reference: `component_id` and `component_version` are raw `AlefStr`s,
/// and each must be valid for reads of its `len` bytes whenever `len` is non-zero and at most
/// [`MAX_DESCRIPTOR_STRING`]; a reference to the descriptor says nothing about what they point
/// to. A component's entry point guarantees this for the descriptor it returns.
unsafe fn validate_descriptor(
    descriptor: &AlefComponentV1,
    requirements: &ComponentRequirements,
) -> Result<(String, String), ComponentError> {
    if descriptor.struct_size < core::mem::size_of::<AlefComponentV1>() {
        return Err(ComponentError::InvalidDescriptor("descriptor is smaller than ABI v1"));
    }
    if descriptor.abi_major != ABI_MAJOR_V1 || descriptor.abi_minor > ABI_MINOR_V1 {
        return Err(ComponentError::IncompatibleAbi {
            expected_major: ABI_MAJOR_V1,
            expected_minor: ABI_MINOR_V1,
            actual_major: descriptor.abi_major,
            actual_minor: descriptor.abi_minor,
        });
    }
    // SAFETY: both strings satisfy `copy_abi_str`'s contract per this function's own.
    let component_id = unsafe { copy_abi_str(descriptor.component_id)? };
    // SAFETY: as above.
    let component_version = unsafe { copy_abi_str(descriptor.component_version)? };
    if component_id != requirements.component_id {
        return Err(ComponentError::IdentityMismatch {
            expected: requirements.component_id.clone(),
            actual: component_id,
        });
    }
    if descriptor.contract_hash != requirements.contract_hash {
        return Err(ComponentError::ContractHashMismatch);
    }
    if requirements
        .feature_set_hash
        .is_some_and(|expected| descriptor.feature_set_hash != expected)
    {
        return Err(ComponentError::FeatureSetHashMismatch);
    }
    if descriptor.contract_size > 0 && descriptor.contract.is_null() {
        return Err(ComponentError::InvalidDescriptor(
            "non-empty contract has a null pointer",
        ));
    }
    Ok((component_id, component_version))
}

/// Copy a component-owned string out as a `String`, refusing more than
/// [`MAX_DESCRIPTOR_STRING`] bytes, a null pointer with a non-zero length, or invalid UTF-8.
///
/// # Safety
///
/// When `value.len` is non-zero and at most [`MAX_DESCRIPTOR_STRING`] and `value.ptr` is
/// non-null, `value.ptr` must be valid for reads of `value.len` bytes. Longer or null values are
/// rejected before the pointer is read.
unsafe fn copy_abi_str(value: alef_component_abi::AlefStr) -> Result<String, ComponentError> {
    if value.len > MAX_DESCRIPTOR_STRING {
        return Err(ComponentError::StringTooLong);
    }
    if value.len == 0 {
        return Ok(String::new());
    }
    if value.ptr.is_null() {
        return Err(ComponentError::InvalidDescriptor("non-empty string has a null pointer"));
    }
    // SAFETY: non-null, bounded and valid for `len` bytes per this function's contract.
    let bytes = unsafe { core::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) };
    let text = core::str::from_utf8(bytes).map_err(|_| ComponentError::InvalidUtf8)?;
    Ok(text.to_owned())
}

pub(crate) fn decode_hash(value: &str) -> Result<[u8; 32], ComponentError> {
    let bytes = hex::decode(value).map_err(|_| ComponentError::InvalidDigest(value.to_owned()))?;
    bytes
        .try_into()
        .map_err(|_| ComponentError::InvalidDigest(value.to_owned()))
}

#[cfg(test)]
mod tests;
