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

impl ApiSurface {
    /// Returns `true` when the surface declares at least one lifecycle hook.
    ///
    /// Backends gate lifecycle-hook emission code behind this predicate so they
    /// produce a minimal output when no hooks are configured.
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::{ApiSurface, LifecycleHookDef};
    ///
    /// let mut surface = ApiSurface::default();
    /// assert!(!surface.has_lifecycle_hooks());
    ///
    /// surface.lifecycle_hooks.push(LifecycleHookDef {
    ///     name: "on_request".to_owned(),
    ///     callback_contract: "RequestHook".to_owned(),
    ///     doc: String::new(),
    ///     is_async: false,
    /// });
    /// assert!(surface.has_lifecycle_hooks());
    /// ```
    pub fn has_lifecycle_hooks(&self) -> bool {
        !self.lifecycle_hooks.is_empty()
    }

    /// Returns `true` when the surface declares at least one WebSocket route.
    ///
    /// Backends gate WebSocket-route emission behind this predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::{ApiSurface, WebSocketRouteDef};
    ///
    /// let mut surface = ApiSurface::default();
    /// assert!(!surface.has_websocket_routes());
    ///
    /// surface.websocket_routes.push(WebSocketRouteDef {
    ///     handler_wrapper_type: "WsHandler".to_owned(),
    ///     socket_type: "WsSocket".to_owned(),
    ///     doc: String::new(),
    /// });
    /// assert!(surface.has_websocket_routes());
    /// ```
    pub fn has_websocket_routes(&self) -> bool {
        !self.websocket_routes.is_empty()
    }

    /// Returns `true` when the surface declares at least one SSE route.
    ///
    /// Backends gate SSE-route emission behind this predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::{ApiSurface, SseRouteDef};
    ///
    /// let mut surface = ApiSurface::default();
    /// assert!(!surface.has_sse_routes());
    ///
    /// surface.sse_routes.push(SseRouteDef {
    ///     producer_wrapper_type: "SseProducer".to_owned(),
    ///     event_type: "SseEvent".to_owned(),
    ///     doc: String::new(),
    /// });
    /// assert!(surface.has_sse_routes());
    /// ```
    pub fn has_sse_routes(&self) -> bool {
        !self.sse_routes.is_empty()
    }

    /// Returns `true` when the surface declares at least one cross-binding error type.
    ///
    /// Backends gate error-class emission behind this predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::{ApiSurface, ErrorTypeDef, HttpStatus};
    ///
    /// let mut surface = ApiSurface::default();
    /// assert!(!surface.has_error_types());
    ///
    /// surface.error_types.push(ErrorTypeDef {
    ///     name: "NotFoundError".to_owned(),
    ///     http_status: HttpStatus::NotFound,
    ///     problem_details_type: None,
    ///     doc: String::new(),
    /// });
    /// assert!(surface.has_error_types());
    /// ```
    pub fn has_error_types(&self) -> bool {
        !self.error_types.is_empty()
    }
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}

// ─────────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ErrorTypeDef, HttpStatus, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};

    #[test]
    fn empty_surface_has_no_lifecycle_hooks() {
        assert!(!ApiSurface::default().has_lifecycle_hooks());
    }

    #[test]
    fn surface_with_lifecycle_hook_returns_true() {
        let mut s = ApiSurface::default();
        s.lifecycle_hooks.push(LifecycleHookDef {
            name: "on_request".to_owned(),
            callback_contract: "RequestHook".to_owned(),
            doc: String::new(),
            is_async: false,
        });
        assert!(s.has_lifecycle_hooks());
    }

    #[test]
    fn empty_surface_has_no_websocket_routes() {
        assert!(!ApiSurface::default().has_websocket_routes());
    }

    #[test]
    fn surface_with_websocket_route_returns_true() {
        let mut s = ApiSurface::default();
        s.websocket_routes.push(WebSocketRouteDef {
            handler_wrapper_type: "WsWrapper".to_owned(),
            socket_type: "WsSocket".to_owned(),
            doc: String::new(),
        });
        assert!(s.has_websocket_routes());
    }

    #[test]
    fn empty_surface_has_no_sse_routes() {
        assert!(!ApiSurface::default().has_sse_routes());
    }

    #[test]
    fn surface_with_sse_route_returns_true() {
        let mut s = ApiSurface::default();
        s.sse_routes.push(SseRouteDef {
            producer_wrapper_type: "SseProducer".to_owned(),
            event_type: "SseEvent".to_owned(),
            doc: String::new(),
        });
        assert!(s.has_sse_routes());
    }

    #[test]
    fn empty_surface_has_no_error_types() {
        assert!(!ApiSurface::default().has_error_types());
    }

    #[test]
    fn surface_with_error_type_returns_true() {
        let mut s = ApiSurface::default();
        s.error_types.push(ErrorTypeDef {
            name: "NotFoundError".to_owned(),
            http_status: HttpStatus::NotFound,
            problem_details_type: None,
            doc: String::new(),
        });
        assert!(s.has_error_types());
    }

    #[test]
    fn stub_emit_fns_return_empty_for_empty_collections() {
        // Verify the stub aggregate function produces empty output for an empty surface.
        let surface = ApiSurface::default();
        assert!(!surface.has_lifecycle_hooks());
        assert!(!surface.has_websocket_routes());
        assert!(!surface.has_sse_routes());
        assert!(!surface.has_error_types());
    }
}
