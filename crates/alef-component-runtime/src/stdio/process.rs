use super::{STDIO_PROTOCOL_VERSION, StdioDescriptor, StdioRequest, StdioResponse, read_frame, write_frame};
use crate::loader::decode_hash;
use crate::{ComponentError, ComponentManifest};
use alef_component_abi::{ABI_MAJOR_V1, ABI_MINOR_V1};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::Duration;

// On a multi-threaded host, another thread can fork while the just-installed sidecar executable's write
// descriptor is still open, and the child holds that descriptor until `exec` — so the start fails with
// `ExecutableFileBusy` ("Text file busy", os error 26) on Linux. Retrying briefly is what cargo, rustup and
// poly do for the same race.
const SPAWN_BUSY_RETRIES: u32 = 200;
const SPAWN_BUSY_BACKOFF: Duration = Duration::from_millis(10);

struct Exchange {
    request: Vec<u8>,
    response: mpsc::Sender<Result<Vec<u8>, ComponentError>>,
}

pub(crate) struct SidecarProcess {
    child: Mutex<Child>,
    sender: Mutex<Option<mpsc::Sender<Exchange>>>,
    timeout: Duration,
    stopped: AtomicBool,
}

impl SidecarProcess {
    pub(crate) fn spawn(
        executable: &Path,
        manifest: &ComponentManifest,
        timeout: Duration,
    ) -> Result<Self, ComponentError> {
        make_executable(executable)?;
        let mut child = retry_executable_busy(SPAWN_BUSY_RETRIES, SPAWN_BUSY_BACKOFF, || {
            Command::new(executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
        })
        .map_err(|error| {
            ComponentError::SidecarProtocol(format!("failed to start {}: {error}", executable.display()))
        })?;
        let mut stdin = child.stdin.take().expect("piped child stdin missing");
        let mut stdout = child.stdout.take().expect("piped child stdout missing");
        let (sender, receiver) = mpsc::channel::<Exchange>();
        let process = Self {
            child: Mutex::new(child),
            sender: Mutex::new(Some(sender)),
            timeout,
            stopped: AtomicBool::new(false),
        };
        std::thread::Builder::new()
            .name("alef-sidecar-stdio".into())
            .spawn(move || {
                while let Ok(exchange) = receiver.recv() {
                    let response = write_frame(&mut stdin, &exchange.request).and_then(|()| read_frame(&mut stdout));
                    let failed = response.is_err();
                    if exchange.response.send(response).is_err() || failed {
                        break;
                    }
                }
            })?;
        let payload = process.exchange(&StdioRequest::Describe)?;
        let descriptor: StdioDescriptor = serde_json::from_slice(&payload)
            .map_err(|error| ComponentError::SidecarProtocol(format!("invalid describe payload: {error}")))?;
        validate_descriptor(&descriptor, manifest)?;
        Ok(process)
    }

    pub(crate) fn invoke(&self, contract: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, ComponentError> {
        if payload.len() > super::MAX_FRAME_SIZE {
            return Err(ComponentError::SidecarProtocol(
                "request payload exceeds frame limit".into(),
            ));
        }
        self.exchange(&StdioRequest::Call {
            contract: contract.to_owned(),
            method: method.to_owned(),
            payload: payload.to_owned(),
        })
    }

    pub(crate) fn ensure_running(&self) -> Result<(), ComponentError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(ComponentError::SidecarStopped);
        }
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if child.try_wait()?.is_some() {
            self.stopped.store(true, Ordering::Release);
            return Err(ComponentError::SidecarStopped);
        }
        Ok(())
    }

    fn exchange(&self, request: &StdioRequest) -> Result<Vec<u8>, ComponentError> {
        self.ensure_running()?;
        let request = serde_json::to_vec(request)?;
        super::validate_length(request.len())?;
        let mut sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        let channel = sender.as_ref().ok_or(ComponentError::SidecarStopped)?;
        let (response_sender, response) = mpsc::channel();
        let result = channel
            .send(Exchange {
                request,
                response: response_sender,
            })
            .map_err(|_| ComponentError::SidecarStopped)
            .and_then(|()| match response.recv_timeout(self.timeout) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => Err(ComponentError::SidecarTimeout),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(ComponentError::SidecarStopped),
            })
            .and_then(|bytes| {
                serde_json::from_slice::<StdioResponse>(&bytes)
                    .map_err(|error| ComponentError::SidecarProtocol(format!("invalid response: {error}")))
            });
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                *sender = None;
                self.stop();
                return Err(error);
            }
        };
        if response.status != 0 {
            return Err(ComponentError::SidecarCall {
                status: response.status,
                message: String::from_utf8_lossy(&response.payload).into_owned(),
            });
        }
        Ok(response.payload)
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        // The child may already have exited; wait still reaps it in that case. ~keep
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_descriptor(descriptor: &StdioDescriptor, manifest: &ComponentManifest) -> Result<(), ComponentError> {
    if descriptor.protocol_version != STDIO_PROTOCOL_VERSION {
        return Err(ComponentError::SidecarProtocol("unsupported protocol version".into()));
    }
    if descriptor.abi_major != ABI_MAJOR_V1 || descriptor.abi_minor > ABI_MINOR_V1 {
        return Err(ComponentError::IncompatibleAbi {
            expected_major: ABI_MAJOR_V1,
            expected_minor: ABI_MINOR_V1,
            actual_major: descriptor.abi_major,
            actual_minor: descriptor.abi_minor,
        });
    }
    if descriptor.component_id != manifest.identity.component {
        return Err(ComponentError::IdentityMismatch {
            expected: manifest.identity.component.clone(),
            actual: descriptor.component_id.clone(),
        });
    }
    if descriptor.component_version != manifest.identity.version {
        return Err(ComponentError::SidecarProtocol(
            "component version does not match manifest".into(),
        ));
    }
    if decode_hash(&descriptor.feature_set_hash)? != decode_hash(&manifest.identity.feature_hash)? {
        return Err(ComponentError::FeatureSetHashMismatch);
    }
    if descriptor.provides != manifest.provides {
        return Err(ComponentError::SidecarProtocol(
            "provided contracts do not match manifest".into(),
        ));
    }
    Ok(())
}

/// Calls `try_start`, retrying while the error kind is `ExecutableFileBusy`.
///
/// `attempts` is the total number of calls (0 is treated as 1), so 1 means a single try with no retries. Any
/// other error returns immediately, as does the final busy error once the retries run out.
fn retry_executable_busy<T>(
    attempts: u32,
    backoff: Duration,
    mut try_start: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut attempt = 1_u32;
    loop {
        match try_start() {
            Ok(value) => return Ok(value),
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < attempts => {
                attempt += 1;
                std::thread::sleep(backoff);
            }
            Err(error) => return Err(error),
        }
    }
}

fn make_executable(path: &Path) -> Result<(), ComponentError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(path)?.permissions();
        let mode = permissions.mode();
        if mode & 0o100 == 0 {
            permissions.set_mode(mode | 0o100);
            std::fs::set_permissions(path, permissions)?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn busy_error() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::ExecutableFileBusy, "text file busy")
    }

    #[test]
    fn retries_a_busy_spawn_until_it_succeeds() {
        let calls = AtomicUsize::new(0);
        let mut busy_left = 2;
        let result = retry_executable_busy(10, Duration::ZERO, || {
            calls.fetch_add(1, Ordering::Relaxed);
            if busy_left > 0 {
                busy_left -= 1;
                Err(busy_error())
            } else {
                Ok(7)
            }
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn returns_the_last_busy_error_once_the_retries_run_out() {
        let calls = AtomicUsize::new(0);
        let result = retry_executable_busy(4, Duration::ZERO, || {
            calls.fetch_add(1, Ordering::Relaxed);
            Err::<u32, _>(busy_error())
        });
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn returns_other_errors_on_the_first_call_without_retrying() {
        let calls = AtomicUsize::new(0);
        let result = retry_executable_busy(5, Duration::ZERO, || {
            calls.fetch_add(1, Ordering::Relaxed);
            Err::<u32, _>(std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"))
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_single_attempt_makes_exactly_one_call() {
        let calls = AtomicUsize::new(0);
        let result = retry_executable_busy(1, Duration::ZERO, || {
            calls.fetch_add(1, Ordering::Relaxed);
            Err::<u32, _>(busy_error())
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::ExecutableFileBusy);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
