use serde::{Deserialize, Serialize};

use super::application::{ErrorTypeDef, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};
use super::items::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use super::service::{HandlerContractDef, ServiceDef};

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    /// Type names → fully qualified rust_paths for types that were extracted but
    /// then excluded from the public binding surface. Preserved so trait_bridge
    /// codegen can still reference them by qualified path when they appear in
    /// trait method signatures (e.g. `Renderer::render(&HiddenDocument)`).
    #[serde(default)]
    pub excluded_type_paths: std::collections::HashMap<String, String>,
    /// Subset of `excluded_type_paths` keys whose underlying definition is a trait
    /// (`is_trait = true` on the original `TypeDef`). The `is_trait` flag is lost
    /// when the type is stripped, so trait-bridge codegen tracks excluded traits
    /// separately to decide whether a return-type `Named(name)` referencing an
    /// excluded item is a non-bridgeable trait object (skip the method, fall back
    /// to default impl) or a struct/enum still usable via its qualified path.
    #[serde(default)]
    pub excluded_trait_names: std::collections::HashSet<String>,
    /// Descriptions of owner/builder service types with their constructor,
    /// configurator methods, registration points, and run/finalize entrypoints.
    ///
    /// Populated by the service extraction pass when `[[crates.services]]` config
    /// entries are present. Empty for consumers that have not configured any services.
    #[serde(default)]
    pub services: Vec<ServiceDef>,
    /// Async trait contracts that service registration callbacks must satisfy.
    ///
    /// Each entry describes the trait, its dispatch method, and the wire
    /// request/response DTO names the callback receives and returns.
    ///
    /// Populated alongside [`Self::services`]. Empty when no services are configured.
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractDef>,
    /// Lifecycle hook contracts registered on the service owner.
    ///
    /// Each entry describes one named hook slot (e.g. `on_request`, `pre_handler`,
    /// `on_response`, `on_error`) that host-language consumers can bind a callback
    /// into. Backends emit `app.on_<name>(fn)` style registration methods for each entry.
    ///
    /// Populated when `[[crates.lifecycle_hooks]]` entries are present in `alef.toml`.
    /// Empty for consumers that have not configured lifecycle hooks.
    #[serde(default)]
    pub lifecycle_hooks: Vec<LifecycleHookDef>,
    /// WebSocket route registration contracts.
    ///
    /// Each entry causes backends to emit an `app.websocket(path, handler_fn)` method
    /// (or its idiomatic equivalent). Uses concrete wrapper structs to avoid RPITIT
    /// non-dyn-compatibility with `impl Future` return types.
    ///
    /// Populated when `[[crates.websocket_routes]]` entries are present in `alef.toml`.
    #[serde(default)]
    pub websocket_routes: Vec<WebSocketRouteDef>,
    /// SSE route registration contracts.
    ///
    /// Each entry causes backends to emit an `app.sse(path, producer_fn)` method
    /// (or its idiomatic equivalent). Uses concrete wrapper structs to avoid RPITIT
    /// non-dyn-compatibility with `impl AsyncIterator` return types.
    ///
    /// Populated when `[[crates.sse_routes]]` entries are present in `alef.toml`.
    #[serde(default)]
    pub sse_routes: Vec<SseRouteDef>,
    /// Cross-binding error types emitted as native exception classes in every language.
    ///
    /// Each entry describes one member of the exception hierarchy. Backends emit native
    /// exception/error classes whose `status_code()` method returns the mapped HTTP
    /// status and whose serialization produces an RFC 9457 ProblemDetails JSON body.
    ///
    /// Populated when `[[crates.error_types]]` entries are present in `alef.toml`.
    #[serde(default)]
    pub error_types: Vec<ErrorTypeDef>,
    /// Public Rust items that Alef saw during extraction but intentionally did not
    /// lower into the binding IR because their shape is not safely representable.
    ///
    /// Validation turns these into hard diagnostics instead of letting public
    /// items disappear silently before generation.
    #[serde(default)]
    pub unsupported_public_items: Vec<UnsupportedPublicItem>,
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}
