//! Framed stdio transport for bytes-only named component contracts.
//!
//! An executable stays alive for its component's lifetime. Stdin and stdout carry only frames:
//! a four-byte little-endian unsigned JSON byte length, followed by that JSON. Stderr is
//! inherited for diagnostics. Frames are bounded by [`MAX_FRAME_SIZE`]. Calls are serialized
//! across all contracts sharing the executable. A timeout or protocol failure terminates and
//! reaps the child; the connection is then respawned from the verified executable on a later
//! call, up to the manager's restart budget, re-running the describe validation and the host's
//! session setup ([`SidecarSessionSetup`]) each time. Producer status failures leave the
//! connection usable.
//!
//! The first request is `{"op":"describe"}`. Its successful response payload contains JSON
//! matching [`StdioDescriptor`], checked against the verified artifact's manifest. Subsequent
//! requests use `{"op":"call","contract":"name","method":"name","payload":[0,255]}`.
//! Every response is `{"status":0,"payload":[0,255]}`. Status codes use `AlefStatus` semantics;
//! a nonzero status carries error message bytes. Payloads are JSON byte arrays, not base64.
//!
//! This transport supports [`crate::LoadedComponent::invoke_bytes`]. Arbitrary generated C
//! function tables cannot be represented across a process boundary: typed table access and
//! typed instance creation return an explicit unsupported-transport error.

mod process;

pub(crate) use process::{SidecarPolicy, SidecarProcess};

use crate::{ComponentError, ComponentProvidedContract};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::Arc;

pub const STDIO_PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Metadata returned as JSON inside the successful `describe` response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioDescriptor {
    pub protocol_version: u32,
    pub abi_major: u32,
    pub abi_minor: u32,
    pub component_id: String,
    pub component_version: String,
    pub feature_set_hash: String,
    pub provides: Vec<ComponentProvidedContract>,
}

/// A request envelope. A producer must answer exactly once for every request.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum StdioRequest {
    Describe,
    Call {
        contract: String,
        method: String,
        payload: Vec<u8>,
    },
}

/// A response envelope, using the component ABI's integer status codes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioResponse {
    pub status: i32,
    pub payload: Vec<u8>,
}

/// Read one complete bounded frame. EOF, including between frames, is an error.
pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, ComponentError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).map_err(protocol_io)?;
    let length = u32::from_le_bytes(header) as usize;
    validate_length(length)?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).map_err(protocol_io)?;
    Ok(payload)
}

/// Write and flush one bounded frame.
pub fn write_frame(writer: &mut impl Write, payload: &[u8]) -> Result<(), ComponentError> {
    validate_length(payload.len())?;
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .map_err(protocol_io)?;
    writer.write_all(payload).map_err(protocol_io)?;
    writer.flush().map_err(protocol_io)
}

fn validate_length(length: usize) -> Result<(), ComponentError> {
    if length == 0 || length > MAX_FRAME_SIZE {
        return Err(ComponentError::SidecarProtocol(format!(
            "frame length {length} is outside 1..={MAX_FRAME_SIZE}"
        )));
    }
    Ok(())
}

/// One stdio connection as seen by a host's session setup: a [`SidecarSessionSetup`] receives it
/// once a sidecar child has started and passed describe validation, and calls
/// [`SidecarSession::invoke`] to re-establish whatever session the host had built on the previous
/// connection.
pub struct SidecarSession<'a> {
    pub(crate) process: &'a SidecarProcess,
}

impl SidecarSession<'_> {
    /// Invoke a bytes-only method on the connection's component, as
    /// [`crate::LoadedComponent::invoke_bytes`] does.
    pub fn invoke(&self, contract: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, ComponentError> {
        self.process.invoke_for_session(contract, method, payload)
    }
}

/// Host-provided setup for a stdio sidecar connection, replayed after the first spawn and after
/// every respawn of the connection (see [`crate::ComponentManager::with_sidecar_session`]).
pub type SidecarSessionSetup = Arc<dyn Fn(&SidecarSession) -> Result<(), ComponentError> + Send + Sync>;

/// Handler called with the component id and the new restart count after each successful stdio
/// respawn (see [`crate::ComponentManager::with_sidecar_restart_event`]).
pub type SidecarRestartEvent = Arc<dyn Fn(&str, u32) + Send + Sync>;

fn protocol_io(error: std::io::Error) -> ComponentError {
    ComponentError::SidecarProtocol(format!("frame I/O failed: {error}"))
}
