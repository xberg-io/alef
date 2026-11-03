//! Safe producer-side view of the host API handed to a component's `create` function.

use crate::{AlefHostApiV1, AlefStr};
use alloc::format;
use alloc::string::String;
use core::marker::PhantomData;

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

/// A copy of the host API a component was created with.
///
/// The host must keep `context` valid, and its log callback callable from any
/// thread, for as long as the component instance lives. A detached `HostApi`
/// (null or too-small host struct) silently drops log calls.
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

    /// Copy the host API behind `host`. Null, or a struct smaller than
    /// [`AlefHostApiV1`], yields a detached API.
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
        if struct_size < core::mem::size_of::<AlefHostApiV1>() {
            return Self::detached();
        }
        // SAFETY: `struct_size` covers the whole v1 struct. ~keep
        Self {
            raw: Some(unsafe { host.read_unaligned() }),
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
    use alloc::vec::Vec;
    use core::ffi::c_void;

    unsafe extern "C" fn record(context: *mut c_void, level: u32, message: AlefStr) {
        // SAFETY: tests pass a `Vec<(u32, String)>` as context and a valid UTF-8 message.
        let sink = unsafe { &mut *context.cast::<Vec<(u32, String)>>() };
        let bytes = unsafe { core::slice::from_raw_parts(message.ptr.cast::<u8>(), message.len) };
        sink.push((level, String::from(core::str::from_utf8(bytes).unwrap())));
    }

    fn host_with(sink: &mut Vec<(u32, String)>) -> AlefHostApiV1 {
        AlefHostApiV1 {
            struct_size: core::mem::size_of::<AlefHostApiV1>(),
            abi_major: 1,
            abi_minor: 0,
            context: core::ptr::from_mut(sink).cast(),
            log: Some(record),
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
