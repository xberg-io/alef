//! Framed stdio transport for bytes-only named component contracts.
//!
//! An executable stays alive for its component's lifetime. Stdin and stdout carry only frames;
//! stderr is inherited for diagnostics. A timeout or protocol failure that leaves the connection
//! unusable terminates and reaps the child; the connection is then respawned from the verified
//! executable on a later call, up to the manager's restart budget, re-running the describe
//! validation and the host's session setup ([`SidecarSessionSetup`]) each time. Producer status
//! failures leave the connection usable.
//!
//! The host starts the child with [`STDIO_PROTOCOLS_ENV`] listing the protocol versions it speaks
//! (`1,2`). The first request is always a protocol-1 JSON frame: a four-byte little-endian
//! unsigned JSON byte length, followed by `{"op":"describe"}`. Its successful response payload
//! contains JSON matching [`StdioDescriptor`], checked against the verified artifact's manifest.
//! The descriptor's `protocol_version` selects the framing of every later frame:
//!
//! * Protocol 1 ([`STDIO_PROTOCOL_VERSION`]): JSON envelopes,
//!   `{"op":"call","contract":"name","method":"name","payload":[0,255]}` answered by
//!   `{"status":0,"payload":[0,255]}`, with payloads as JSON byte arrays. Calls are serialized
//!   across all contracts sharing the executable, and a timeout is fatal.
//! * Protocol 2 ([`STDIO_BINARY_PROTOCOL_VERSION`]): binary frames ([`frame`]) carrying raw payload
//!   bytes and a request id. A protocol-2 descriptor may add `max_in_flight` (default 1): the host
//!   pipelines up to that many unanswered requests and the sidecar may answer them in any order.
//!   A timeout abandons only its own request (a late response is dropped) unless the sidecar
//!   answered nothing during the whole deadline, which is fatal.
//!
//! Status codes use `AlefStatus` semantics; a nonzero status carries error message bytes. The wire
//! format is specified in `docs/design/component-stdio-binary-protocol.md`.
//!
//! This transport supports [`crate::LoadedComponent::invoke_bytes`]. Arbitrary generated C
//! function tables cannot be represented across a process boundary: typed table access and
//! typed instance creation return an explicit unsupported-transport error.

mod binary;
pub mod frame;
mod handshake;
mod process;

pub(crate) use process::{SidecarPolicy, SidecarProcess, SidecarVerifier};

use crate::{ComponentError, ComponentProvidedContract};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::Arc;

pub use frame::{
    BINARY_FRAME_MAGIC, BINARY_FRAME_PREFIX_LEN, BinaryFrame, BinaryFrameKind, MAX_BINARY_HEADER_SIZE,
    RESPONSE_HEADER_LEN, decode_call_header, decode_response_header, encode_binary_frame, encode_call_header,
    encode_response_header, read_binary_frame, write_binary_frame,
};

/// The JSON-envelope protocol every sidecar speaks for `describe`, and for every later frame when
/// its descriptor declares this version.
pub const STDIO_PROTOCOL_VERSION: u32 = 1;
/// The binary-frame protocol: raw payload bytes, request ids, optional pipelining.
pub const STDIO_BINARY_PROTOCOL_VERSION: u32 = 2;
/// Environment variable the host sets on the child: the comma-separated protocol versions it
/// speaks. A sidecar declares protocol 2 only when this lists `2`, so it still serves older hosts.
pub const STDIO_PROTOCOLS_ENV: &str = "ALEF_STDIO_PROTOCOLS";
/// Largest JSON body of a protocol-1 frame, and largest payload of a protocol-2 frame.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Metadata returned as JSON inside the successful `describe` response payload. A protocol-2
/// descriptor may also carry `max_in_flight`, which the runtime reads separately so this type stays
/// exactly the protocol-1 shape.
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
