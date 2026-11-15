//! Producer-side helpers every generated component crate calls instead of carrying its own copy.
//!
//! Generated producers hand owned buffers, task handles and static descriptor tables across the
//! C boundary. The code for that is identical in every producer, so it lives here once, where it
//! is documented and tested, rather than being pasted into each generated `src/lib.rs`.

use crate::{AlefOwnedBuffer, AlefStatus, AlefTaskV1};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::ffi::c_void;
use core::fmt;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicBool, Ordering};

/// A `static`-friendly wrapper for an immutable C table that carries raw pointers.
///
/// Component descriptors ([`AlefComponentV1`](crate::AlefComponentV1)) and generated contract
/// tables hold raw pointers, which makes them `!Sync`, so they cannot be plain `static`s. Before
/// this wrapper existed producers declared them `static mut` purely to get past that check. The
/// wrapper restores an ordinary immutable `static`: the value is only ever read, and the
/// constructor's safety contract is what makes sharing it between threads sound.
#[repr(transparent)]
pub struct SyncStatic<T>(T);

// SAFETY: `SyncStatic::new` requires the wrapped value to be immutable plain data whose raw
// pointers refer to immutable, process-lifetime data (or are null); `SyncStatic` exposes no
// mutation, so concurrent shared reads from any thread are sound.
unsafe impl<T> Sync for SyncStatic<T> {}

impl<T> SyncStatic<T> {
    /// Wrap `value` for use in a `static`.
    ///
    /// # Safety
    ///
    /// `value` must be safe to read concurrently from any thread for the life of the process: it
    /// has no interior mutability, and every raw pointer it holds is null or points to immutable
    /// data that lives for the rest of the process (string literals, other statics, functions).
    #[must_use]
    pub const unsafe fn new(value: T) -> Self {
        Self(value)
    }

    /// The wrapped value.
    #[must_use]
    pub const fn get(&self) -> &T {
        &self.0
    }

    /// A raw pointer to the wrapped value, valid for the life of a `static` wrapper.
    #[must_use]
    pub const fn as_ptr(&self) -> *const T {
        &raw const self.0
    }
}

/// The `free` callback of every buffer [`owned_buffer`] produces.
///
/// # Safety
///
/// `ptr`, `len` and `capacity` must be the parts of one buffer [`owned_buffer`] produced, and the
/// buffer must be released exactly once. A null `ptr` is ignored.
pub unsafe extern "C" fn free_owned_buffer(_context: *mut c_void, ptr: *mut u8, len: usize, capacity: usize) {
    if !ptr.is_null() {
        // SAFETY: the caller contract says these are the parts of a `Vec<u8>` `owned_buffer`
        // leaked, released once.
        drop(unsafe { Vec::from_raw_parts(ptr, len, capacity) });
    }
}

/// Hand `bytes` across the boundary as an [`AlefOwnedBuffer`] the receiver releases through the
/// buffer's own `free` callback ([`free_owned_buffer`]).
#[must_use]
pub fn owned_buffer(bytes: Vec<u8>) -> AlefOwnedBuffer {
    let mut bytes = ManuallyDrop::new(bytes);
    AlefOwnedBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
        capacity: bytes.capacity(),
        context: core::ptr::null_mut(),
        free: Some(free_owned_buffer),
    }
}

/// An owned buffer holding `message` as UTF-8, for a producer's `out_error`.
#[must_use]
pub fn error_buffer(message: impl fmt::Display) -> AlefOwnedBuffer {
    owned_buffer(message.to_string().into_bytes())
}

/// Copy an owned buffer's bytes into a `Vec<u8>` and release the original allocation through its
/// own `free` callback.
///
/// This is the receiving side of [`owned_buffer`]: hosts call it on a component's results, and
/// components on buffers the host returns from a service call.
///
/// # Safety
///
/// `buffer` must be a valid `AlefOwnedBuffer`: `ptr` must be non-null and valid for `len` bytes
/// whenever `len > 0`, and `free` (if present) must be safe to call with this buffer's own
/// `context`, `ptr`, `len`, and `capacity`. The buffer must not be used again afterwards.
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

/// Best-effort text of a caught panic payload (`&'static str` or `String`).
#[must_use]
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        String::from(*message)
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        String::from("non-string panic payload")
    }
}

/// An [`AlefTaskV1`] whose `cancel` sets `cancelled` and whose `drop` releases the task's own
/// reference to it.
///
/// A producer's async wrapper keeps a clone of `cancelled`, checks it before starting the work,
/// and owns nothing else about the handle: the host calls `cancel` and `drop` through the task.
#[must_use]
pub fn cancel_flag_task(cancelled: Arc<AtomicBool>) -> AlefTaskV1 {
    AlefTaskV1 {
        struct_size: core::mem::size_of::<AlefTaskV1>(),
        context: Box::into_raw(Box::new(cancelled)).cast(),
        start: None,
        cancel: Some(cancel_flag),
        drop: Some(drop_cancel_flag),
    }
}

/// # Safety
///
/// `context` must be null or the context of a live task [`cancel_flag_task`] built.
unsafe extern "C" fn cancel_flag(context: *mut c_void) -> AlefStatus {
    if context.is_null() {
        return AlefStatus::INVALID_ARGUMENT;
    }
    // SAFETY: a non-null context is the `Box<Arc<AtomicBool>>` `cancel_flag_task` leaked, and the
    // task has not been dropped yet.
    unsafe { &*context.cast::<Arc<AtomicBool>>() }.store(true, Ordering::Release);
    AlefStatus::OK
}

/// # Safety
///
/// `context` must be null or the context of a task [`cancel_flag_task`] built, dropped once.
unsafe extern "C" fn drop_cancel_flag(context: *mut c_void) {
    if !context.is_null() {
        // SAFETY: a non-null context is the `Box<Arc<AtomicBool>>` `cancel_flag_task` leaked, and
        // the host drops each task once.
        drop(unsafe { Box::from_raw(context.cast::<Arc<AtomicBool>>()) });
    }
}

/// Construct a component implementation, preferring [`FromHost`](crate::FromHost) when the type
/// implements it and falling back to [`Default`].
///
/// Expands to the autoref-specialization call over [`Construct`](crate::Construct), with both
/// candidate traits in scope, so generated producers need neither import nor lint allowance.
#[macro_export]
macro_rules! construct_component {
    ($implementation:ty, $host:expr) => {{
        #[allow(unused_imports, reason = "only one of the two traits applies to a given type")]
        use $crate::{ConstructDefault as _, ConstructFromHost as _};
        #[allow(clippy::needless_borrow, reason = "the borrow selects the `Default` fallback")]
        let value: $implementation = (&$crate::Construct::<$implementation>::new()).construct($host);
        value
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlefComponentV1, AlefStr, FromHost, HostApi};

    #[test]
    fn owned_buffer_round_trips_through_take_owned_bytes() {
        let buffer = owned_buffer(alloc::vec![1, 2, 3]);
        assert!(buffer.free.is_some());
        // SAFETY: `buffer` was just built by `owned_buffer` and is consumed once.
        assert_eq!(unsafe { take_owned_bytes(buffer) }, [1, 2, 3]);
        // SAFETY: an empty buffer with no `free` is valid and needs no release.
        assert!(unsafe { take_owned_bytes(AlefOwnedBuffer::EMPTY) }.is_empty());
    }

    #[test]
    fn error_buffer_holds_the_display_text() {
        let buffer = error_buffer(format_args!("bad {}", 7));
        // SAFETY: `buffer` was just built by `owned_buffer` and is consumed once.
        assert_eq!(unsafe { take_owned_bytes(buffer) }, b"bad 7");
    }

    #[test]
    fn free_owned_buffer_ignores_null() {
        // SAFETY: a null pointer is documented as ignored.
        unsafe { free_owned_buffer(core::ptr::null_mut(), core::ptr::null_mut(), 0, 0) };
    }

    #[test]
    fn panic_message_reads_str_and_string_payloads() {
        let text: Box<dyn Any + Send> = Box::new("static");
        assert_eq!(panic_message(&*text), "static");
        let owned: Box<dyn Any + Send> = Box::new(String::from("owned"));
        assert_eq!(panic_message(&*owned), "owned");
        let other: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(&*other), "non-string panic payload");
    }

    #[test]
    fn cancel_flag_task_sets_the_shared_flag_and_releases_its_reference() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let task = cancel_flag_task(Arc::clone(&cancelled));
        assert_eq!(Arc::strong_count(&cancelled), 2);
        let cancel = task.cancel.expect("cancel callback");
        let drop_task = task.drop.expect("drop callback");
        // SAFETY: `task` is live and its context came from `cancel_flag_task`.
        assert_eq!(unsafe { cancel(task.context) }, AlefStatus::OK);
        assert!(cancelled.load(Ordering::Acquire));
        // SAFETY: as above; a null context is rejected rather than dereferenced.
        assert_eq!(unsafe { cancel(core::ptr::null_mut()) }, AlefStatus::INVALID_ARGUMENT);
        // SAFETY: the task is dropped exactly once.
        unsafe { drop_task(task.context) };
        assert_eq!(Arc::strong_count(&cancelled), 1);
    }

    const ID: &[u8] = b"demo";

    // SAFETY: every pointer in the descriptor is null or points at the `ID` literal.
    static DESCRIPTOR: SyncStatic<AlefComponentV1> = unsafe {
        SyncStatic::new(AlefComponentV1 {
            struct_size: core::mem::size_of::<AlefComponentV1>(),
            abi_major: 1,
            abi_minor: 0,
            component_id: AlefStr {
                ptr: ID.as_ptr().cast(),
                len: ID.len(),
            },
            component_version: AlefStr::EMPTY,
            contract_hash: [0; 32],
            feature_set_hash: [0; 32],
            contract: core::ptr::null(),
            contract_size: 0,
            create: None,
            destroy: None,
        })
    };

    #[test]
    fn sync_static_is_a_transparent_plain_static() {
        assert_eq!(
            core::mem::size_of::<SyncStatic<AlefComponentV1>>(),
            core::mem::size_of::<AlefComponentV1>()
        );
        assert_eq!(DESCRIPTOR.as_ptr(), core::ptr::from_ref(DESCRIPTOR.get()));
        assert_eq!(DESCRIPTOR.get().component_id.len, 4);
    }

    #[derive(Default)]
    struct OnlyDefault(u8);

    struct Configured(bool);

    impl FromHost for Configured {
        fn from_host(host: &HostApi) -> Self {
            Self(host.has_log())
        }
    }

    #[test]
    fn construct_component_prefers_from_host_and_falls_back_to_default() {
        let host = HostApi::detached();
        let configured = construct_component!(Configured, &host);
        assert!(!configured.0);
        let fallback = construct_component!(OnlyDefault, &host);
        assert_eq!(fallback.0, 0);
    }
}
