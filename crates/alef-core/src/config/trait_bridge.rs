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
    /// Optional — when set, the generated registration function inserts the bridge into a registry.
    #[serde(default)]
    pub registry_getter: Option<String>,
    /// Name of the registration function to generate
    /// (e.g., `"register_ocr_backend"`).
    /// Optional — when set, a `#[pyfunction]` registration function is generated.
    /// When absent, only the wrapper struct and trait impl are emitted (per-call bridge pattern).
    #[serde(default)]
    pub register_fn: Option<String>,
    /// Named type alias in the IR that maps to this bridge (e.g., `"VisitorHandle"`).
    ///
    /// When a function parameter has a `TypeRef::Named` matching this alias, code
    /// generators replace the parameter type with the language-native callback object
    /// (e.g., `Py<PyAny>` for Python) and emit wrapping code to construct the bridge.
    #[serde(default)]
    pub type_alias: Option<String>,
}
