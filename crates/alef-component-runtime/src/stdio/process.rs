use super::{
    STDIO_PROTOCOL_VERSION, SidecarRestartEvent, SidecarSession, SidecarSessionSetup, StdioDescriptor, StdioRequest,
    StdioResponse, read_frame, write_frame,
};
use crate::loader::decode_hash;
use crate::{ComponentError, ComponentManifest};
use alef_component_abi::{ABI_MAJOR_V1, ABI_MINOR_V1};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// Optional respawn knobs for a stdio connection, shared by every [`SidecarProcess`] a
/// [`crate::ComponentManager`] spawns (see its `with_sidecar_*` builders).
#[derive(Clone)]
pub(crate) struct SidecarPolicy {
    pub(crate) restart_budget: u32,
    pub(crate) session: Option<SidecarSessionSetup>,
    pub(crate) on_restart: Option<SidecarRestartEvent>,
}

impl Default for SidecarPolicy {
    fn default() -> Self {
        Self {
            restart_budget: 1,
            session: None,
            on_restart: None,
        }
    }
}

pub(crate) struct SidecarProcess {
    executable: PathBuf,
    manifest: ComponentManifest,
    child: Mutex<Child>,
    sender: Mutex<Option<mpsc::Sender<Exchange>>>,
    timeout: Duration,
    stopped: AtomicBool,
    policy: SidecarPolicy,
    restarts: AtomicU32,
    respawn_attempts: AtomicU32,
    // Respawn serialization. Taken with no other lock held; inside it, `child` (in `stop`) and
    // `sender` (to swap) are each taken briefly. `exchange` holds `sender` across a whole call,
    // which is what keeps a respawn waiting behind an in-flight call.
    respawn: Mutex<()>,
}

impl SidecarProcess {
    pub(crate) fn spawn(
        executable: &Path,
        manifest: &ComponentManifest,
        timeout: Duration,
        policy: SidecarPolicy,
    ) -> Result<Self, ComponentError> {
        let (child, channel) = Self::start_child(executable)?;
        let process = Self {
            executable: executable.to_owned(),
            manifest: manifest.clone(),
            child: Mutex::new(child),
            sender: Mutex::new(None),
            timeout,
            stopped: AtomicBool::new(false),
            policy,
            restarts: AtomicU32::new(0),
            respawn_attempts: AtomicU32::new(0),
            respawn: Mutex::new(()),
        };
        process.describe(&channel)?;
        *process.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(channel);
        if let Err(error) = process.run_session() {
            process.stop();
            return Err(error);
        }
        Ok(process)
    }

    /// Start one child for `executable` plus its framed-exchange reader thread. Shared by `spawn`
    /// and by every respawn; the caller installs the returned pair on `self` (or cleans it up).
    fn start_child(executable: &Path) -> Result<(Child, mpsc::Sender<Exchange>), ComponentError> {
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
        let reader = std::thread::Builder::new()
            .name("alef-sidecar-stdio".into())
            .spawn(move || {
                while let Ok(exchange) = receiver.recv() {
                    let response = write_frame(&mut stdin, &exchange.request).and_then(|()| read_frame(&mut stdout));
                    let failed = response.is_err();
                    if exchange.response.send(response).is_err() || failed {
                        break;
                    }
                }
            });
        match reader {
            Ok(_reader) => Ok((child, sender)),
            Err(error) => {
                // Without the reader thread nothing drains the child, and the streams moved into
                // the closure die with it: do not leave the child running.
                let _ = child.kill();
                let _ = child.wait();
                Err(error.into())
            }
        }
    }

    /// Describe a freshly started child over its not-yet-installed channel and check the descriptor
    /// against the manifest this process was spawned from.
    fn describe(&self, channel: &mpsc::Sender<Exchange>) -> Result<(), ComponentError> {
        let response = Self::request(channel, self.timeout, &StdioRequest::Describe)?;
        if response.status != 0 {
            return Err(ComponentError::SidecarCall {
                status: response.status,
                message: String::from_utf8_lossy(&response.payload).into_owned(),
            });
        }
        let descriptor: StdioDescriptor = serde_json::from_slice(&response.payload)
            .map_err(|error| ComponentError::SidecarProtocol(format!("invalid describe payload: {error}")))?;
        validate_descriptor(&descriptor, &self.manifest)
    }

    /// Replay the host's session setup on this connection. Its calls bypass the liveness gate: the
    /// setup runs while the connection is still marked stopped, and re-entering the respawn it is
    /// part of would block on the respawn mutex forever.
    fn run_session(&self) -> Result<(), ComponentError> {
        let Some(setup) = self.policy.session.as_ref() else {
            return Ok(());
        };
        setup(&SidecarSession { process: self })
    }

    pub(crate) fn invoke(&self, contract: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, ComponentError> {
        let request = Self::call_request(contract, method, payload)?;
        self.exchange(&request)
    }

    /// Like [`Self::invoke`], for a [`SidecarSession`]: the caller has just established that the
    /// connection is running, so the liveness gate (which may respawn) is skipped.
    pub(crate) fn invoke_for_session(
        &self,
        contract: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, ComponentError> {
        let request = Self::call_request(contract, method, payload)?;
        self.exchange_running(&request)
    }

    fn call_request(contract: &str, method: &str, payload: &[u8]) -> Result<StdioRequest, ComponentError> {
        if payload.len() > super::MAX_FRAME_SIZE {
            return Err(ComponentError::SidecarProtocol(
                "request payload exceeds frame limit".into(),
            ));
        }
        Ok(StdioRequest::Call {
            contract: contract.to_owned(),
            method: method.to_owned(),
            payload: payload.to_owned(),
        })
    }

    pub(crate) fn restart_count(&self) -> u32 {
        self.restarts.load(Ordering::Acquire)
    }

    /// Passive liveness probe: the connection is not stopped and the child has not exited. Never
    /// respawns, never spends the restart budget.
    pub(crate) fn is_running(&self) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let exited = {
            let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            child.try_wait().is_ok_and(|status| status.is_some())
        };
        !exited
    }

    pub(crate) fn ensure_running(&self) -> Result<(), ComponentError> {
        if self.stopped.load(Ordering::Acquire) {
            return self.try_respawn();
        }
        let exited = {
            let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            child.try_wait()?.is_some()
        };
        if !exited {
            return Ok(());
        }
        self.stopped.store(true, Ordering::Release);
        // The child lock is dropped before respawning: `try_respawn` re-takes it in `stop`, and the
        // respawn mutex must never be waited for while holding `child` or `sender`.
        self.try_respawn()
    }

    /// Replace a dead connection with a fresh child, within [`SidecarPolicy::restart_budget`]. The
    /// failing call that stopped the connection returned long ago: a respawn happens only on a
    /// later `ensure_running`, so the next call is the first one the new child serves and no error
    /// is ever retried transparently.
    fn try_respawn(&self) -> Result<(), ComponentError> {
        let restarts = {
            let _guard = self.respawn.lock().unwrap_or_else(PoisonError::into_inner);
            if !self.stopped.load(Ordering::Acquire) {
                // A concurrent caller completed a respawn while this one waited for the mutex.
                return Ok(());
            }
            // Attempts, not successes, consume the budget: a permanently broken executable must not
            // cycle the connection through start-fail rounds forever.
            if self.respawn_attempts.load(Ordering::Acquire) >= self.policy.restart_budget {
                return Err(ComponentError::SidecarStopped);
            }
            self.respawn_attempts.fetch_add(1, Ordering::AcqRel);
            // Reaps the old child, whether it was killed earlier or exited on its own.
            self.stop();
            let (mut child, channel) = Self::start_child(&self.executable)?;
            if let Err(error) = self.describe(&channel) {
                // This child is not adopted; do not leave it lingering past the failed respawn.
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            *self.child.lock().unwrap_or_else(PoisonError::into_inner) = child;
            *self.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(channel);
            if let Err(error) = self.run_session() {
                self.stop();
                return Err(error);
            }
            // Published only once the session setup has succeeded, so nothing is dispatched to the
            // new child before it is ready to serve the host's calls (the gated call path re-checks
            // `stopped` under the sender guard).
            self.stopped.store(false, Ordering::Release);
            // The count is committed while the guard is still held, so no later respawn can record
            // itself before this one; the handler runs after the guard is released, because it may
            // call into the component.
            self.restarts.fetch_add(1, Ordering::AcqRel) + 1
        };
        if let Some(on_restart) = self.policy.on_restart.as_ref() {
            on_restart(&self.manifest.identity.component, restarts);
        }
        Ok(())
    }

    fn exchange(&self, request: &StdioRequest) -> Result<Vec<u8>, ComponentError> {
        self.ensure_running()?;
        self.exchange_gated(request)
    }

    /// The call itself, over the channel installed on `self` (the caller has ensured the process is
    /// running). A transport failure kills the connection; the next call respawns it.
    fn exchange_running(&self, request: &StdioRequest) -> Result<Vec<u8>, ComponentError> {
        let mut sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        self.dispatch(&mut sender, request)
    }

    /// The gated call path. After taking the sender guard it re-checks `stopped`: a respawn swaps
    /// the channel under that guard and publishes `stopped = false` only once its session setup has
    /// finished, so this re-check is what keeps a call that raced the liveness gate from reaching
    /// the new child before it is ready to serve host calls.
    fn exchange_gated(&self, request: &StdioRequest) -> Result<Vec<u8>, ComponentError> {
        let mut sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
        if self.stopped.load(Ordering::Acquire) {
            return Err(ComponentError::SidecarStopped);
        }
        self.dispatch(&mut sender, request)
    }

    /// Send one request over the installed channel, the sender guard already held. A transport
    /// failure kills the connection; the next gated call respawns it.
    fn dispatch(
        &self,
        sender: &mut Option<mpsc::Sender<Exchange>>,
        request: &StdioRequest,
    ) -> Result<Vec<u8>, ComponentError> {
        let channel = sender.as_ref().ok_or(ComponentError::SidecarStopped)?;
        let response = match Self::request(channel, self.timeout, request) {
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

    /// Send one framed request over `channel` and collect its response envelope, with no side
    /// effects of its own: the caller decides what a transport failure means for the connection.
    fn request(
        channel: &mpsc::Sender<Exchange>,
        timeout: Duration,
        request: &StdioRequest,
    ) -> Result<StdioResponse, ComponentError> {
        let request = serde_json::to_vec(request)?;
        super::validate_length(request.len())?;
        let (response_sender, response) = mpsc::channel();
        channel
            .send(Exchange {
                request,
                response: response_sender,
            })
            .map_err(|_| ComponentError::SidecarStopped)
            .and_then(|()| match response.recv_timeout(timeout) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => Err(ComponentError::SidecarTimeout),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(ComponentError::SidecarStopped),
            })
            .and_then(|bytes| {
                serde_json::from_slice::<StdioResponse>(&bytes)
                    .map_err(|error| ComponentError::SidecarProtocol(format!("invalid response: {error}")))
            })
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
