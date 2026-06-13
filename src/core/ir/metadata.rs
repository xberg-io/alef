use serde::{Deserialize, Serialize};

/// Indicates the core Rust type wraps the resolved type in a smart pointer or cow.
/// Used by codegen to generate correct From/Into conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CoreWrapper {
    #[default]
    None,
    /// `Cow<'static, str>` — binding uses String, core needs `.into()`
    Cow,
    /// `Arc<T>` — binding unwraps, core wraps with `Arc::new()`
    Arc,
    /// `bytes::Bytes` — binding uses `Vec<u8>`, core needs `Bytes::from()`
    Bytes,
    /// `Arc<Mutex<T>>` — binding wraps with `Arc::new(Mutex::new())`, methods call `.lock()`
    ArcMutex,
    /// `Box<str>` — binding uses String, core needs `.into()` (same shape as Cow
    /// but distinct so backends can keep wrapper-specific behavior addressable).
    Box,
}

/// Typed default value for a field, enabling backends to emit language-native defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DefaultValue {
    BoolLiteral(bool),
    StringLiteral(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    EnumVariant(String),
    /// Empty collection or Default::default()
    Empty,
    /// None / null
    None,
}

/// Deprecation metadata extracted from `#[deprecated(...)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeprecationInfo {
    /// Version when the item was deprecated (from `#[deprecated(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation note (from `#[deprecated(note = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Version annotation on an IR item.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionAnnotation {
    /// Version when this item was introduced (from `#[alef(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation info (from `#[deprecated(...)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationInfo>,
}
