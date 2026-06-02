//! Per-crate service and handler-contract configuration.
//!
//! A **service** is an owner/builder type that:
//! 1. Has a constructor (`new` or similar).
//! 2. Exposes chaining configurator methods (no callback).
//! 3. Exposes registration methods that each accept a host-language callback
//!    and optional metadata parameters.
//! 4. Has one or more entrypoints — a long-running async `run`, and/or a
//!    consuming `finalize` transform (e.g. `into_router`).
//!
//! A **handler contract** is the async Rust trait that every registered
//! callback must satisfy.  It is extracted from the existing trait surface and
//! augmented with service-specific metadata (wire DTOs, dispatch method name).
//!
//! Both fields use `#[serde(default)]` so consumers that omit them entirely
//! get unchanged extraction/codegen behaviour.

use serde::{Deserialize, Serialize};

/// Per-registration configuration entry inside a `[[crates.services]]` table.
///
/// Example in `alef.toml`:
///
/// ```toml
/// [[crates.services.registrations]]
/// method = "add_route"
/// callback_param = "handler"
/// callback_bound = "IntoHandler"
/// callback_contract = "Handler"
///
/// [[crates.services.registrations.variants]]
/// name = "get"
/// fixed = { method = "GET" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationSpec {
    /// Name of the method on the owner type (e.g. `"add_route"`).
    pub method: String,
    /// Name of the parameter that carries the callback (e.g. `"handler"`).
    pub callback_param: String,
    /// The generic type bound that the callback parameter uses
    /// (e.g. `"IntoHandler"`). Used to recognise which generic parameter to
    /// skip during the usual generic-method skip so the method is extracted.
    pub callback_bound: String,
    /// Name of the [`HandlerContractConfig`] (and trait) this callback maps to
    /// (e.g. `"Handler"`).
    pub callback_contract: String,
    /// Named shortcuts over this registration with pinned parameter values.
    /// Each variant emits as an additional method on the service owner whose
    /// signature drops the pinned params and whose body forwards to this base
    /// registration with the pinned values substituted in.
    #[serde(default)]
    pub variants: Vec<RegistrationVariantSpec>,
}

/// A named shortcut over a base [`RegistrationSpec`] with one or more pinned
/// parameter values.
///
/// The variant's emitted method takes the **non-pinned** subset of the base's
/// metadata params and forwards them, along with the handler, to the base
/// registration with the pinned values substituted in. For library-supplied
/// enum overrides, the pinned value is the variant *name* (e.g. `"GET"`); the
/// extractor resolves it against the param type's [`EnumDef`] variants. For
/// non-enum types, the pinned value is a verbatim expression in the host
/// language's Rust bridge.
///
/// ```toml
/// [[crates.services.registrations.variants]]
/// name = "get"
/// fixed = { method = "GET" }
/// style = "verb_decorator"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationVariantSpec {
    /// Shortcut name (e.g. `"get"`). Used as the variant method's name on the
    /// owner, transformed to each language's idiomatic casing by the backend
    /// templates.
    pub name: String,
    /// Map of base-registration metadata-param name → pinned value expression.
    /// For enum-typed params, the value is the enum variant name. For other
    /// types, the value is a verbatim expression substituted in the wrapper
    /// constructor call.
    #[serde(default)]
    pub fixed: std::collections::BTreeMap<String, String>,
    /// Optional documentation for the variant. When absent, backends emit a
    /// generic docstring referencing the base registration.
    #[serde(default)]
    pub doc: Option<String>,
    /// How backends should expose this variant's host-language surface.
    ///
    /// Valid values (case-insensitive): `"builder"`, `"verb_decorator"`, `"hybrid"`.
    /// When absent, defaults to `"hybrid"` (both forms emitted), preserving
    /// backward compatibility for consumers that have not yet migrated.
    ///
    /// - `"builder"` — emit only the decorator-factory form (`app.get(path)` returns a callable).
    /// - `"verb_decorator"` — emit only the direct method form (`app.get(path, handler)`).
    /// - `"hybrid"` — emit both forms (default).
    #[serde(default)]
    pub style: Option<String>,
}

/// Per-entrypoint configuration inside a `[[crates.services]]` table.
///
/// ```toml
/// [[crates.services.entrypoints]]
/// method = "run"
/// kind = "run"
///
/// [[crates.services.entrypoints]]
/// method = "into_router"
/// kind = "finalize"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrypointSpec {
    /// Name of the method on the owner type (e.g. `"run"`, `"into_router"`).
    pub method: String,
    /// `"run"` for a long-lived async entrypoint; `"finalize"` for a consuming
    /// transform.  Checked at validation time.
    pub kind: String,
}

/// Full configuration for one service definition in `[[crates.services]]`.
///
/// ```toml
/// [[crates.services]]
/// owner_type = "App"
/// constructor = "new"
/// configurators = ["set_address", "set_tls"]
/// skip_languages = ["wasm"]
///
/// [[crates.services.registrations]]
/// method = "add_route"
/// callback_param = "handler"
/// callback_bound = "IntoHandler"
/// callback_contract = "Handler"
///
/// [[crates.services.entrypoints]]
/// method = "run"
/// kind = "run"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Name of the owner/builder type in the extracted surface (e.g. `"App"`).
    pub owner_type: String,
    /// Name of the constructor method (defaults to `"new"` when absent).
    #[serde(default)]
    pub constructor: Option<String>,
    /// Names of chaining configurator methods (no callbacks).
    #[serde(default)]
    pub configurators: Vec<String>,
    /// Registration points — each binds a callback to a slot.
    #[serde(default)]
    pub registrations: Vec<RegistrationSpec>,
    /// Entrypoints — run or finalize methods.
    #[serde(default)]
    pub entrypoints: Vec<EntrypointSpec>,
    /// Language backends that should NOT generate a service API for this entry.
    /// Values must match canonical language names (`"python"`, `"node"`, etc.).
    #[serde(default)]
    pub skip_languages: Vec<String>,
}

/// Configuration for one handler-contract entry in `[[crates.handler_contracts]]`.
///
/// This augments a trait already present in the extracted surface with
/// service-specific metadata: the dispatch method that backends must bridge,
/// whether that method is async, and the names of the wire request/response DTOs.
///
/// ```toml
/// [[crates.handler_contracts]]
/// trait_name = "Handler"
/// dispatch_method = "call"
/// is_async = true
/// wire_request_type = "RequestData"
/// wire_response_type = "ResponseData"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerContractConfig {
    /// Name of the Rust trait in the surface (e.g. `"Handler"`).
    pub trait_name: String,
    /// Name of the primary dispatch method (e.g. `"call"`).
    pub dispatch_method: String,
    /// Whether the dispatch method is async (defaults to `true`).
    #[serde(default = "default_true")]
    pub is_async: bool,
    /// Name of the wire request DTO (e.g. `"RequestData"`).
    /// When absent, the dispatch method's signature is used verbatim.
    #[serde(default)]
    pub wire_request_type: Option<String>,
    /// Name of the wire response DTO (e.g. `"ResponseData"`).
    /// When absent, the dispatch method's return type is used verbatim.
    #[serde(default)]
    pub wire_response_type: Option<String>,
    /// Methods that backends may optionally override (have default implementations
    /// in the trait).  Subset of the trait's method names.
    #[serde(default)]
    pub optional_overrides: Vec<String>,
    /// Verbatim parameter declarations the generated bridge inserts *before* the wire
    /// request parameter and then ignores in the body. Used when the dispatch method
    /// has leading parameters whose concrete types cannot be reconstructed from the
    /// sanitized surface (e.g. foreign framework types). Default: none.
    #[serde(default)]
    pub dispatch_extra_params: Vec<String>,
    /// Name of the wire request parameter in the generated dispatch signature.
    /// When absent, defaults to `"request"`.
    #[serde(default)]
    pub wire_param_name: Option<String>,
    /// Verbatim return type for the generated dispatch future's `Output`. When absent,
    /// the bridge synthesizes `Result<{wire_response}, Box<dyn Error + Send + Sync>>`.
    /// Set this when the trait's dispatch returns a library-specific type the bridge
    /// must produce via [`Self::response_adapter`].
    #[serde(default)]
    pub dispatch_return_type: Option<String>,
    /// Path to a library function that converts the bridge's
    /// `Result<{wire_response}, Box<dyn Error + Send + Sync>>` outcome into the
    /// [`Self::dispatch_return_type`]. When absent, the bridge returns the wire response
    /// directly. The function is opaque to the generator — it simply emits a call to it.
    #[serde(default)]
    pub response_adapter: Option<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_config_minimal_deserializes() {
        let toml_str = r#"
owner_type = "App"
"#;
        let cfg: ServiceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.owner_type, "App");
        assert!(cfg.constructor.is_none());
        assert!(cfg.configurators.is_empty());
        assert!(cfg.registrations.is_empty());
        assert!(cfg.entrypoints.is_empty());
        assert!(cfg.skip_languages.is_empty());
    }

    #[test]
    fn service_config_full_roundtrips() {
        let toml_str = r#"
owner_type = "App"
constructor = "new"
configurators = ["set_address", "set_tls"]
skip_languages = ["wasm"]

[[registrations]]
method = "add_route"
callback_param = "handler"
callback_bound = "IntoHandler"
callback_contract = "Handler"

[[entrypoints]]
method = "run"
kind = "run"

[[entrypoints]]
method = "into_router"
kind = "finalize"
"#;
        let cfg: ServiceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.owner_type, "App");
        assert_eq!(cfg.constructor.as_deref(), Some("new"));
        assert_eq!(cfg.configurators, vec!["set_address", "set_tls"]);
        assert_eq!(cfg.registrations.len(), 1);
        assert_eq!(cfg.registrations[0].method, "add_route");
        assert_eq!(cfg.registrations[0].callback_bound, "IntoHandler");
        assert_eq!(cfg.registrations[0].callback_contract, "Handler");
        assert_eq!(cfg.entrypoints.len(), 2);
        assert_eq!(cfg.entrypoints[0].kind, "run");
        assert_eq!(cfg.entrypoints[1].kind, "finalize");
        assert_eq!(cfg.skip_languages, vec!["wasm"]);
    }

    #[test]
    fn handler_contract_config_defaults() {
        let toml_str = r#"
trait_name = "Handler"
dispatch_method = "call"
"#;
        let cfg: HandlerContractConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.trait_name, "Handler");
        assert_eq!(cfg.dispatch_method, "call");
        assert!(cfg.is_async, "is_async should default to true");
        assert!(cfg.wire_request_type.is_none());
        assert!(cfg.wire_response_type.is_none());
        assert!(cfg.optional_overrides.is_empty());
    }

    #[test]
    fn handler_contract_config_full() {
        let toml_str = r#"
trait_name = "Handler"
dispatch_method = "call"
is_async = true
wire_request_type = "RequestData"
wire_response_type = "ResponseData"
optional_overrides = ["on_error"]
"#;
        let cfg: HandlerContractConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.wire_request_type.as_deref(), Some("RequestData"));
        assert_eq!(cfg.wire_response_type.as_deref(), Some("ResponseData"));
        assert_eq!(cfg.optional_overrides, vec!["on_error"]);
    }
}
