//! Binary frames of stdio protocol 2. The byte layout is specified in
//! `docs/design/component-stdio-binary-protocol.md`; this module is its reference implementation.

use super::MAX_FRAME_SIZE;
use crate::ComponentError;
use std::io::{ErrorKind, Read, Write};

/// First byte of every protocol-2 frame. A reader that sees anything else has lost frame sync.
pub const BINARY_FRAME_MAGIC: u8 = 0xA2;
/// Length of the fixed prefix that precedes a frame's header and payload bytes.
pub const BINARY_FRAME_PREFIX_LEN: usize = 20;
/// Upper bound on a frame's header bytes. Payload bytes are bounded by [`MAX_FRAME_SIZE`].
pub const MAX_BINARY_HEADER_SIZE: usize = 64 * 1024;
/// Header length of a [`BinaryFrameKind::Response`] frame: one little-endian `i32` status.
pub const RESPONSE_HEADER_LEN: usize = 4;

/// What a protocol-2 frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BinaryFrameKind {
    /// Host to sidecar: invoke `method` of `contract`. Header: see [`encode_call_header`].
    Call = 1,
    /// Sidecar to host: the answer to the call with the same request id. Header: an `i32` status.
    Response = 2,
}

impl BinaryFrameKind {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Call),
            2 => Some(Self::Response),
            _ => None,
        }
    }
}

/// One decoded protocol-2 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFrame {
    pub kind: BinaryFrameKind,
    /// Host-assigned request id, never 0. A response carries the id of the call it answers.
    pub id: u64,
    pub header: Vec<u8>,
    pub payload: Vec<u8>,
}

/// Encode one frame into a single buffer, so it can be written with one call.
pub fn encode_binary_frame(
    kind: BinaryFrameKind,
    id: u64,
    header: &[u8],
    payload: &[u8],
) -> Result<Vec<u8>, ComponentError> {
    let prefix = frame_prefix(kind, id, header.len(), payload.len())?;
    let mut frame = Vec::with_capacity(BINARY_FRAME_PREFIX_LEN + header.len() + payload.len());
    frame.extend_from_slice(&prefix);
    frame.extend_from_slice(header);
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Write and flush one frame.
pub fn write_binary_frame(
    writer: &mut impl Write,
    kind: BinaryFrameKind,
    id: u64,
    header: &[u8],
    payload: &[u8],
) -> Result<(), ComponentError> {
    let prefix = frame_prefix(kind, id, header.len(), payload.len())?;
    writer.write_all(&prefix).map_err(super::protocol_io)?;
    writer.write_all(header).map_err(super::protocol_io)?;
    writer.write_all(payload).map_err(super::protocol_io)?;
    writer.flush().map_err(super::protocol_io)
}

/// Read one complete frame. EOF anywhere, including between frames, is an error.
pub fn read_binary_frame(reader: &mut impl Read) -> Result<BinaryFrame, ComponentError> {
    let mut prefix = [0_u8; BINARY_FRAME_PREFIX_LEN];
    reader.read_exact(&mut prefix).map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            ComponentError::SidecarProtocol("stream closed before a complete frame prefix".into())
        } else {
            super::protocol_io(error)
        }
    })?;
    if prefix[0] != BINARY_FRAME_MAGIC {
        return Err(protocol(format!(
            "frame magic is {:#04x}, expected {BINARY_FRAME_MAGIC:#04x}",
            prefix[0]
        )));
    }
    let kind =
        BinaryFrameKind::from_byte(prefix[1]).ok_or_else(|| protocol(format!("unknown frame kind {}", prefix[1])))?;
    let flags = u16::from_le_bytes([prefix[2], prefix[3]]);
    if flags != 0 {
        return Err(protocol(format!("frame flags {flags:#06x} are reserved and must be 0")));
    }
    let id = u64::from_le_bytes(prefix[4..12].try_into().expect("eight id bytes"));
    let header_len = u32::from_le_bytes(prefix[12..16].try_into().expect("four length bytes")) as usize;
    let payload_len = u32::from_le_bytes(prefix[16..20].try_into().expect("four length bytes")) as usize;
    validate(id, header_len, payload_len)?;
    let mut header = vec![0; header_len];
    reader.read_exact(&mut header).map_err(super::protocol_io)?;
    let mut payload = vec![0; payload_len];
    reader.read_exact(&mut payload).map_err(super::protocol_io)?;
    Ok(BinaryFrame {
        kind,
        id,
        header,
        payload,
    })
}

/// Encode a call header: the contract name's byte length as a little-endian `u16`, the contract
/// name, then the method name, which runs to the end of the header. Both are UTF-8.
pub fn encode_call_header(contract: &str, method: &str) -> Result<Vec<u8>, ComponentError> {
    let contract_len = u16::try_from(contract.len())
        .map_err(|_| protocol(format!("contract name of {} bytes exceeds 65535", contract.len())))?;
    let mut header = Vec::with_capacity(2 + contract.len() + method.len());
    header.extend_from_slice(&contract_len.to_le_bytes());
    header.extend_from_slice(contract.as_bytes());
    header.extend_from_slice(method.as_bytes());
    if header.len() > MAX_BINARY_HEADER_SIZE {
        return Err(protocol(format!(
            "call header of {} bytes exceeds {MAX_BINARY_HEADER_SIZE}",
            header.len()
        )));
    }
    Ok(header)
}

/// Decode a call header into its contract and method names.
pub fn decode_call_header(header: &[u8]) -> Result<(&str, &str), ComponentError> {
    let Some((length, rest)) = header.split_first_chunk::<2>() else {
        return Err(protocol("call header is shorter than its contract length".into()));
    };
    let contract_len = u16::from_le_bytes(*length) as usize;
    if contract_len > rest.len() {
        return Err(protocol(format!(
            "call header contract length {contract_len} exceeds the {} bytes that follow",
            rest.len()
        )));
    }
    let (contract, method) = rest.split_at(contract_len);
    let contract = std::str::from_utf8(contract).map_err(|_| protocol("call contract is not UTF-8".into()))?;
    let method = std::str::from_utf8(method).map_err(|_| protocol("call method is not UTF-8".into()))?;
    Ok((contract, method))
}

/// Encode a response header: the `AlefStatus` code as a little-endian `i32`.
#[must_use]
pub fn encode_response_header(status: i32) -> [u8; RESPONSE_HEADER_LEN] {
    status.to_le_bytes()
}

/// Decode a response header into its status code.
pub fn decode_response_header(header: &[u8]) -> Result<i32, ComponentError> {
    let bytes: [u8; RESPONSE_HEADER_LEN] = header.try_into().map_err(|_| {
        protocol(format!(
            "response header is {} bytes, expected {RESPONSE_HEADER_LEN}",
            header.len()
        ))
    })?;
    Ok(i32::from_le_bytes(bytes))
}

fn frame_prefix(
    kind: BinaryFrameKind,
    id: u64,
    header_len: usize,
    payload_len: usize,
) -> Result<[u8; BINARY_FRAME_PREFIX_LEN], ComponentError> {
    validate(id, header_len, payload_len)?;
    let mut prefix = [0_u8; BINARY_FRAME_PREFIX_LEN];
    prefix[0] = BINARY_FRAME_MAGIC;
    prefix[1] = kind as u8;
    prefix[4..12].copy_from_slice(&id.to_le_bytes());
    // Both lengths fit in u32: `validate` bounds them by limits far below u32::MAX. ~keep
    prefix[12..16].copy_from_slice(&(header_len as u32).to_le_bytes());
    prefix[16..20].copy_from_slice(&(payload_len as u32).to_le_bytes());
    Ok(prefix)
}

fn validate(id: u64, header_len: usize, payload_len: usize) -> Result<(), ComponentError> {
    if id == 0 {
        return Err(protocol("request id 0 is reserved".into()));
    }
    if header_len > MAX_BINARY_HEADER_SIZE {
        return Err(protocol(format!(
            "frame header length {header_len} exceeds {MAX_BINARY_HEADER_SIZE}"
        )));
    }
    if payload_len > MAX_FRAME_SIZE {
        return Err(protocol(format!(
            "frame payload length {payload_len} exceeds {MAX_FRAME_SIZE}"
        )));
    }
    Ok(())
}

fn protocol(message: String) -> ComponentError {
    ComponentError::SidecarProtocol(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip(kind: BinaryFrameKind, id: u64, header: &[u8], payload: &[u8]) -> BinaryFrame {
        let mut written = Vec::new();
        write_binary_frame(&mut written, kind, id, header, payload).unwrap();
        assert_eq!(written, encode_binary_frame(kind, id, header, payload).unwrap());
        assert_eq!(written.len(), BINARY_FRAME_PREFIX_LEN + header.len() + payload.len());
        let mut reader = Cursor::new(written);
        let frame = read_binary_frame(&mut reader).unwrap();
        assert_eq!(
            reader.position() as usize,
            reader.get_ref().len(),
            "the frame is consumed exactly"
        );
        frame
    }

    #[test]
    fn should_lay_out_the_prefix_little_endian() {
        let frame = encode_binary_frame(BinaryFrameKind::Response, 0x0102_0304_0506_0708, b"abcd", b"xyz").unwrap();
        assert_eq!(
            frame,
            [
                0xA2, 0x02, 0x00, 0x00, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x04, 0x00, 0x00, 0x00, 0x03,
                0x00, 0x00, 0x00, b'a', b'b', b'c', b'd', b'x', b'y', b'z'
            ]
        );
    }

    #[test]
    fn should_round_trip_raw_payload_bytes_without_inflation() {
        let payload: Vec<u8> = (0..=255).collect();
        let header = encode_call_header("engine", "echo").unwrap();
        let frame = round_trip(BinaryFrameKind::Call, 7, &header, &payload);
        assert_eq!(frame.kind, BinaryFrameKind::Call);
        assert_eq!(frame.id, 7);
        assert_eq!(decode_call_header(&frame.header).unwrap(), ("engine", "echo"));
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn should_round_trip_empty_header_and_payload() {
        let frame = round_trip(BinaryFrameKind::Response, u64::MAX, b"", b"");
        assert_eq!(frame.id, u64::MAX);
        assert!(frame.header.is_empty() && frame.payload.is_empty());
    }

    #[test]
    fn should_accept_a_payload_of_exactly_the_frame_limit() {
        let payload = vec![0xFF; MAX_FRAME_SIZE];
        let frame = round_trip(BinaryFrameKind::Response, 1, &encode_response_header(0), &payload);
        assert_eq!(frame.payload.len(), MAX_FRAME_SIZE);
    }

    #[test]
    fn should_reject_an_oversized_payload_or_header_on_write() {
        let payload = vec![0; MAX_FRAME_SIZE + 1];
        assert!(encode_binary_frame(BinaryFrameKind::Call, 1, b"", &payload).is_err());
        let mut sink = Vec::new();
        assert!(write_binary_frame(&mut sink, BinaryFrameKind::Call, 1, b"", &payload).is_err());
        assert!(sink.is_empty(), "nothing is written for a rejected frame");
        let header = vec![0; MAX_BINARY_HEADER_SIZE + 1];
        assert!(encode_binary_frame(BinaryFrameKind::Call, 1, &header, b"").is_err());
    }

    #[test]
    fn should_reject_an_oversized_length_on_read_before_allocating() {
        let mut frame = encode_binary_frame(BinaryFrameKind::Response, 1, b"", b"").unwrap();
        frame[16..20].copy_from_slice(&(MAX_FRAME_SIZE as u32 + 1).to_le_bytes());
        let error = read_binary_frame(&mut Cursor::new(frame)).unwrap_err().to_string();
        assert!(error.contains("payload length"), "{error}");
        let mut frame = encode_binary_frame(BinaryFrameKind::Response, 1, b"", b"").unwrap();
        frame[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = read_binary_frame(&mut Cursor::new(frame)).unwrap_err().to_string();
        assert!(error.contains("header length"), "{error}");
    }

    #[test]
    fn should_reject_truncated_frames() {
        let frame = encode_binary_frame(BinaryFrameKind::Response, 3, &encode_response_header(0), b"payload").unwrap();
        for cut in [
            0,
            1,
            BINARY_FRAME_PREFIX_LEN - 1,
            BINARY_FRAME_PREFIX_LEN + 2,
            frame.len() - 1,
        ] {
            assert!(
                read_binary_frame(&mut Cursor::new(&frame[..cut])).is_err(),
                "a frame cut at {cut} of {} bytes must not decode",
                frame.len()
            );
        }
    }

    #[test]
    fn should_reject_bad_magic_kind_flags_and_id() {
        let valid = encode_binary_frame(BinaryFrameKind::Response, 1, b"", b"").unwrap();
        for (offset, value, expected) in [
            (0, 0x7B, "magic"),
            (1, 9, "kind"),
            (2, 1, "flags"),
            (3, 1, "flags"),
            (4, 0, "id 0"),
        ] {
            let mut frame = valid.clone();
            frame[offset] = value;
            let error = read_binary_frame(&mut Cursor::new(frame)).unwrap_err().to_string();
            assert!(error.contains(expected), "offset {offset}: {error}");
        }
        assert!(encode_binary_frame(BinaryFrameKind::Call, 0, b"", b"").is_err());
    }

    #[test]
    fn should_validate_call_and_response_headers() {
        assert_eq!(
            decode_call_header(&encode_call_header("", "").unwrap()).unwrap(),
            ("", "")
        );
        assert!(decode_call_header(b"\x01").is_err());
        assert!(decode_call_header(b"\x05\x00abc").is_err());
        assert!(decode_call_header(b"\x01\x00\xff").is_err());
        assert!(encode_call_header(&"c".repeat(70_000), "m").is_err());
        assert_eq!(decode_response_header(&encode_response_header(-3)).unwrap(), -3);
        assert!(decode_response_header(b"\0\0\0").is_err());
        assert!(decode_response_header(b"\0\0\0\0\0").is_err());
    }
}
