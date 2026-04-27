use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Node,
    Ruby,
    Php,
    Elixir,
    Wasm,
    Ffi,
    Go,
    Java,
    Csharp,
    R,
    Rust,
    Kotlin,
    Swift,
    Dart,
    Gleam,
    Zig,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::Node => write!(f, "node"),
            Self::Ruby => write!(f, "ruby"),
            Self::Php => write!(f, "php"),
            Self::Elixir => write!(f, "elixir"),
            Self::Wasm => write!(f, "wasm"),
            Self::Ffi => write!(f, "ffi"),
            Self::Go => write!(f, "go"),
            Self::Java => write!(f, "java"),
            Self::Csharp => write!(f, "csharp"),
            Self::R => write!(f, "r"),
            Self::Rust => write!(f, "rust"),
            Self::Kotlin => write!(f, "kotlin"),
            Self::Swift => write!(f, "swift"),
            Self::Dart => write!(f, "dart"),
            Self::Gleam => write!(f, "gleam"),
            Self::Zig => write!(f, "zig"),
        }
    }
}

/// A parameter in an adapter function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterParam {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub optional: bool,
}

/// The kind of adapter pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterPattern {
    SyncFunction,
    AsyncMethod,
    CallbackBridge,
    Streaming,
    ServerLifecycle,
}

/// Configuration for a single adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    pub name: String,
    pub pattern: AdapterPattern,
    /// Full Rust path to the core function/method (e.g., "html_to_markdown_rs::convert")
    pub core_path: String,
    /// Parameters
    #[serde(default)]
    pub params: Vec<AdapterParam>,
    /// Return type name
    pub returns: Option<String>,
    /// Error type name
    pub error_type: Option<String>,
    /// For async_method/streaming: the owning type name
    pub owner_type: Option<String>,
    /// For streaming: the item type
    pub item_type: Option<String>,
    /// For Python: release GIL during call
    #[serde(default)]
    pub gil_release: bool,
    /// For callback_bridge: the Rust trait to implement (e.g., "SpikardHandler")
    #[serde(default)]
    pub trait_name: Option<String>,
    /// For callback_bridge: the trait method name (e.g., "handle")
    #[serde(default)]
    pub trait_method: Option<String>,
    /// For callback_bridge: whether to detect async callbacks at construction time
    #[serde(default)]
    pub detect_async: bool,
}
