//! Safe producer-side view of the host API handed to a component's `create` function.

use crate::{AlefHostApiV1, AlefOwnedBuffer, AlefSlice, AlefStatus, AlefStr};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

/// Severity passed to the host log callback as its `level` argument.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

/// Why a host service call or notification did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostServiceError {
    /// The host is older than the service table (its `struct_size` stops before the service
    /// fields), supplied no callback, or the `HostApi` is detached.
    Unsupported,
    /// The host reported a non-`OK` status. `message` is the UTF-8 text the host stored in the
    /// out buffer, empty when it stored none.
    Failed { status: AlefStatus, message: String },
}

impl fmt::Display for HostServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("host does not provide a service table"),
            Self::Failed { status, message } if message.is_empty() => {
                write!(f, "host service failed with status {}", status.0)
            }
            Self::Failed { status, message } => write!(f, "host service failed with status {}: {message}", status.0),
        }
    }
}

impl core::error::Error for HostServiceError {}

/// Copy a host-allocated buffer into a `Vec` and release it through its own `free` callback.
///
/// # Safety
///
/// `buffer` must be a valid `AlefOwnedBuffer`: `ptr` readable for `len` bytes when `len > 0`
/// and `free` (if any) callable with the buffer's own fields.
unsafe fn take_buffer(buffer: AlefOwnedBuffer) -> Vec<u8> {
    let bytes = if buffer.ptr.is_null() || buffer.len == 0 {
        Vec::new()
    } else {
        // SAFETY: upheld by the caller contract.
        unsafe { core::slice::from_raw_parts(buffer.ptr, buffer.len) }.to_vec()
    };
    if let Some(free) = buffer.free {
        // SAFETY: upheld by the caller contract.
        unsafe { free(buffer.context, buffer.ptr, buffer.len, buffer.capacity) };
    }
    bytes
}

/// A copy of the host API a component was created with.
///
/// `HostApi` is `Copy`, so a [`FromHost`] implementation may store it to call the host later.
/// The host guarantees the contained contexts stay valid until the component instance is
/// destroyed; do not use a stored copy after that.
///
/// Threading contract: the host's `log`, `service_call` and `service_notify` callbacks must be
/// callable concurrently from any thread for the instance lifetime. A detached `HostApi`
/// (null or too-small host struct) silently drops log calls and reports
/// [`HostServiceError::Unsupported`] for services.
#[derive(Clone, Copy, Debug)]
pub struct HostApi {
    raw: Option<AlefHostApiV1>,
}

// SAFETY: the ABI requires the host's `context` and log callback to be usable from any thread
// for the instance lifetime (documented on `HostApi`); the struct itself is plain data. ~keep
unsafe impl Send for HostApi {}
// SAFETY: see the `Send` impl above; `HostApi` has no interior mutability. ~keep
unsafe impl Sync for HostApi {}

impl HostApi {
    /// A host API with no callbacks; every call is a no-op.
    #[must_use]
    pub const fn detached() -> Self {
        Self { raw: None }
    }

    /// Copy the host API behind `host`. Null, or a struct smaller than the original v1 layout
    /// ([`AlefHostApiV1::BASE_SIZE`]), yields a detached API. A host that predates the service
    /// table is accepted; its service calls report [`HostServiceError::Unsupported`].
    ///
    /// # Safety
    ///
    /// `host` must be null or point to memory readable for at least its own
    /// `struct_size` bytes (as produced by an ABI-conforming host).
    #[must_use]
    pub unsafe fn from_raw(host: *const AlefHostApiV1) -> Self {
        if host.is_null() {
            return Self::detached();
        }
        // SAFETY: `struct_size` is the first field (offset 0) and the caller guarantees `host`
        // is readable; read it unaligned and before forming a full reference. ~keep
        let struct_size = unsafe { host.cast::<usize>().read_unaligned() };
        if struct_size < AlefHostApiV1::BASE_SIZE {
            return Self::detached();
        }
        // An older host stops before the appended fields: copy only what it declared and leave
        // the rest zeroed (null pointers / `None`), which reads as "service table absent".
        let copy_len = struct_size.min(core::mem::size_of::<AlefHostApiV1>());
        let mut raw = MaybeUninit::<AlefHostApiV1>::zeroed();
        // SAFETY: `copy_len` bytes are readable at `host` per the caller contract and fit in
        // `raw`; an all-zero `AlefHostApiV1` is valid (integers, null pointers, `None`). ~keep
        let mut raw = unsafe {
            core::ptr::copy_nonoverlapping(host.cast::<u8>(), raw.as_mut_ptr().cast::<u8>(), copy_len);
            raw.assume_init()
        };
        raw.struct_size = copy_len;
        Self { raw: Some(raw) }
    }

    /// Whether the host supplied a request/response service callback.
    #[must_use]
    pub fn has_services(&self) -> bool {
        self.raw
            .is_some_and(|raw| raw.struct_size >= core::mem::size_of::<AlefHostApiV1>() && raw.service_call.is_some())
    }

    /// Call a host service with a bytes-only `request` and return its response bytes.
    ///
    /// The host owns the semantics of `service` and the payload encoding. The host-allocated
    /// response is copied out and freed through its own `free` callback before this returns.
    pub fn call_service(&self, service: &str, request: &[u8]) -> Result<Vec<u8>, HostServiceError> {
        let raw = self.raw.ok_or(HostServiceError::Unsupported)?;
        if raw.struct_size < core::mem::size_of::<AlefHostApiV1>() {
            return Err(HostServiceError::Unsupported);
        }
        let call = raw.service_call.ok_or(HostServiceError::Unsupported)?;
        let mut out = AlefOwnedBuffer::EMPTY;
        // SAFETY: the host promised `call` accepts its own `services_context`; `service`,
        // `request` and `out` outlive the call. ~keep
        let status = unsafe { call(raw.services_context, alef_str(service), alef_slice(request), &mut out) };
        // SAFETY: on every status the host either left `out` empty or filled it with a buffer
        // that follows the `AlefOwnedBuffer` ownership rules. ~keep
        let bytes = unsafe { take_buffer(out) };
        if status.is_ok() {
            Ok(bytes)
        } else {
            Err(HostServiceError::Failed {
                status,
                message: String::from_utf8_lossy(&bytes).into_owned(),
            })
        }
    }

    /// Send a one-way `payload` to a host service (progress, streaming chunks).
    pub fn notify(&self, service: &str, payload: &[u8]) -> Result<(), HostServiceError> {
        let raw = self.raw.ok_or(HostServiceError::Unsupported)?;
        if raw.struct_size < core::mem::size_of::<AlefHostApiV1>() {
            return Err(HostServiceError::Unsupported);
        }
        let notify = raw.service_notify.ok_or(HostServiceError::Unsupported)?;
        // SAFETY: the host promised `notify` accepts its own `services_context`; the arguments
        // outlive the call. ~keep
        let status = unsafe { notify(raw.services_context, alef_str(service), alef_slice(payload)) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(HostServiceError::Failed {
                status,
                message: String::new(),
            })
        }
    }

    /// Whether the host supplied a log callback.
    #[must_use]
    pub fn has_log(&self) -> bool {
        self.raw.is_some_and(|raw| raw.log.is_some())
    }

    /// Forward `message` to the host log callback as `"<target>: <message>"`
    /// (just `message` when `target` is empty). No-op without a callback.
    pub fn log(&self, level: LogLevel, target: &str, message: &str) {
        let Some(raw) = self.raw else { return };
        let Some(callback) = raw.log else { return };
        let text = if target.is_empty() {
            String::from(message)
        } else {
            format!("{target}: {message}")
        };
        let text: &str = &text;
        let message = AlefStr {
            ptr: text.as_ptr().cast(),
            len: text.len(),
        };
        // SAFETY: the host promised `callback` accepts its own `context`; `message` outlives the call. ~keep
        unsafe { callback(raw.context, level as u32, message) };
    }
}

fn alef_str(text: &str) -> AlefStr {
    AlefStr {
        ptr: text.as_ptr().cast(),
        len: text.len(),
    }
}

fn alef_slice(bytes: &[u8]) -> AlefSlice {
    AlefSlice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}

/// Constructor a component implementation may provide to receive the host API.
///
/// Generated producers call `from_host` when the implementation type implements
/// this trait, and fall back to `Default` otherwise (see [`Construct`]).
pub trait FromHost {
    fn from_host(host: &HostApi) -> Self;
}

/// Autoref-specialization probe used by generated producers: a call written as
/// `(&Construct::<T>::new()).construct(&host)` picks [`FromHost`] when `T`
/// implements it, else [`Default`]. Both traits must be in scope.
pub struct Construct<T>(PhantomData<T>);

impl<T> Construct<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for Construct<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Preferred arm: matches when `T: FromHost`.
pub trait ConstructFromHost<T> {
    fn construct(&self, host: &HostApi) -> T;
}

impl<T: FromHost> ConstructFromHost<T> for Construct<T> {
    fn construct(&self, host: &HostApi) -> T {
        T::from_host(host)
    }
}

/// Fallback arm: matches when `T: Default` (one autoref level further out).
pub trait ConstructDefault<T> {
    fn construct(&self, host: &HostApi) -> T;
}

impl<T: Default> ConstructDefault<T> for &Construct<T> {
    fn construct(&self, _host: &HostApi) -> T {
        T::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;

    unsafe extern "C" fn record(context: *mut c_void, level: u32, message: AlefStr) {
        // SAFETY: tests pass a `Vec<(u32, String)>` as context and a valid UTF-8 message.
        let sink = unsafe { &mut *context.cast::<Vec<(u32, String)>>() };
        let bytes = unsafe { core::slice::from_raw_parts(message.ptr.cast::<u8>(), message.len) };
        sink.push((level, String::from(core::str::from_utf8(bytes).unwrap())));
    }

    fn host_with(sink: &mut Vec<(u32, String)>) -> AlefHostApiV1 {
        AlefHostApiV1 {
            context: core::ptr::from_mut(sink).cast(),
            log: Some(record),
            ..AlefHostApiV1::EMPTY
        }
    }

    #[test]
    fn log_forwards_level_and_prefixed_message_to_the_callback() {
        let mut sink = Vec::new();
        let raw = host_with(&mut sink);
        let host = unsafe { HostApi::from_raw(&raw) };
        assert!(host.has_log());
        host.log(LogLevel::Warn, "engine", "slow");
        host.log(LogLevel::Info, "", "bare");
        assert_eq!(sink, [(2, String::from("engine: slow")), (3, String::from("bare"))]);
    }

    #[test]
    fn null_undersized_or_callbackless_hosts_drop_logs() {
        let detached = unsafe { HostApi::from_raw(core::ptr::null()) };
        assert!(!detached.has_log());
        detached.log(LogLevel::Error, "t", "m");

        let mut sink = Vec::new();
        let mut raw = host_with(&mut sink);
        raw.struct_size = 0;
        assert!(!unsafe { HostApi::from_raw(&raw) }.has_log());

        raw.struct_size = core::mem::size_of::<AlefHostApiV1>();
        raw.log = None;
        let host = unsafe { HostApi::from_raw(&raw) };
        host.log(LogLevel::Error, "t", "m");
        assert!(sink.is_empty());
    }

    unsafe extern "C" fn free_vec(_context: *mut c_void, ptr: *mut u8, len: usize, capacity: usize) {
        // SAFETY: tests only hand out buffers built from `Vec<u8>` parts.
        drop(unsafe { Vec::from_raw_parts(ptr, len, capacity) });
    }

    fn buffer_of(bytes: &[u8]) -> AlefOwnedBuffer {
        let mut vec = core::mem::ManuallyDrop::new(bytes.to_vec());
        AlefOwnedBuffer {
            ptr: vec.as_mut_ptr(),
            len: vec.len(),
            capacity: vec.capacity(),
            context: core::ptr::null_mut(),
            free: Some(free_vec),
        }
    }

    unsafe fn text(value: AlefStr) -> &'static str {
        core::str::from_utf8(unsafe { core::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }).unwrap()
    }

    /// Echoes `request` back reversed for "reverse"; fails with a message for "boom"; fails
    /// without a message for anything else.
    unsafe extern "C" fn fake_call(
        _context: *mut c_void,
        service: AlefStr,
        request: AlefSlice,
        out: *mut AlefOwnedBuffer,
    ) -> AlefStatus {
        let request = unsafe { core::slice::from_raw_parts(request.ptr, request.len) };
        match unsafe { text(service) } {
            "reverse" => {
                let mut bytes = request.to_vec();
                bytes.reverse();
                unsafe { *out = buffer_of(&bytes) };
                AlefStatus::OK
            }
            "boom" => {
                unsafe { *out = buffer_of(b"exploded") };
                AlefStatus::INTERNAL_ERROR
            }
            _ => AlefStatus::INVALID_ARGUMENT,
        }
    }

    unsafe extern "C" fn fake_notify(context: *mut c_void, service: AlefStr, payload: AlefSlice) -> AlefStatus {
        let sink = unsafe { &mut *context.cast::<Vec<(String, Vec<u8>)>>() };
        let payload = unsafe { core::slice::from_raw_parts(payload.ptr, payload.len) };
        sink.push((String::from(unsafe { text(service) }), payload.to_vec()));
        AlefStatus::OK
    }

    fn service_host(notes: &mut Vec<(String, Vec<u8>)>) -> AlefHostApiV1 {
        AlefHostApiV1 {
            services_context: core::ptr::from_mut(notes).cast(),
            service_call: Some(fake_call),
            service_notify: Some(fake_notify),
            ..AlefHostApiV1::EMPTY
        }
    }

    #[test]
    fn call_service_returns_the_host_response_and_frees_the_buffer() {
        let mut notes = Vec::new();
        let raw = service_host(&mut notes);
        let host = unsafe { HostApi::from_raw(&raw) };
        assert!(host.has_services());
        assert_eq!(host.call_service("reverse", b"abc").unwrap(), b"cba");
        assert_eq!(host.call_service("reverse", b"").unwrap(), b"");
    }

    #[test]
    fn call_service_maps_error_status_and_message() {
        let mut notes = Vec::new();
        let raw = service_host(&mut notes);
        let host = unsafe { HostApi::from_raw(&raw) };
        assert_eq!(
            host.call_service("boom", b"").unwrap_err(),
            HostServiceError::Failed {
                status: AlefStatus::INTERNAL_ERROR,
                message: String::from("exploded"),
            }
        );
        assert_eq!(
            host.call_service("unknown", b"").unwrap_err(),
            HostServiceError::Failed {
                status: AlefStatus::INVALID_ARGUMENT,
                message: String::new(),
            }
        );
    }

    #[test]
    fn notify_delivers_the_payload_and_reports_nothing_on_success() {
        let mut notes = Vec::new();
        let raw = service_host(&mut notes);
        let host = unsafe { HostApi::from_raw(&raw) };
        host.notify("progress", b"50%").unwrap();
        assert_eq!(notes, [(String::from("progress"), b"50%".to_vec())]);
    }

    #[test]
    fn services_are_unsupported_for_old_null_detached_or_callbackless_hosts() {
        let mut notes = Vec::new();
        let mut raw = service_host(&mut notes);

        raw.struct_size = AlefHostApiV1::BASE_SIZE;
        let old = unsafe { HostApi::from_raw(&raw) };
        assert!(!old.has_services());
        assert_eq!(old.call_service("reverse", b"x"), Err(HostServiceError::Unsupported));
        assert_eq!(old.notify("progress", b"x"), Err(HostServiceError::Unsupported));

        let null = unsafe { HostApi::from_raw(core::ptr::null()) };
        assert_eq!(null.call_service("reverse", b"x"), Err(HostServiceError::Unsupported));
        assert_eq!(
            HostApi::detached().notify("progress", b"x"),
            Err(HostServiceError::Unsupported)
        );

        let mut raw = service_host(&mut notes);
        raw.service_call = None;
        raw.service_notify = None;
        let none = unsafe { HostApi::from_raw(&raw) };
        assert!(!none.has_services());
        assert_eq!(none.call_service("reverse", b"x"), Err(HostServiceError::Unsupported));
        assert_eq!(none.notify("progress", b"x"), Err(HostServiceError::Unsupported));
    }

    #[test]
    fn an_old_host_keeps_working_logging_without_reading_past_its_struct_size() {
        let mut sink = Vec::new();
        let mut raw = host_with(&mut sink);
        // Poison the appended fields: an old host never wrote them, so they must be ignored.
        raw.service_call = Some(fake_call);
        raw.struct_size = AlefHostApiV1::BASE_SIZE;
        let host = unsafe { HostApi::from_raw(&raw) };
        assert!(host.has_log());
        assert!(!host.has_services());
        host.log(LogLevel::Info, "", "still logs");
        assert_eq!(sink, [(3, String::from("still logs"))]);
    }

    #[derive(Default)]
    struct OnlyDefault;

    struct Configured(&'static str);

    impl FromHost for Configured {
        fn from_host(host: &HostApi) -> Self {
            Self(if host.has_log() { "with-log" } else { "no-log" })
        }
    }

    #[test]
    // The explicit `&` is the autoref-specialization dispatch the generated producer relies on.
    #[allow(clippy::needless_borrow)]
    fn construct_prefers_from_host_and_falls_back_to_default() {
        let host = HostApi::detached();
        let configured = (&Construct::<Configured>::new()).construct(&host);
        assert_eq!(configured.0, "no-log");
        let _default: OnlyDefault = (&Construct::<OnlyDefault>::new()).construct(&host);
    }
}
