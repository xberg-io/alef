use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("component `{component_id}` has no artifact for target `{target}`")]
    ArtifactNotFound { component_id: String, target: String },
    #[error(
        "component `{component_id}` is not locked; the embedded components.lock.json is still \
         the empty bootstrap lock `alef generate` writes before any artifact exists -- run \
         `alef component lock` to populate it"
    )]
    ComponentsNotLocked { component_id: String },
    #[error("component `{component}` is bundled for this target and cannot be downloaded")]
    BundledComponentNotLoadable { component: String },
    #[error("component `{component_id}` does not provide contract `{contract}`")]
    ContractNotProvided { component_id: String, contract: String },
    #[error("invalid SHA-256 digest `{0}`")]
    InvalidDigest(String),
    #[error("artifact digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("artifact size mismatch: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("artifact signature is required")]
    SignatureRequired,
    #[error("invalid Ed25519 signature encoding")]
    InvalidSignatureEncoding,
    #[error("artifact signature verification failed")]
    SignatureVerification,
    #[error("component manifest is not in canonical JSON form")]
    NonCanonicalManifest,
    #[error("component manifest digest does not match the lock")]
    ManifestDigestMismatch,
    #[error("component manifest identity does not match the lock")]
    ManifestIdentityMismatch,
    #[error("component manifest schema or ABI version is unsupported")]
    UnsupportedManifest,
    #[error("component signature uses unsupported algorithm `{0}`")]
    UnsupportedSignatureAlgorithm(String),
    #[error("component signature key `{0}` is not embedded in the lock")]
    UnknownSignatureKey(String),
    #[error("component signature key does not match the lock entry")]
    SignatureKeyMismatch,
    #[error("unsafe archive entry `{0}`")]
    UnsafeArchiveEntry(String),
    #[error("artifact library path `{0}` is not a safe relative path")]
    UnsafeLibraryPath(String),
    #[error("component library is missing at `{0}`")]
    MissingLibrary(PathBuf),
    #[error("failed to download `{url}`: {message}")]
    Download { url: String, message: String },
    #[error("component `{component}` is not cached and network access is disabled (would fetch `{url}`)")]
    Offline { component: String, url: String },
    #[error("failed to load component library `{path}`: {message}")]
    LibraryLoad { path: PathBuf, message: String },
    #[error("component library does not export the requested entry point: {0}")]
    MissingEntrypoint(String),
    #[error("component entrypoint failed with status {0}")]
    EntrypointFailed(i32),
    #[error("component returned an invalid descriptor: {0}")]
    InvalidDescriptor(&'static str),
    #[error(
        "component ABI {actual_major}.{actual_minor} is incompatible with host ABI {expected_major}.{expected_minor}"
    )]
    IncompatibleAbi {
        expected_major: u32,
        expected_minor: u32,
        actual_major: u32,
        actual_minor: u32,
    },
    #[error("component identity mismatch: expected `{expected}`, got `{actual}`")]
    IdentityMismatch { expected: String, actual: String },
    #[error("component contract hash does not match the requested contract")]
    ContractHashMismatch,
    #[error("component feature-set hash does not match the requested feature set")]
    FeatureSetHashMismatch,
    #[error("component lock contains duplicate entry `{component_id}` for target `{target}`")]
    DuplicateLockEntry { component_id: String, target: String },
    #[error("component lock schema version {0} is not supported")]
    UnsupportedLockSchema(u32),
    #[error("component lock public key is not a valid Ed25519 public key")]
    InvalidPublicKey,
    #[error("component contract table is too small or incorrectly aligned")]
    InvalidContractTable,
    #[error("component descriptor does not provide an instance factory")]
    MissingInstanceFactory,
    #[error("component descriptor does not provide an instance destructor")]
    MissingInstanceDestructor,
    #[error("component instance creation failed with status {status}: {message}")]
    InstanceCreation { status: i32, message: String },
    #[error("component instance factory returned a null handle")]
    NullInstance,
    #[error("component string is not valid UTF-8")]
    InvalidUtf8,
    #[error("component string exceeds the maximum supported length")]
    StringTooLong,
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl ComponentError {
    /// A stable, language-agnostic identifier for this error's kind, independent of its
    /// human-readable message and never changing across releases for the same variant.
    /// Generated bindings prefix the message with this (see
    /// `alef::backends::native_components`) so callers in any host language can distinguish,
    /// for example, an offline failure from an invalid signature from an unsupported host
    /// without parsing free-form text.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ArtifactNotFound { .. } => "artifact_not_found",
            Self::ComponentsNotLocked { .. } => "components_not_locked",
            Self::BundledComponentNotLoadable { .. } => "bundled_component_not_loadable",
            Self::ContractNotProvided { .. } => "contract_not_provided",
            Self::InvalidDigest(_) => "invalid_digest",
            Self::DigestMismatch { .. } => "digest_mismatch",
            Self::SizeMismatch { .. } => "size_mismatch",
            Self::SignatureRequired => "signature_required",
            Self::InvalidSignatureEncoding => "invalid_signature_encoding",
            Self::SignatureVerification => "signature_verification_failed",
            Self::NonCanonicalManifest => "non_canonical_manifest",
            Self::ManifestDigestMismatch => "manifest_digest_mismatch",
            Self::ManifestIdentityMismatch => "manifest_identity_mismatch",
            Self::UnsupportedManifest => "unsupported_manifest",
            Self::UnsupportedSignatureAlgorithm(_) => "unsupported_signature_algorithm",
            Self::UnknownSignatureKey(_) => "unknown_signature_key",
            Self::SignatureKeyMismatch => "signature_key_mismatch",
            Self::UnsafeArchiveEntry(_) => "unsafe_archive_entry",
            Self::UnsafeLibraryPath(_) => "unsafe_library_path",
            Self::MissingLibrary(_) => "missing_library",
            Self::Download { .. } => "download_failed",
            Self::Offline { .. } => "offline",
            Self::LibraryLoad { .. } => "library_load_failed",
            Self::MissingEntrypoint(_) => "missing_entrypoint",
            Self::EntrypointFailed(_) => "entrypoint_failed",
            Self::InvalidDescriptor(_) => "invalid_descriptor",
            Self::IncompatibleAbi { .. } => "incompatible_abi",
            Self::IdentityMismatch { .. } => "identity_mismatch",
            Self::ContractHashMismatch => "contract_hash_mismatch",
            Self::FeatureSetHashMismatch => "feature_set_hash_mismatch",
            Self::DuplicateLockEntry { .. } => "duplicate_lock_entry",
            Self::UnsupportedLockSchema(_) => "unsupported_lock_schema",
            Self::InvalidPublicKey => "invalid_public_key",
            Self::InvalidContractTable => "invalid_contract_table",
            Self::MissingInstanceFactory => "missing_instance_factory",
            Self::MissingInstanceDestructor => "missing_instance_destructor",
            Self::InstanceCreation { .. } => "instance_creation_failed",
            Self::NullInstance => "null_instance",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::StringTooLong => "string_too_long",
            Self::Json(_) => "json_error",
            Self::Io(_) => "io_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_stable_and_distinct_for_representative_variants() {
        assert_eq!(
            ComponentError::Offline {
                component: "demo".into(),
                url: "https://example.invalid".into(),
            }
            .code(),
            "offline"
        );
        assert_eq!(
            ComponentError::SignatureVerification.code(),
            "signature_verification_failed"
        );
        assert_eq!(
            ComponentError::BundledComponentNotLoadable {
                component: "demo".into()
            }
            .code(),
            "bundled_component_not_loadable"
        );
    }

    #[test]
    fn code_never_appears_inside_its_own_display_message_by_accident() {
        // Not a hard requirement, just documents that `code()` and `Display` are independent
        // surfaces: generated bindings prefix the message with the code themselves rather
        // than relying on it already being embedded.
        let error = ComponentError::SignatureVerification;
        assert!(!error.to_string().contains(error.code()));
    }
}
