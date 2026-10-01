use crate::core::ir::{ApiSurface, MethodDef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for generating trait bridge code that allows foreign language
/// objects to implement Rust traits via FFI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraitBridgeConfig {
    /// Name of the Rust trait to bridge (e.g., `"OcrBackend"`).
    pub trait_name: String,
    /// Super-trait that requires forwarding (e.g., `"Plugin"`).
    /// When set, the bridge generates an `impl SuperTrait for Wrapper` block.
    #[serde(default)]
    pub super_trait: Option<String>,
    /// Rust path to the registry getter function
    /// (e.g., `"sample_core::plugins::registry::get_ocr_backend_registry"`).
    /// Optional — when set, the generated registration function inserts the bridge into a registry.
    /// When omitted, supporting backends call the core host function named by `register_fn`
    /// directly, allowing the core to own validation, lifecycle, and locking.
    #[serde(default)]
    pub registry_getter: Option<String>,
    /// Name of the registration function to generate
    /// (e.g., `"register_ocr_backend"`).
    /// Optional — when set, a host-language registration function is generated. If
    /// `registry_getter` is absent, this is also the core host function name.
    /// When absent, only the wrapper struct and trait impl are emitted (per-call bridge pattern).
    #[serde(default)]
    pub register_fn: Option<String>,
    /// Name of the unregister function to generate
    /// (e.g., `"unregister_ocr_backend"`).
    /// Optional — when set, a host-language wrapper that removes a previously
    /// registered plugin from the registry is emitted alongside `register_fn`.
    /// The function takes the plugin name as a string.
    #[serde(default)]
    pub unregister_fn: Option<String>,
    /// Name of the clear function to generate
    /// (e.g., `"clear_ocr_backends"`).
    /// Optional — when set, a host-language wrapper that removes ALL registered
    /// plugins of this type is emitted alongside `register_fn`. The function
    /// takes no arguments and is typically used in test teardown.
    #[serde(default)]
    pub clear_fn: Option<String>,
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
    ///
    /// A target answers to its language name and, where they differ, to its backend's own
    /// name: `["python"]` and `["pyo3"]` are the same exclusion, as are `node`/`napi`,
    /// `ruby`/`magnus`, `elixir`/`rustler`, `r`/`extendr` and `c`/`ffi`. When a target is
    /// listed here, the bridge struct and everything that references it — the wrapper
    /// functions, the options setter, the registration surface — are omitted from that
    /// target's output.
    ///
    /// For PHP, use `"php:callbacks"` to retain the public interface/class and configured
    /// register, unregister, and clear symbols while disabling host callback execution.
    /// Registration then returns a deterministic unsupported error, while unregister and clear
    /// continue to call the Rust host. This preserves an existing PHP plugin API without
    /// retaining request-bound Zend values behind `Send + Sync` Rust trait objects. No other
    /// language supports this suffix. It uses this existing string list so adding the mode does
    /// not break Rust callers that construct `TraitBridgeConfig` exhaustively.
    ///
    /// An entry whose language/backend portion names neither a language nor a backend is rejected
    /// at config resolution rather than silently excluding nothing.
    #[serde(default)]
    pub exclude_languages: Vec<String>,
    /// Free functions that should NOT gain this bridge's extra argument.
    ///
    /// Every generated function that takes the bridge's owning type picks up the bridge
    /// keyword by default. List a function's Rust name here to leave it alone — a synchronous
    /// diagnostic entry point, say, where a listener has no meaning. Mirrors the
    /// `exclude_functions` key each `[crates.<lang>]` table already accepts, and is honoured
    /// identically by the Rust wrapper, the host-language facade and the type stub.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Methods that the FFI backend should NOT forward through the vtable.
    /// These methods fall back to the trait's default implementation.
    /// Useful for methods whose signatures involve trait-object references
    /// (`&dyn Trait`) that can't traverse the C FFI boundary.
    #[serde(default)]
    pub ffi_skip_methods: Vec<String>,
    /// How the bridge attaches to the public API.
    ///
    /// - `"function_param"` (default): the bridge object arrives as a function argument
    ///   at the position of any `param_name`-matching parameter. This is the legacy mode.
    /// - `"options_field"`: the bridge object lives as a field on a configured options
    ///   struct that itself arrives as a function argument. Backends emit a host-language
    ///   field on that struct instead of a separate function parameter; the bridge object
    ///   is attached to `options.<field>` before the underlying core call.
    ///
    /// In both modes the host value must be an **object that provides the trait's methods by
    /// name**. A bare callable (a Python `lambda`, a JS arrow function, a Ruby `proc`) is not
    /// accepted: the bridge dispatches `obj.<method>(...)`, never `obj(...)`. Passing one is
    /// rejected with an error naming the methods the object has to define — a method with a
    /// Rust default would otherwise be skipped silently and the listener would simply never
    /// fire.
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
    /// IR type name of the trait's context associated type (e.g., `"NodeContext"`).
    ///
    /// When set, backends skip generic record/enum codegen for this type and instead
    /// emit the richer visitor-specific version. Replaces the former literal
    /// `"NodeContext"` string comparisons scattered across backends.
    #[serde(default)]
    pub context_type: Option<String>,
    /// IR type name of the trait's result associated type (e.g., `"VisitResult"`).
    ///
    /// When set, backends skip generic record/enum codegen for this type and instead
    /// emit the richer visitor-specific version. Replaces the former literal
    /// `"VisitResult"` string comparisons scattered across backends.
    #[serde(default)]
    pub result_type: Option<String>,
}

/// How a trait bridge attaches to the public API.
///
/// See [`TraitBridgeConfig::bind_via`] for the user-facing description.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BridgeBinding {
    /// The bridge arrives as a positional function argument. Legacy default.
    #[default]
    FunctionParam,
    /// The bridge lives as a field on a configured options struct.
    OptionsField,
}

/// Backend spellings `exclude_languages` accepts in addition to every
/// [`Language`](crate::core::config::Language) name.
///
/// A target that answers to two names — the language and the backend crate that emits it —
/// honours either, so `exclude_languages = ["node"]` and `["napi"]` mean the same thing. Kept
/// in lock-step with the backends' own `TARGET_SPELLINGS` constants by
/// `tests/trait_bridge_exclude_language_spellings.rs`, which scans them out of the source
/// rather than trusting this list. ~keep
pub const BRIDGE_BACKEND_SPELLINGS: [&str; 5] = ["extendr", "magnus", "napi", "pyo3", "rustler"];

const UNSUPPORTED_CALLBACK_SUFFIX: &str = ":callbacks";

/// Whether `name` is a spelling any backend answers to in `exclude_languages`.
///
/// An unrecognised entry disables nothing at all, and the symptom surfaces far away as a
/// generated crate that does not compile — so it is rejected at config-resolution time
/// instead (alef #476).
pub fn is_known_bridge_language(name: &str) -> bool {
    if let Some(language) = name.strip_suffix(UNSUPPORTED_CALLBACK_SUFFIX) {
        return language == "php";
    }
    crate::core::config::Language::ALL
        .iter()
        .any(|language| language.to_string() == name)
        || BRIDGE_BACKEND_SPELLINGS.contains(&name)
}

impl TraitBridgeConfig {
    /// Whether this bridge should be generated for `language` (a `Backend::name()` /
    /// [`crate::core::config::Language`] display string, e.g. `"java"`, `"csharp"`, `"go"`).
    ///
    /// Centralizes the `exclude_languages` membership check so visitor-file generators
    /// across backends (and any added later) apply the same rule instead of each
    /// re-deriving it inline.
    pub fn is_active_for(&self, language: &str) -> bool {
        !self.exclude_languages.iter().any(|excluded| excluded == language)
    }

    /// Whether PHP keeps the bridge's public lifecycle surface but rejects callback registration
    /// without retaining the Zend object.
    pub fn php_callbacks_unsupported(&self) -> bool {
        self.exclude_languages.iter().any(|entry| entry == "php:callbacks")
    }

    pub(crate) fn php_safety_error(&self) -> String {
        format!(
            "PHP trait bridge `{}` is disabled: generated wrappers cannot safely retain \
             request-bound Zend values behind Rust Send + Sync trait objects. Add `php` to this \
             bridge's `exclude_languages` as `php:callbacks` to preserve its public lifecycle \
             surface with deterministic unsupported registration, add plain `php` to omit the \
             surface, or remove PHP from the crate's languages.",
            self.trait_name
        )
    }

    /// Resolve the field name on `options_type` that holds this bridge.
    ///
    /// Falls back to [`Self::param_name`] when [`Self::options_field`] is unset, matching
    /// the convention that the field name and parameter name are the same in most cases.
    /// Returns `None` if neither is set.
    pub fn resolved_options_field(&self) -> Option<&str> {
        self.options_field.as_deref().or(self.param_name.as_deref())
    }

    /// Return the names of associated types declared in `context_type` and `result_type`.
    ///
    /// Backends use this list to skip generic record/enum codegen for these types,
    /// deferring to visitor-specific generators instead.
    pub fn associated_type_names(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if let Some(s) = self.context_type.as_deref() {
            names.push(s);
        }
        if let Some(s) = self.result_type.as_deref() {
            names.push(s);
        }
        names
    }

    /// Resolve the trait's methods from the API surface.
    ///
    /// Searches `api.types` for a type matching this bridge's `trait_name`,
    /// then returns the methods from that type. Returns an empty vector if
    /// the trait is not found (e.g., excluded from public API).
    pub fn resolve_methods<'a>(&self, api: &'a ApiSurface) -> Vec<&'a MethodDef> {
        api.types
            .iter()
            .find(|t| t.name == self.trait_name)
            .map(|t| t.methods.iter().collect())
            .unwrap_or_default()
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
    fn parses_unregister_and_clear_fns() {
        let toml_src = r#"
trait_name = "OcrBackend"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.unregister_fn.as_deref(), Some("unregister_ocr_backend"));
        assert_eq!(cfg.clear_fn.as_deref(), Some("clear_ocr_backends"));
    }

    #[test]
    fn unregister_and_clear_default_to_none() {
        let toml_src = r#"
trait_name = "OcrBackend"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert!(cfg.unregister_fn.is_none());
        assert!(cfg.clear_fn.is_none());
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

    #[test]
    fn associated_type_names_returns_configured_names() {
        let toml_src = r#"
trait_name = "HtmlVisitor"
context_type = "NodeContext"
result_type = "VisitResult"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.context_type.as_deref(), Some("NodeContext"));
        assert_eq!(cfg.result_type.as_deref(), Some("VisitResult"));
        let names = cfg.associated_type_names();
        assert_eq!(names, vec!["NodeContext", "VisitResult"]);
    }

    #[test]
    fn associated_type_names_empty_when_not_set() {
        let toml_src = r#"
trait_name = "OcrBackend"
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert!(cfg.associated_type_names().is_empty());
    }

    #[test]
    fn exclude_functions_parses_and_defaults_to_empty() {
        let cfg: TraitBridgeConfig = toml::from_str(&sample_toml("options_field")).unwrap();
        assert!(cfg.exclude_functions.is_empty());

        let with_exclusions: TraitBridgeConfig = toml::from_str(
            r#"
trait_name = "HtmlVisitor"
bind_via = "options_field"
options_type = "ConversionOptions"
param_name = "visitor"
exclude_functions = ["doctor", "version"]
"#,
        )
        .unwrap();
        assert_eq!(with_exclusions.exclude_functions, vec!["doctor", "version"]);
    }

    #[test]
    fn every_language_name_and_backend_spelling_is_a_known_exclude_target() {
        for language in crate::core::config::Language::ALL {
            let name = language.to_string();
            assert!(
                is_known_bridge_language(&name),
                "`{name}` is a Language variant, so `exclude_languages` must accept it"
            );
        }
        for spelling in BRIDGE_BACKEND_SPELLINGS {
            assert!(is_known_bridge_language(spelling));
        }
    }

    #[test]
    fn a_misspelled_exclude_target_is_not_known() {
        for typo in ["nodejs", "c#", "Python", "kotlin-android", ""] {
            assert!(
                !is_known_bridge_language(typo),
                "`{typo}` must not pass as a known exclude target"
            );
        }
    }

    #[test]
    fn is_active_for_true_when_exclude_languages_empty() {
        let cfg = TraitBridgeConfig::default();
        assert!(cfg.is_active_for("go"));
        assert!(cfg.is_active_for("csharp"));
        assert!(cfg.is_active_for("java"));
    }

    #[test]
    fn is_active_for_false_when_language_listed() {
        let toml_src = r#"
trait_name = "HtmlVisitor"
exclude_languages = ["go", "csharp"]
"#;
        let cfg: TraitBridgeConfig = toml::from_str(toml_src).unwrap();
        assert!(!cfg.is_active_for("go"));
        assert!(!cfg.is_active_for("csharp"));
        assert!(cfg.is_active_for("java"));
    }

    #[test]
    fn callback_suffix_keeps_surface_active_and_disables_only_callbacks() {
        let cfg: TraitBridgeConfig = toml::from_str(
            r#"
trait_name = "OcrBackend"
exclude_languages = ["php:callbacks"]
"#,
        )
        .unwrap();

        assert!(cfg.is_active_for("php"));
        assert!(cfg.php_callbacks_unsupported());
        assert!(is_known_bridge_language("php:callbacks"));
        assert!(!is_known_bridge_language("python:callbacks"));
    }
}
