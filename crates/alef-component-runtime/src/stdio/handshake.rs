//! The `describe` handshake every stdio connection starts with, and protocol negotiation.

use super::{
    STDIO_BINARY_PROTOCOL_VERSION, STDIO_PROTOCOL_VERSION, StdioDescriptor, StdioRequest, StdioResponse, read_frame,
    write_frame,
};
use crate::loader::decode_hash;
use crate::{ComponentError, ComponentManifest};
use alef_component_abi::{ABI_MAJOR_V1, ABI_MINOR_V1};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc;
use std::time::Duration;

/// What the sidecar's descriptor selected for the rest of the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Negotiated {
    /// Protocol 1: JSON envelopes, one request at a time.
    Json,
    /// Protocol 2: binary frames, at most `max_in_flight` unanswered requests.
    Binary { max_in_flight: u32 },
}

/// The child's streams after a successful handshake, and the protocol to speak over them.
pub(super) struct Handshake {
    pub(super) stdin: ChildStdin,
    pub(super) stdout: ChildStdout,
    pub(super) negotiated: Negotiated,
}

/// Send `describe` as a protocol-1 frame and validate the answer against `manifest`. The exchange
/// runs on a helper thread so `timeout` holds even when the child never answers; the caller kills
/// the child on any error, which also ends that thread.
pub(super) fn describe(
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    manifest: &ComponentManifest,
    timeout: Duration,
) -> Result<Handshake, ComponentError> {
    let request = serde_json::to_vec(&StdioRequest::Describe)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("alef-sidecar-describe".into())
        .spawn(move || {
            let response = write_frame(&mut stdin, &request).and_then(|()| read_frame(&mut stdout));
            let _ = sender.send(response.map(|bytes| (stdin, stdout, bytes)));
        })?;
    let (stdin, stdout, bytes) = match receiver.recv_timeout(timeout) {
        Ok(result) => result?,
        Err(mpsc::RecvTimeoutError::Timeout) => return Err(ComponentError::SidecarTimeout),
        Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ComponentError::SidecarStopped),
    };
    let response: StdioResponse = serde_json::from_slice(&bytes)
        .map_err(|error| ComponentError::SidecarProtocol(format!("invalid response: {error}")))?;
    if response.status != 0 {
        return Err(ComponentError::SidecarCall {
            status: response.status,
            message: String::from_utf8_lossy(&response.payload).into_owned(),
        });
    }
    let negotiated = negotiate(&response.payload, manifest)?;
    Ok(Handshake {
        stdin,
        stdout,
        negotiated,
    })
}

/// Parse and validate a describe payload. `max_in_flight` is the one field beyond the protocol-1
/// [`StdioDescriptor`] shape, and only a protocol-2 descriptor may carry it, so a protocol-1
/// descriptor stays exactly what older hosts accept.
pub(super) fn negotiate(payload: &[u8], manifest: &ComponentManifest) -> Result<Negotiated, ComponentError> {
    let invalid =
        |error: serde_json::Error| ComponentError::SidecarProtocol(format!("invalid describe payload: {error}"));
    let mut value: serde_json::Value = serde_json::from_slice(payload).map_err(invalid)?;
    let max_in_flight = value.as_object_mut().and_then(|object| object.remove("max_in_flight"));
    let descriptor: StdioDescriptor = serde_json::from_value(value).map_err(invalid)?;
    validate_descriptor(&descriptor, manifest)?;
    match descriptor.protocol_version {
        STDIO_PROTOCOL_VERSION if max_in_flight.is_some() => Err(ComponentError::SidecarProtocol(
            "max_in_flight requires protocol version 2".into(),
        )),
        STDIO_PROTOCOL_VERSION => Ok(Negotiated::Json),
        _ => {
            let max_in_flight = match max_in_flight {
                None => 1,
                Some(value) => value
                    .as_u64()
                    .and_then(|count| u32::try_from(count).ok())
                    .filter(|count| *count > 0)
                    .ok_or_else(|| {
                        ComponentError::SidecarProtocol(format!(
                            "max_in_flight must be an integer in 1..=4294967295, got {value}"
                        ))
                    })?,
            };
            Ok(Negotiated::Binary { max_in_flight })
        }
    }
}

fn validate_descriptor(descriptor: &StdioDescriptor, manifest: &ComponentManifest) -> Result<(), ComponentError> {
    if descriptor.protocol_version != STDIO_PROTOCOL_VERSION
        && descriptor.protocol_version != STDIO_BINARY_PROTOCOL_VERSION
    {
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
