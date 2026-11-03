//! Host side of the component-to-host service table (`AlefHostApiV1::service_call` /
//! `service_notify`): a [`HostServices`] trait hosts implement, and the `extern "C"`
//! trampolines that route a component's calls to it.

use alef_component_abi::{AlefHostApiV1, AlefOwnedBuffer, AlefSlice, AlefStatus, AlefStr};
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Services a host exposes to the components it loads.
///
/// The host owns the meaning of `service` names and payload bytes; the ABI only moves bytes.
/// Components may call from any thread, so implementations must be thread-safe. A panic in an
/// implementation is caught at the FFI boundary and reported to the component as an error
/// status, never unwound across it.
pub trait HostServices: Send + Sync {
    /// Handle a request/response call. An `Err` message is delivered to the component with an
    /// `INTERNAL_ERROR` status.
    fn call(&self, service: &str, request: &[u8]) -> Result<Vec<u8>, String>;

    /// Handle a one-way notification (progress, streaming chunks).
    fn notify(&self, service: &str, payload: &[u8]);
}

/// Keeps the `Arc` the table's `services_context` points at alive and at a stable address for
/// as long as the loaded component exists.
pub(crate) struct ServicesHandle {
    services: Box<Arc<dyn HostServices>>,
}

impl ServicesHandle {
    pub(crate) fn new(services: Arc<dyn HostServices>) -> Self {
        Self {
            services: Box::new(services),
        }
    }

    /// Fill the service fields of `host` with this handle's trampolines.
    pub(crate) fn install(&self, host: &mut AlefHostApiV1) {
        host.services_context = core::ptr::from_ref(&*self.services).cast_mut().cast();
        host.service_call = Some(service_call);
        host.service_notify = Some(service_notify);
    }
}

unsafe extern "C" fn free_vec(_context: *mut c_void, ptr: *mut u8, len: usize, capacity: usize) {
    // SAFETY: only `owned_buffer` hands out buffers with this `free`, built from `Vec<u8>` parts.
    drop(unsafe { Vec::from_raw_parts(ptr, len, capacity) });
}

fn owned_buffer(bytes: Vec<u8>) -> AlefOwnedBuffer {
    let mut bytes = ManuallyDrop::new(bytes);
    AlefOwnedBuffer {
        ptr: bytes.as_mut_ptr(),
        len: bytes.len(),
        capacity: bytes.capacity(),
        context: core::ptr::null_mut(),
        free: Some(free_vec),
    }
}

/// # Safety
///
/// `value.ptr` must be null or readable for `value.len` bytes.
unsafe fn str_arg<'a>(value: AlefStr) -> Option<&'a str> {
    if value.len == 0 {
        return Some("");
    }
    if value.ptr.is_null() {
        return None;
    }
    // SAFETY: upheld by the caller contract.
    std::str::from_utf8(unsafe { core::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }).ok()
}

/// # Safety
///
/// `value.ptr` must be null or readable for `value.len` bytes.
unsafe fn slice_arg<'a>(value: AlefSlice) -> Option<&'a [u8]> {
    if value.len == 0 {
        return Some(&[]);
    }
    if value.ptr.is_null() {
        return None;
    }
    // SAFETY: upheld by the caller contract.
    Some(unsafe { core::slice::from_raw_parts(value.ptr, value.len) })
}

unsafe extern "C" fn service_call(
    context: *mut c_void,
    service: AlefStr,
    request: AlefSlice,
    out: *mut AlefOwnedBuffer,
) -> AlefStatus {
    if context.is_null() || out.is_null() {
        return AlefStatus::INVALID_ARGUMENT;
    }
    // SAFETY: `context` is the `Arc<dyn HostServices>` that `ServicesHandle::install` published,
    // kept alive by the loaded component; the component's arguments are valid per the ABI.
    let (services, service, request) = unsafe {
        (
            &*context.cast::<Arc<dyn HostServices>>(),
            str_arg(service),
            slice_arg(request),
        )
    };
    let (Some(service), Some(request)) = (service, request) else {
        return AlefStatus::INVALID_ARGUMENT;
    };
    let (status, bytes) = match catch_unwind(AssertUnwindSafe(|| services.call(service, request))) {
        Ok(Ok(bytes)) => (AlefStatus::OK, bytes),
        Ok(Err(message)) => (AlefStatus::INTERNAL_ERROR, message.into_bytes()),
        Err(_) => (AlefStatus::INTERNAL_ERROR, b"host service panicked".to_vec()),
    };
    // SAFETY: `out` is non-null and writable per the ABI; it holds no live value to drop.
    unsafe { out.write(owned_buffer(bytes)) };
    status
}

unsafe extern "C" fn service_notify(context: *mut c_void, service: AlefStr, payload: AlefSlice) -> AlefStatus {
    if context.is_null() {
        return AlefStatus::INVALID_ARGUMENT;
    }
    // SAFETY: see `service_call`.
    let (services, service, payload) = unsafe {
        (
            &*context.cast::<Arc<dyn HostServices>>(),
            str_arg(service),
            slice_arg(payload),
        )
    };
    let (Some(service), Some(payload)) = (service, payload) else {
        return AlefStatus::INVALID_ARGUMENT;
    };
    match catch_unwind(AssertUnwindSafe(|| services.notify(service, payload))) {
        Ok(()) => AlefStatus::OK,
        Err(_) => AlefStatus::INTERNAL_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_component_abi::HostApi;
    use alef_component_abi::HostServiceError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        notes: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl HostServices for Recorder {
        fn call(&self, service: &str, request: &[u8]) -> Result<Vec<u8>, String> {
            match service {
                "echo" => Ok(request.to_vec()),
                "fail" => Err("nope".to_owned()),
                _ => panic!("unexpected service {service}"),
            }
        }

        fn notify(&self, service: &str, payload: &[u8]) {
            if service == "panic" {
                panic!("notify panic");
            }
            self.notes.lock().unwrap().push((service.to_owned(), payload.to_vec()));
        }
    }

    fn host_for(recorder: &Arc<Recorder>) -> (ServicesHandle, AlefHostApiV1) {
        let handle = ServicesHandle::new(Arc::clone(recorder) as Arc<dyn HostServices>);
        let mut raw = AlefHostApiV1::EMPTY;
        handle.install(&mut raw);
        (handle, raw)
    }

    #[test]
    fn trampolines_route_calls_and_notifications_to_the_registered_services() {
        let recorder = Arc::new(Recorder::default());
        let (_handle, raw) = host_for(&recorder);
        // SAFETY: `raw` is a complete, live host table.
        let api = unsafe { HostApi::from_raw(&raw) };
        assert_eq!(api.call_service("echo", b"hello").unwrap(), b"hello");
        assert_eq!(api.call_service("echo", b"").unwrap(), b"");
        api.notify("progress", b"1/2").unwrap();
        assert_eq!(
            *recorder.notes.lock().unwrap(),
            [("progress".to_owned(), b"1/2".to_vec())]
        );
    }

    #[test]
    fn host_errors_reach_the_component_as_internal_error_with_the_message() {
        let recorder = Arc::new(Recorder::default());
        let (_handle, raw) = host_for(&recorder);
        // SAFETY: as above.
        let api = unsafe { HostApi::from_raw(&raw) };
        assert_eq!(
            api.call_service("fail", b"").unwrap_err(),
            HostServiceError::Failed {
                status: AlefStatus::INTERNAL_ERROR,
                message: "nope".to_owned(),
            }
        );
    }

    #[test]
    fn panics_in_host_callbacks_become_error_statuses() {
        let recorder = Arc::new(Recorder::default());
        let (_handle, raw) = host_for(&recorder);
        // SAFETY: as above.
        let api = unsafe { HostApi::from_raw(&raw) };
        assert_eq!(
            api.call_service("boom", b"").unwrap_err(),
            HostServiceError::Failed {
                status: AlefStatus::INTERNAL_ERROR,
                message: "host service panicked".to_owned(),
            }
        );
        assert!(matches!(
            api.notify("panic", b""),
            Err(HostServiceError::Failed {
                status: AlefStatus::INTERNAL_ERROR,
                ..
            })
        ));
    }

    #[test]
    fn invalid_utf8_service_names_are_rejected() {
        let recorder = Arc::new(Recorder::default());
        let (_handle, raw) = host_for(&recorder);
        let bad = [0xff_u8, 0xfe];
        let name = AlefStr {
            ptr: bad.as_ptr().cast(),
            len: bad.len(),
        };
        let mut out = AlefOwnedBuffer::EMPTY;
        let call = raw.service_call.unwrap();
        // SAFETY: the table is live and the arguments are valid for the call.
        let status = unsafe { call(raw.services_context, name, AlefSlice::EMPTY, &mut out) };
        assert_eq!(status, AlefStatus::INVALID_ARGUMENT);
        assert!(out.ptr.is_null());
    }
}
