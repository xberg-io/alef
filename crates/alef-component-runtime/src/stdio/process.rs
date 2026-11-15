use super::binary::{BinaryConnection, CallError};
use super::handshake::{self, Handshake, Negotiated};
use super::{
    MAX_FRAME_SIZE, STDIO_PROTOCOLS_ENV, SidecarRestartEvent, SidecarSession, SidecarSessionSetup, StdioRequest,
    StdioResponse, read_frame, write_frame,
};
use crate::identity::VerifiedFile;
use crate::{ComponentError, ComponentManifest};
use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError, mpsc};
use std::time::Duration;

// On a multi-threaded host, another thread can fork while the just-installed sidecar executable's write
// descriptor is still open, and the child holds that descriptor until `exec` — so the start fails with
// `ExecutableFileBusy` ("Text file busy", os error 26) on Linux. Retrying briefly is what cargo, rustup and
// poly do for the same race.
const SPAWN_BUSY_RETRIES: u32 = 200;
const SPAWN_BUSY_BACKOFF: Duration = Duration::from_millis(10);

/// The protocol versions this host speaks, as advertised to the child in [`STDIO_PROTOCOLS_ENV`].
const ADVERTISED_PROTOCOLS: &str = "1,2";

/// Host environment variables a sidecar inherits; everything else is cleared. A trailing `*`
/// matches a prefix. Names compare case-insensitively on Windows, where the environment does.
const DEFAULT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "TZ",
    "LANG",
    "LANGUAGE",
    "LC_*",
    // Windows processes fail in surprising ways without these.
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "USERPROFILE",
    "PATHEXT",
    "COMSPEC",
];

struct Exchange {
    request: Vec<u8>,
    response: mpsc::Sender<Result<Vec<u8>, ComponentError>>,
}

/// The protocol-specific half of a running connection, chosen by the describe handshake.
enum Connection {
    /// Protocol 1: one exchange at a time through the stdio thread. A caller holds the
    /// `connection` guard for its whole exchange, which is what serializes calls.
    Json(mpsc::Sender<Exchange>),
    /// Protocol 2: pipelined. A caller clones the handle and releases the guard before it waits.
    Binary(Arc<BinaryConnection>),
}

/// Verifies a sidecar's executable against the lock again and returns it open and verified. Every
/// respawn calls it, so a respawn never executes a file that was not checked (ADR-0026).
pub(crate) type SidecarVerifier = Arc<dyn Fn() -> Result<VerifiedFile, ComponentError> + Send + Sync>;

/// Optional respawn knobs for a stdio connection, shared by every [`SidecarProcess`] a
/// [`crate::ComponentManager`] spawns (see its `with_sidecar_*` builders).
#[derive(Clone)]
pub(crate) struct SidecarPolicy {
    pub(crate) restart_budget: u32,
    pub(crate) session: Option<SidecarSessionSetup>,
    pub(crate) on_restart: Option<SidecarRestartEvent>,
    /// Host environment variables passed through on top of [`DEFAULT_ENV_ALLOWLIST`].
    pub(crate) env_passthrough: Vec<String>,
}

impl Default for SidecarPolicy {
    fn default() -> Self {
        Self {
            restart_budget: 1,
            session: None,
            on_restart: None,
            env_passthrough: Vec::new(),
        }
    }
}

pub(crate) struct SidecarProcess {
    verify: SidecarVerifier,
    manifest: ComponentManifest,
    child: Mutex<Child>,
    connection: Mutex<Option<Connection>>,
    timeout: Duration,
    stopped: AtomicBool,
    policy: SidecarPolicy,
    restarts: AtomicU32,
    respawn_attempts: AtomicU32,
    // Respawn serialization. Taken with no other lock held; inside it, `child` (in `stop`) and
    // `connection` (to swap) are taken briefly, `connection` before `child` whenever both are held.
    // A protocol-1 exchange holds `connection` across a whole call, which keeps a respawn waiting
    // behind it; a protocol-2 call holds its own handle to the connection instead, so a respawn
    // replaces the connection under it and the call fails with the old child.
    respawn: Mutex<()>,
}

impl SidecarProcess {
    /// Start the first child from `executable`, which the caller has just verified; `verify`
    /// re-verifies it before every respawn.
    pub(crate) fn spawn(
        executable: &VerifiedFile,
        verify: SidecarVerifier,
        manifest: &ComponentManifest,
        timeout: Duration,
        policy: SidecarPolicy,
    ) -> Result<Self, ComponentError> {
        let (child, connection) = Self::start_child(executable, manifest, timeout, &policy)?;
        let process = Self {
            verify,
            manifest: manifest.clone(),
            child: Mutex::new(child),
            connection: Mutex::new(Some(connection)),
            timeout,
            stopped: AtomicBool::new(false),
            policy,
            restarts: AtomicU32::new(0),
            respawn_attempts: AtomicU32::new(0),
            respawn: Mutex::new(()),
        };
        if let Err(error) = process.run_session() {
            process.stop();
            return Err(error);
        }
        Ok(process)
    }

    /// Start one child for the verified `executable`, run the describe handshake against `manifest`
    /// and connect with the protocol it negotiated. Shared by `spawn` and by every respawn; the
    /// caller installs the returned pair on `self`. A child that fails here is killed and reaped.
    fn start_child(
        executable: &VerifiedFile,
        manifest: &ComponentManifest,
        timeout: Duration,
        policy: &SidecarPolicy,
    ) -> Result<(Child, Connection), ComponentError> {
        let path = executable.path();
        make_executable(path)?;
        let environment = sidecar_environment(std::env::vars_os(), &policy.env_passthrough);
        // The component's own cache directory: never the host's working tree.
        let working_dir = path.parent().unwrap_or(path);
        let mut refused = None;
        let started = retry_executable_busy(SPAWN_BUSY_RETRIES, SPAWN_BUSY_BACKOFF, || {
            // Resolved per attempt, immediately before the spawn: the verified descriptor's path on
            // Linux, else the path after a last identity check (see `identity`).
            let program = executable.executable_launch_path().map_err(|error| {
                refused = Some(error);
                std::io::Error::other("verified sidecar executable changed")
            })?;
            let mut command = Command::new(program);
            #[cfg(unix)]
            std::os::unix::process::CommandExt::arg0(&mut command, path);
            command
                .env_clear()
                .envs(environment.iter().map(|(name, value)| (name, value)))
                .env(STDIO_PROTOCOLS_ENV, ADVERTISED_PROTOCOLS)
                .current_dir(working_dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
        });
        if let Some(error) = refused {
            return Err(error);
        }
        let mut child = started
            .map_err(|error| ComponentError::SidecarProtocol(format!("failed to start {}: {error}", path.display())))?;
        let stdin = child.stdin.take().expect("piped child stdin missing");
        let stdout = child.stdout.take().expect("piped child stdout missing");
        match handshake::describe(stdin, stdout, manifest, timeout).and_then(Self::connect) {
            Ok(connection) => Ok((child, connection)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    fn connect(handshake: Handshake) -> Result<Connection, ComponentError> {
        match handshake.negotiated {
            Negotiated::Json => Self::json_channel(handshake.stdin, handshake.stdout).map(Connection::Json),
            Negotiated::Binary { max_in_flight } => {
                BinaryConnection::start(handshake.stdin, BufReader::new(handshake.stdout), max_in_flight)
                    .map(|binary| Connection::Binary(Arc::new(binary)))
            }
        }
    }

    /// The protocol-1 exchange thread: writes one request frame, reads its response, repeats.
    fn json_channel(mut stdin: ChildStdin, mut stdout: ChildStdout) -> Result<mpsc::Sender<Exchange>, ComponentError> {
        let (sender, receiver) = mpsc::channel::<Exchange>();
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
        Ok(sender)
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
        check_payload(payload)?;
        self.ensure_running()?;
        self.exchange(contract, method, payload, true)
    }

    /// Like [`Self::invoke`], for a [`SidecarSession`]: the caller has just established that the
    /// connection is running, so the liveness gate (which may respawn) is skipped.
    pub(crate) fn invoke_for_session(
        &self,
        contract: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, ComponentError> {
        check_payload(payload)?;
        self.exchange(contract, method, payload, false)
    }

    pub(crate) fn restart_count(&self) -> u32 {
        self.restarts.load(Ordering::Acquire)
    }

    /// Passive liveness probe: the connection is not stopped, its transport has not failed and the
    /// child has not exited. Never respawns, never spends the restart budget.
    pub(crate) fn is_running(&self) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let exited = {
            let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            child.try_wait().is_ok_and(|status| status.is_some())
        };
        !exited && !self.transport_failed()
    }

    pub(crate) fn ensure_running(&self) -> Result<(), ComponentError> {
        if self.stopped.load(Ordering::Acquire) {
            return self.try_respawn();
        }
        let exited = {
            let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            child.try_wait()?.is_some()
        };
        if !exited && !self.transport_failed() {
            return Ok(());
        }
        self.stopped.store(true, Ordering::Release);
        // The child lock is dropped before respawning: `try_respawn` re-takes it in `stop`, and the
        // respawn mutex must never be waited for while holding `child` or `connection`.
        self.try_respawn()
    }

    /// Whether a protocol-2 connection's reader or writer has already failed (a broken frame, an
    /// unknown response id, a failed write) while the child may still be running. Never waits for
    /// the connection guard: a protocol-1 exchange holds it for a whole call, and a protocol-1
    /// connection never fails this way. A protocol-2 failure missed here is caught by the next call.
    fn transport_failed(&self) -> bool {
        let connection = match self.connection.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        matches!(connection.as_ref(), Some(Connection::Binary(binary)) if binary.is_dead())
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
            // The cached file is checked against the lock again before it is executed: the
            // verification behind the first spawn says nothing about the file now.
            let executable = (self.verify)()?;
            let (child, connection) = Self::start_child(&executable, &self.manifest, self.timeout, &self.policy)?;
            {
                // Swapped together under the connection guard, so a failing protocol-2 call that
                // checks its handle against the current connection never stops the new child.
                let mut current = self.lock_connection();
                *self.child.lock().unwrap_or_else(PoisonError::into_inner) = child;
                *current = Some(connection);
            }
            if let Err(error) = self.run_session() {
                self.stop();
                return Err(error);
            }
            // Published only once the session setup has succeeded, so nothing is dispatched to the
            // new child before it is ready to serve the host's calls (the gated call path re-checks
            // `stopped` under the connection guard).
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

    /// The call itself, over the connection installed on `self`. When `gated`, it re-checks
    /// `stopped` after taking the connection guard: a respawn swaps the connection under that guard
    /// and publishes `stopped = false` only once its session setup has finished, so this re-check
    /// is what keeps a call that raced the liveness gate from reaching the new child before it is
    /// ready to serve host calls. Session-setup calls are not gated.
    fn exchange(&self, contract: &str, method: &str, payload: &[u8], gated: bool) -> Result<Vec<u8>, ComponentError> {
        let mut connection = self.lock_connection();
        if gated && self.stopped.load(Ordering::Acquire) {
            return Err(ComponentError::SidecarStopped);
        }
        let binary = match connection.as_ref() {
            None => return Err(ComponentError::SidecarStopped),
            Some(Connection::Json(_)) => return self.call_json(&mut connection, contract, method, payload),
            Some(Connection::Binary(binary)) => Arc::clone(binary),
        };
        drop(connection);
        self.call_binary(&binary, contract, method, payload)
    }

    /// A protocol-2 call. Only a fatal failure stops the connection, and only if it is still the
    /// one the call was sent on.
    fn call_binary(
        &self,
        binary: &Arc<BinaryConnection>,
        contract: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, ComponentError> {
        match binary.call(contract, method, payload, self.timeout) {
            Ok((status, payload)) => status_result(status, payload),
            Err(CallError::Recoverable(error)) => Err(error),
            Err(CallError::Fatal(error)) => {
                let mut current = self.lock_connection();
                if matches!(current.as_ref(), Some(Connection::Binary(installed)) if Arc::ptr_eq(installed, binary)) {
                    *current = None;
                    self.stop();
                }
                Err(error)
            }
        }
    }

    /// A protocol-1 exchange, the connection guard held throughout. A transport failure kills the
    /// connection; the next gated call respawns it.
    fn call_json(
        &self,
        connection: &mut MutexGuard<'_, Option<Connection>>,
        contract: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, ComponentError> {
        let request = serde_json::to_vec(&StdioRequest::Call {
            contract: contract.to_owned(),
            method: method.to_owned(),
            payload: payload.to_owned(),
        })?;
        // Checked before anything is sent: a request whose JSON encoding outgrows the frame limit
        // is the caller's error and leaves the connection usable.
        super::validate_length(request.len())?;
        let Some(Connection::Json(channel)) = connection.as_ref() else {
            return Err(ComponentError::SidecarStopped);
        };
        match Self::json_request(channel, self.timeout, request) {
            Ok(response) => status_result(response.status, response.payload),
            Err(error) => {
                **connection = None;
                self.stop();
                Err(error)
            }
        }
    }

    /// Send one protocol-1 request frame over `channel` and collect its response envelope, with no
    /// side effects of its own: the caller decides what a transport failure means.
    fn json_request(
        channel: &mpsc::Sender<Exchange>,
        timeout: Duration,
        request: Vec<u8>,
    ) -> Result<StdioResponse, ComponentError> {
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

    fn lock_connection(&self) -> MutexGuard<'_, Option<Connection>> {
        self.connection.lock().unwrap_or_else(PoisonError::into_inner)
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

/// The subset of `vars` a sidecar inherits: [`DEFAULT_ENV_ALLOWLIST`] plus the host's `extra`
/// patterns. The protocol variable is never inherited; the host sets it itself.
fn sidecar_environment(
    vars: impl Iterator<Item = (OsString, OsString)>,
    extra: &[String],
) -> Vec<(OsString, OsString)> {
    vars.filter(|(name, _)| {
        name.to_str().is_some_and(|name| {
            !env_name_matches(STDIO_PROTOCOLS_ENV, name)
                && DEFAULT_ENV_ALLOWLIST
                    .iter()
                    .copied()
                    .chain(extra.iter().map(String::as_str))
                    .any(|pattern| env_name_matches(pattern, name))
        })
    })
    .collect()
}

fn env_name_matches(pattern: &str, name: &str) -> bool {
    let same = |left: &str, right: &str| {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    };
    match pattern.strip_suffix('*') {
        Some(prefix) => name.get(..prefix.len()).is_some_and(|head| same(head, prefix)),
        None => same(pattern, name),
    }
}

fn check_payload(payload: &[u8]) -> Result<(), ComponentError> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ComponentError::SidecarProtocol(
            "request payload exceeds frame limit".into(),
        ));
    }
    Ok(())
}

fn status_result(status: i32, payload: Vec<u8>) -> Result<Vec<u8>, ComponentError> {
    if status != 0 {
        return Err(ComponentError::SidecarCall {
            status,
            message: String::from_utf8_lossy(&payload).into_owned(),
        });
    }
    Ok(payload)
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

    fn vars(names: &[&str]) -> impl Iterator<Item = (OsString, OsString)> {
        names
            .iter()
            .map(|name| (OsString::from(name), OsString::from("value")))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn names(environment: &[(OsString, OsString)]) -> Vec<&str> {
        environment.iter().map(|(name, _)| name.to_str().unwrap()).collect()
    }

    #[test]
    fn the_sidecar_environment_keeps_only_allowlisted_variables() {
        let host = vars(&[
            "PATH",
            "HOME",
            "LC_ALL",
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            STDIO_PROTOCOLS_ENV,
        ]);
        let environment = sidecar_environment(host, &[]);
        assert_eq!(names(&environment), ["PATH", "HOME", "LC_ALL"]);
    }

    #[test]
    fn the_host_extends_the_allowlist_with_names_and_prefixes() {
        let host = vars(&[
            "PATH",
            "POLY_CONFIG",
            "POLY_CACHE",
            "RUST_LOG",
            "SECRET",
            STDIO_PROTOCOLS_ENV,
        ]);
        let extra = [
            "POLY_*".to_owned(),
            "RUST_LOG".to_owned(),
            STDIO_PROTOCOLS_ENV.to_owned(),
        ];
        let environment = sidecar_environment(host, &extra);
        assert_eq!(names(&environment), ["PATH", "POLY_CONFIG", "POLY_CACHE", "RUST_LOG"]);
    }

    #[test]
    fn env_patterns_match_whole_names_or_prefixes() {
        assert!(env_name_matches("LC_*", "LC_CTYPE"));
        assert!(!env_name_matches("LC_*", "LC"));
        assert!(!env_name_matches("PATH", "PATHEXT"));
        assert_eq!(env_name_matches("PATH", "Path"), cfg!(windows));
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
