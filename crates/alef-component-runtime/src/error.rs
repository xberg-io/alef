use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("component `{component_id}` has no artifact for target `{target}`")]
    ArtifactNotFound { component_id: String, target: String },
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
    #[error("failed to load component library `{path}`: {message}")]
    LibraryLoad { path: PathBuf, message: String },
    #[error("component library does not export `alef_component_entry_v1`: {0}")]
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
