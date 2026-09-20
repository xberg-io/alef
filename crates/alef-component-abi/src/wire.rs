//! Compact, self-describing binary encoding for component wire values.
//!
//! Scalars, `Utf8`, and `Bytes` cross the component boundary directly as
//! `#[repr(C)]` values (see the crate root). Compound wire types --
//! records, `Option<T>`, `Vec<T>`, and fieldless enums -- are instead encoded
//! into a flat byte buffer carried in an [`crate::AlefSlice`] (input,
//! borrowed for the call) or an [`crate::AlefOwnedBuffer`] (output, owned by
//! the caller), so the `#[repr(C)]` function-table shape never has to grow a
//! new struct per compound type. Generated producer and proxy code both call
//! into this module to write and read that buffer identically; see
//! `alef::codegen::component`'s `## Wire format` documentation for the exact
//! layout each `WireType` variant uses.
use alloc::vec::Vec;

/// An error produced while decoding a wire-format buffer.
///
/// A generated decoder maps this to `AlefStatus::INVALID_ARGUMENT` (input) or
/// a component-panicked-style error buffer (output); the buffer contents are
/// never trusted enough to `panic!` on a malformed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The buffer ended before the value it describes was fully read.
    UnexpectedEnd,
    /// A `Utf8`-typed value's bytes were not valid UTF-8.
    InvalidUtf8,
    /// An `Option<T>` presence byte was neither `0` nor `1`.
    InvalidPresenceTag(u8),
    /// An enum's `u32`/`i32` discriminant did not match any known variant.
    InvalidVariant(u32),
    /// The buffer had bytes left over after decoding the value it describes.
    TrailingBytes,
}

/// Appends values to a growable buffer using the compact wire encoding.
#[derive(Debug, Default)]
pub struct WireWriter {
    buffer: Vec<u8>,
}

impl WireWriter {
    #[must_use]
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Consumes the writer, returning the encoded bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    pub fn write_bool(&mut self, value: bool) {
        self.buffer.push(u8::from(value));
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    pub fn write_i8(&mut self, value: i8) {
        self.buffer.push(value.to_le_bytes()[0]);
    }

    pub fn write_u16(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i16(&mut self, value: i16) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u32(&mut self, value: u32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i32(&mut self, value: i32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i64(&mut self, value: i64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_f32(&mut self, value: f32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_f64(&mut self, value: f64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a `u32` byte count followed by `value` itself.
    pub fn write_bytes(&mut self, value: &[u8]) {
        self.write_u32(u32::try_from(value.len()).unwrap_or(u32::MAX));
        self.buffer.extend_from_slice(value);
    }

    /// Writes a UTF-8 string as its length-prefixed byte form.
    pub fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    /// Writes a one-byte presence flag, followed by the value itself when present.
    pub fn write_option<T>(&mut self, value: Option<&T>, write_value: impl FnOnce(&mut Self, &T)) {
        match value {
            Some(inner) => {
                self.write_u8(1);
                write_value(self, inner);
            }
            None => self.write_u8(0),
        }
    }

    /// Writes a `u32` element count, followed by each element in order.
    pub fn write_slice<T>(&mut self, values: &[T], mut write_value: impl FnMut(&mut Self, &T)) {
        self.write_u32(u32::try_from(values.len()).unwrap_or(u32::MAX));
        for value in values {
            write_value(self, value);
        }
    }
}

/// Reads values out of a borrowed byte buffer using the compact wire encoding.
pub struct WireReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> WireReader<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self.position.checked_add(len).ok_or(WireError::UnexpectedEnd)?;
        let slice = self.bytes.get(self.position..end).ok_or(WireError::UnexpectedEnd)?;
        self.position = end;
        Ok(slice)
    }

    pub fn read_bool(&mut self) -> Result<bool, WireError> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8, WireError> {
        Ok(self.take(1)?[0] as i8)
    }

    pub fn read_u16(&mut self) -> Result<u16, WireError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("length checked by take");
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn read_i16(&mut self) -> Result<i16, WireError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("length checked by take");
        Ok(i16::from_le_bytes(bytes))
    }

    pub fn read_u32(&mut self) -> Result<u32, WireError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("length checked by take");
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_i32(&mut self) -> Result<i32, WireError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("length checked by take");
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn read_u64(&mut self) -> Result<u64, WireError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("length checked by take");
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn read_i64(&mut self) -> Result<i64, WireError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("length checked by take");
        Ok(i64::from_le_bytes(bytes))
    }

    pub fn read_f32(&mut self) -> Result<f32, WireError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("length checked by take");
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn read_f64(&mut self) -> Result<f64, WireError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("length checked by take");
        Ok(f64::from_le_bytes(bytes))
    }

    /// Reads a `u32` byte count followed by that many bytes.
    pub fn read_bytes(&mut self) -> Result<&'a [u8], WireError> {
        let len = self.read_u32()? as usize;
        self.take(len)
    }

    /// Reads a length-prefixed UTF-8 string.
    pub fn read_str(&mut self) -> Result<&'a str, WireError> {
        core::str::from_utf8(self.read_bytes()?).map_err(|_| WireError::InvalidUtf8)
    }

    /// Reads a one-byte presence flag, then the value itself when present.
    pub fn read_option<T>(
        &mut self,
        read_value: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        match self.read_u8()? {
            0 => Ok(None),
            1 => Ok(Some(read_value(self)?)),
            other => Err(WireError::InvalidPresenceTag(other)),
        }
    }

    /// Reads a `u32` element count, then that many elements in order.
    pub fn read_slice<T>(
        &mut self,
        mut read_value: impl FnMut(&mut Self) -> Result<T, WireError>,
    ) -> Result<Vec<T>, WireError> {
        let len = self.read_u32()? as usize;
        let mut values = Vec::with_capacity(len.min(4096));
        for _ in 0..len {
            values.push(read_value(self)?);
        }
        Ok(values)
    }

    /// Confirms every byte in the buffer was consumed by the preceding reads.
    ///
    /// Generated decoders call this once, after decoding a call's top-level
    /// value, to reject a truncated-looking but otherwise well-formed buffer
    /// (e.g. one record's bytes concatenated with a second, unrelated value).
    pub fn finish(self) -> Result<(), WireError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn round_trips_every_scalar_kind() {
        let mut writer = WireWriter::new();
        writer.write_bool(true);
        writer.write_u8(1);
        writer.write_i8(-1);
        writer.write_u16(2);
        writer.write_i16(-2);
        writer.write_u32(3);
        writer.write_i32(-3);
        writer.write_u64(4);
        writer.write_i64(-4);
        writer.write_f32(1.5);
        writer.write_f64(-2.5);
        writer.write_str("hi");
        writer.write_bytes(&[9, 8, 7]);
        let bytes = writer.into_bytes();

        let mut reader = WireReader::new(&bytes);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.read_u8().unwrap(), 1);
        assert_eq!(reader.read_i8().unwrap(), -1);
        assert_eq!(reader.read_u16().unwrap(), 2);
        assert_eq!(reader.read_i16().unwrap(), -2);
        assert_eq!(reader.read_u32().unwrap(), 3);
        assert_eq!(reader.read_i32().unwrap(), -3);
        assert_eq!(reader.read_u64().unwrap(), 4);
        assert_eq!(reader.read_i64().unwrap(), -4);
        assert_eq!(reader.read_f32().unwrap(), 1.5);
        assert_eq!(reader.read_f64().unwrap(), -2.5);
        assert_eq!(reader.read_str().unwrap(), "hi");
        assert_eq!(reader.read_bytes().unwrap(), &[9, 8, 7]);
        reader.finish().unwrap();
    }

    #[test]
    fn round_trips_option_and_slice() {
        let mut writer = WireWriter::new();
        writer.write_option(Some(&7u32), |writer, value| writer.write_u32(*value));
        writer.write_option(None::<&u32>, |writer, value| writer.write_u32(*value));
        writer.write_slice(&[1u32, 2, 3], |writer, value| writer.write_u32(*value));
        let bytes = writer.into_bytes();

        let mut reader = WireReader::new(&bytes);
        assert_eq!(reader.read_option(WireReader::read_u32).unwrap(), Some(7));
        assert_eq!(reader.read_option(WireReader::read_u32).unwrap(), None);
        assert_eq!(reader.read_slice(WireReader::read_u32).unwrap(), vec![1, 2, 3]);
        reader.finish().unwrap();
    }

    #[test]
    fn rejects_an_invalid_presence_tag() {
        let mut reader = WireReader::new(&[2]);
        assert_eq!(
            reader.read_option(WireReader::read_u8),
            Err(WireError::InvalidPresenceTag(2))
        );
    }

    #[test]
    fn rejects_a_truncated_buffer() {
        let mut reader = WireReader::new(&[0, 0]);
        assert_eq!(reader.read_u32(), Err(WireError::UnexpectedEnd));
    }

    #[test]
    fn rejects_trailing_bytes_after_a_complete_value() {
        let mut writer = WireWriter::new();
        writer.write_u32(1);
        let mut bytes = writer.into_bytes();
        bytes.push(0xff);
        let mut reader = WireReader::new(&bytes);
        let _ = reader.read_u32().unwrap();
        assert_eq!(reader.finish(), Err(WireError::TrailingBytes));
    }
}
