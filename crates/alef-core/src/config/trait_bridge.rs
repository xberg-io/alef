use serde::{Deserialize, Serialize};

/// Configuration for generating trait bridge code that allows foreign language
/// objects to implement Rust traits via FFI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitBridgeConfig {
    /// Name of the Rust trait to bridge (e.g., `"OcrBackend"`).
    pub trait_name: String,
    /// Super-trait that requires forwarding (e.g., `"Plugin"`).
    /// When set, the bridge generates an `impl SuperTrait for Wrapper` block.
    #[serde(default)]
    pub super_trait: Option<String>,
    /// Rust path to the registry getter function
    /// (e.g., `"kreuzberg::plugins::registry::get_ocr_backend_registry"`).
    pub registry_getter: String,
    /// Name of the registration function to generate
    /// (e.g., `"register_ocr_backend"`).
    pub register_fn: String,
}
