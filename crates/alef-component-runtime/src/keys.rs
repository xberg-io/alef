//! Ed25519 public key decoding shared by every layer that trusts a component signature:
//! this crate's own artifact cache, `alef`'s config validation, and `alef`'s artifact
//! signing/verification CLI path. All three used to decode independently and disagreed on
//! accepted base64 padding, so a key config validation accepted could fail every load. This
//! is the one decoder all of them call now.

use base64::Engine as _;
use ed25519_dalek::pkcs8::{DecodePublicKey as _, EncodePublicKey as _};
use ed25519_dalek::VerifyingKey;

use crate::ComponentError;

/// Decode an Ed25519 public key from PEM, or from base64-encoded (standard or unpadded)
/// raw 32-byte / DER `SubjectPublicKeyInfo` bytes.
///
/// Standard and unpadded base64 are both accepted because config authors, signing
/// tooling, and hand-copied keys are not consistent about padding, and a key that
/// validates under one variant but not the other is a config that silently breaks at
/// load time.
pub fn decode_public_key(value: &str) -> Result<VerifyingKey, ComponentError> {
    if value.trim_start().starts_with("-----BEGIN") {
        return VerifyingKey::from_public_key_pem(value).map_err(|_| ComponentError::InvalidPublicKey);
    }
    let trimmed = value.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .map_err(|_| ComponentError::InvalidPublicKey)?;
    if let Ok(raw) = <[u8; 32]>::try_from(bytes.as_slice()) {
        return VerifyingKey::from_bytes(&raw).map_err(|_| ComponentError::InvalidPublicKey);
    }
    VerifyingKey::from_public_key_der(&bytes).map_err(|_| ComponentError::InvalidPublicKey)
}

/// Re-encode a decoded key as DER `SubjectPublicKeyInfo` bytes.
///
/// Callers that hand a public key to an external tool (`alef`'s artifact verification
/// shells out to `openssl pkeyutl`) need bytes in a single, unambiguous format regardless
/// of which form [`decode_public_key`] accepted it in.
pub fn encode_public_key_der(key: &VerifyingKey) -> Result<Vec<u8>, ComponentError> {
    key.to_public_key_der()
        .map(|document| document.as_bytes().to_vec())
        .map_err(|_| ComponentError::InvalidPublicKey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// A fixture key independent of this module's own PEM/DER encoding path, generated with
    /// `openssl genpkey -algorithm ED25519` / `openssl pkey -pubout`, so the PEM test does
    /// not validate `decode_public_key` against bytes this same module produced.
    const FIXTURE_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA68ACmVmPwMQ4mvLWXAuT7IezNeigy/lauMP08bFC6lU=\n-----END PUBLIC KEY-----\n";
    const FIXTURE_RAW: [u8; 32] = [
        0xeb, 0xc0, 0x02, 0x99, 0x59, 0x8f, 0xc0, 0xc4, 0x38, 0x9a, 0xf2, 0xd6, 0x5c, 0x0b, 0x93, 0xec, 0x87, 0xb3,
        0x35, 0xe8, 0xa0, 0xcb, 0xf9, 0x5a, 0xb8, 0xc3, 0xf4, 0xf1, 0xb1, 0x42, 0xea, 0x55,
    ];

    fn key() -> VerifyingKey {
        SigningKey::from_bytes(&[9; 32]).verifying_key()
    }

    #[test]
    fn decodes_standard_padded_base64() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(key().to_bytes());
        assert_eq!(decode_public_key(&encoded).unwrap(), key());
    }

    #[test]
    fn decodes_unpadded_base64() {
        let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(key().to_bytes());
        assert_eq!(decode_public_key(&encoded).unwrap(), key());
    }

    #[test]
    fn decodes_pem() {
        let expected = VerifyingKey::from_bytes(&FIXTURE_RAW).unwrap();
        assert_eq!(decode_public_key(FIXTURE_PEM).unwrap(), expected);
    }

    #[test]
    fn encodes_der_that_round_trips_through_decode() {
        let der = encode_public_key_der(&key()).unwrap();
        let decoded = VerifyingKey::from_public_key_der(&der).unwrap();
        assert_eq!(decoded, key());
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode_public_key("not-base64"), Err(ComponentError::InvalidPublicKey)));
    }
}
