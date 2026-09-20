use crate::{
    ArtifactCache, COMPONENT_ABI_VERSION, COMPONENT_MANIFEST_SCHEMA, ComponentError, ComponentLockEntry,
    ComponentManifest,
};
use alef_component_abi::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AlefComponentEntryV1, AlefComponentV1, AlefContract, AlefHostApiV1, AlefOwnedBuffer,
};
use libloading::Library;
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

#[derive(Clone, Debug)]
pub struct Runtime {
    cache: ArtifactCache,
}

impl Runtime {
    pub fn new(cache: ArtifactCache) -> Self {
        Self { cache }
    }

    pub fn install_and_load(
        &self,
        entry: &ComponentLockEntry,
        entry_symbol: &[u8],
        requirements: &ComponentRequirements,
        host: AlefHostApiV1,
    ) -> Result<LoadedComponent, ComponentError> {
        let cached = self.cache.install(entry)?;
        validate_manifest(&cached.manifest, requirements)?;
        LoadedComponent::load(cached.library, entry_symbol, requirements, host)
    }
}

pub struct LoadedComponent {
    library: Arc<Library>,
    descriptor: NonNull<AlefComponentV1>,
    host: Box<AlefHostApiV1>,
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

// SAFETY: the descriptor is static component metadata and the library is pinned
// for the process lifetime. Host callback context thread-safety is part of the
// `AlefHostApiV1` contract supplied by the caller. ~keep
unsafe impl Send for LoadedComponent {}
unsafe impl Sync for LoadedComponent {}

// SAFETY: the C ABI requires every contract method to take `&self` (see
// `component::map_method`) and calls it through a plain function pointer, so the
// component's own implementation is the only thing that can make concurrent access
// unsound; the ABI async wrapper already calls back into an instance from a spawned
// worker thread, so components are contractually required to tolerate this. The
// instance handle itself is owned exclusively by this `ComponentInstance` for its
// lifetime, with no interior aliasing on the host side. ~keep
unsafe impl<T: AlefContract> Send for ComponentInstance<T> {}
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
    pub fn load(
        path: impl Into<std::path::PathBuf>,
        entry_symbol: &[u8],
        requirements: &ComponentRequirements,
        mut host: AlefHostApiV1,
    ) -> Result<Self, ComponentError> {
        let path = path.into();
        host.struct_size = core::mem::size_of::<AlefHostApiV1>();
        host.abi_major = ABI_MAJOR_V1;
        host.abi_minor = ABI_MINOR_V1;
        let host = Box::new(host);

        let library = Arc::new(
            unsafe { Library::new(&path) }.map_err(|error| ComponentError::LibraryLoad {
                path: path.clone(),
                message: error.to_string(),
            })?,
        );
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
        unsafe { reject_undersized_descriptor(descriptor)? };
        let validated = unsafe { validate_descriptor(descriptor.as_ref(), requirements)? };
        pin_for_process(Arc::clone(&library));
        Ok(Self {
            library,
            descriptor,
            host,
            component_id: validated.0,
            component_version: validated.1,
        })
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
        let status = unsafe { create(self.host_api(), &mut raw, &mut error) };
        if !status.is_ok() {
            return Err(ComponentError::InstanceCreation {
                status: status.0,
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
        Arc::strong_count(&self.library)
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

unsafe fn take_owned_buffer(buffer: AlefOwnedBuffer) -> String {
    let message = if buffer.ptr.is_null() {
        if buffer.len == 0 {
            String::new()
        } else {
            "component returned an invalid error buffer".to_string()
        }
    } else {
        String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(buffer.ptr, buffer.len) }).into_owned()
    };
    if let Some(free) = buffer.free {
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
    Ok(unsafe { &*descriptor.contract.cast::<T>() })
}

fn process_pins() -> &'static Mutex<Vec<Arc<Library>>> {
    static PINS: OnceLock<Mutex<Vec<Arc<Library>>>> = OnceLock::new();
    PINS.get_or_init(|| Mutex::new(Vec::new()))
}

fn pin_for_process(library: Arc<Library>) {
    process_pins()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(library);
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
    let component_id = unsafe { copy_abi_str(descriptor.component_id)? };
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
mod tests {
    use super::*;
    use alef_component_abi::{AlefComponentV1, AlefStr};
    use std::ffi::c_char;

    static ID: &[u8] = b"demo";
    static VERSION: &[u8] = b"1.2.3";

    #[repr(C)]
    struct DemoContract {
        answer: u32,
    }

    unsafe impl AlefContract for DemoContract {
        const CONTRACT_HASH: [u8; 32] = [7; 32];
    }

    static CONTRACT: DemoContract = DemoContract { answer: 42 };

    fn descriptor(contract_hash: [u8; 32]) -> AlefComponentV1 {
        AlefComponentV1 {
            struct_size: core::mem::size_of::<AlefComponentV1>(),
            abi_major: ABI_MAJOR_V1,
            abi_minor: ABI_MINOR_V1,
            component_id: AlefStr {
                ptr: ID.as_ptr().cast::<c_char>(),
                len: ID.len(),
            },
            component_version: AlefStr {
                ptr: VERSION.as_ptr().cast::<c_char>(),
                len: VERSION.len(),
            },
            contract_hash,
            feature_set_hash: [8; 32],
            contract: core::ptr::dangling(),
            contract_size: 1,
            create: None,
            destroy: None,
        }
    }

    fn requirements() -> ComponentRequirements {
        ComponentRequirements {
            component_id: "demo".into(),
            contract_name: "engine".into(),
            contract_hash: [7; 32],
            feature_set_hash: Some([8; 32]),
        }
    }

    #[test]
    fn accepts_matching_descriptor() {
        let descriptor = descriptor([7; 32]);
        let validated = unsafe { validate_descriptor(&descriptor, &requirements()) }.unwrap();
        assert_eq!(validated, ("demo".into(), "1.2.3".into()));
    }

    #[test]
    fn rejects_wrong_contract_hash() {
        let descriptor = descriptor([9; 32]);
        assert!(matches!(
            unsafe { validate_descriptor(&descriptor, &requirements()) },
            Err(ComponentError::ContractHashMismatch)
        ));
    }

    #[test]
    fn rejects_short_descriptor_before_optional_fields_are_read() {
        let mut descriptor = descriptor([7; 32]);
        descriptor.struct_size = 8;
        assert!(matches!(
            unsafe { validate_descriptor(&descriptor, &requirements()) },
            Err(ComponentError::InvalidDescriptor(_))
        ));
    }

    /// Regression for forming a `&AlefComponentV1` over memory that only actually holds a
    /// `struct_size` field. The backing allocation below is deliberately sized for nothing
    /// more than that one `usize`, so `reject_undersized_descriptor`'s raw pointer read must
    /// be the only access that happens -- ever forming a full `&AlefComponentV1` reference
    /// over it would read past the allocation.
    #[test]
    fn rejects_a_descriptor_backed_by_memory_too_small_to_hold_the_full_struct() {
        let layout = std::alloc::Layout::from_size_align(
            core::mem::size_of::<usize>(),
            core::mem::align_of::<AlefComponentV1>(),
        )
        .unwrap();
        // SAFETY: `layout` has non-zero size.
        let raw = unsafe { std::alloc::alloc(layout) };
        assert!(!raw.is_null(), "test allocation failed");
        // SAFETY: `raw` is valid for `size_of::<usize>()` bytes per `layout` and is
        // sufficiently aligned for a `usize` write.
        unsafe { raw.cast::<usize>().write_unaligned(8) };
        let descriptor = NonNull::new(raw).unwrap().cast::<AlefComponentV1>();

        // SAFETY: `descriptor` points to `size_of::<usize>()` valid bytes, matching this
        // function's caller contract; no wider reference is formed here.
        let result = unsafe { reject_undersized_descriptor(descriptor) };

        // SAFETY: `raw`/`layout` match the earlier allocation exactly.
        unsafe { std::alloc::dealloc(raw, layout) };
        assert!(matches!(result, Err(ComponentError::InvalidDescriptor(_))));
    }

    #[test]
    fn manifest_validation_checks_feature_identity() {
        let manifest = ComponentManifest {
            schema_version: COMPONENT_MANIFEST_SCHEMA,
            abi_version: 1,
            identity: crate::ComponentIdentity {
                crate_name: "demo-core".into(),
                component: "demo".into(),
                version: "1.2.3".into(),
                target: "test-target".into(),
                feature_hash: hex::encode([9; 32]),
                contract_hash: hex::encode([7; 32]),
            },
            provides: vec![crate::ComponentProvidedContract {
                contract: "engine".into(),
                interface_version: 1,
                contract_hash: hex::encode([7; 32]),
                implementation: "demo::Engine".into(),
            }],
            features: Vec::new(),
            default_features: false,
            library: crate::ComponentLibrary {
                file: "libdemo.so".into(),
                sha256: hex::encode([0; 32]),
                size: 0,
            },
        };
        assert!(matches!(
            validate_manifest(&manifest, &requirements()),
            Err(ComponentError::FeatureSetHashMismatch)
        ));
    }

    #[test]
    fn typed_contract_table_checks_hash_size_and_alignment() {
        let mut descriptor = descriptor(DemoContract::CONTRACT_HASH);
        descriptor.contract = (&raw const CONTRACT).cast();
        descriptor.contract_size = core::mem::size_of::<DemoContract>();
        assert_eq!(
            contract_table_from_descriptor::<DemoContract>(&descriptor)
                .unwrap()
                .answer,
            42
        );

        descriptor.contract_hash = [9; 32];
        assert!(matches!(
            contract_table_from_descriptor::<DemoContract>(&descriptor),
            Err(ComponentError::InvalidContractTable)
        ));
    }
}
