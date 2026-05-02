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
    /// Parameter name override — when the extractor sanitizes the type (e.g., `VisitorHandle`
    /// becomes `String` because it is a type alias over `Rc<RefCell<dyn Trait>>`), use the
    /// parameter name instead of the IR type to detect which parameter to bridge.
    ///
    /// For example, `param_name = "visitor"` ensures that a sanitized `visitor: Option<String>`
    /// parameter is still treated as a bridge param for this trait.
    #[serde(default)]
    pub param_name: Option<String>,
    /// Extra arguments to append to the `registry.register(arc, ...)` call.
    /// Example: `"0"` produces `registry.register(arc, 0)`.
    #[serde(default)]
    pub register_extra_args: Option<String>,
    /// Language backends that should NOT generate this trait bridge.
    /// Use backend names as they appear in `Backend::name()`, e.g. `["elixir", "wasm"]`.
    /// When a backend's name is listed here, the bridge struct and all related code are
    /// omitted from that backend's output.
    #[serde(default)]
    pub exclude_languages: Vec<String>,
    /// How the bridge attaches to the public API.
    ///
    /// - `"function_param"` (default): the bridge object arrives as a function argument
    ///   at the position of any `param_name`-matching parameter. This is the legacy mode.
    /// - `"options_field"`: the bridge object lives as a field on a configured options
    ///   struct that itself arrives as a function argument. Backends emit a host-language
    ///   field on that struct instead of a separate function parameter; the bridge object
    ///   is attached to `options.<field>` before the underlying core call.
    #[serde(default)]
    pub bind_via: BridgeBinding,
    /// IR type name that owns the bridge field when `bind_via = "options_field"` (e.g.,
    /// `"ConversionOptions"`). Required in that mode; ignored otherwise.
    #[serde(default)]
    pub options_type: Option<String>,
    /// Field name on `options_type` that holds the bridge handle when
    /// `bind_via = "options_field"` (e.g., `"visitor"`). When omitted, defaults to
    /// `param_name`. Ignored when `bind_via = "function_param"`.
    #[serde(default)]
    pub options_field: Option<String>,
}

/// How a trait bridge attaches to the public API.
///
/// See [`TraitBridgeConfig::bind_via`] for the user-facing description.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeBinding {
    /// The bridge arrives as a positional function argument. Legacy default.
    #[default]
    FunctionParam,
    /// The bridge lives as a field on a configured options struct.
    OptionsField,
}

impl TraitBridgeConfig {
    /// Resolve the field name on `options_type` that holds this bridge.
    ///
    /// Falls back to [`Self::param_name`] when [`Self::options_field`] is unset, matching
    /// the convention that the field name and parameter name are the same in most cases.
    /// Returns `None` if neither is set.
    pub fn resolved_options_field(&self) -> Option<&str> {
        self.options_field.as_deref().or(self.param_name.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_toml(bind_via: &str) -> String {
        format!(
            r#"
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
bind_via = "{bind_via}"
options_type = "ConversionOptions"
"#
        )
    }

    #[test]
    fn parses_options_field_binding() {
        let cfg: TraitBridgeConfig = toml::from_str(&sample_toml("options_field")).unwrap();
        assert_eq!(cfg.bind_via, BridgeBinding::OptionsField);
        assert_eq!(cfg.options_type.as_deref(), Some("ConversionOptions"));
        assert_eq!(cfg.resolved_options_field(), Some("visitor"));
    }

    #[test]
    fn defaults_to_function_param_when_omitted() {
        let toml_src = r#"
trait_name = "OcrBackend"
type_alias = "BackendHandle"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.bind_via, BridgeBinding::FunctionParam);
        assert!(cfg.options_type.is_none());
    }

    #[test]
    fn options_field_falls_back_to_param_name() {
        let toml_src = r#"
trait_name = "HtmlVisitor"
param_name = "visitor"
bind_via = "options_field"
options_type = "ConversionOptions"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.resolved_options_field(), Some("visitor"));
    }

    #[test]
    fn explicit_options_field_overrides_param_name() {
        let toml_src = r#"
trait_name = "HtmlVisitor"
param_name = "visitor"
bind_via = "options_field"
options_type = "ConversionOptions"
options_field = "callback"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.resolved_options_field(), Some("callback"));
    }
}
