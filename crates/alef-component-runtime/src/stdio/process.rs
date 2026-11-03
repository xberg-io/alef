use super::{STDIO_PROTOCOL_VERSION, StdioDescriptor, StdioRequest, StdioResponse, read_frame, write_frame};
use crate::loader::decode_hash;
use crate::{ComponentError, ComponentManifest};
use alef_component_abi::{ABI_MAJOR_V1, ABI_MINOR_V1};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::Duration;

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
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
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
