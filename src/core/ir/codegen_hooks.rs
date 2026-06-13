//! Backend-agnostic emission hook traits for the new IR sections.
//!
//! Each trait declares the seam where a backend drops in its Jinja-template-driven
//! emission for one of the four new IR collections on [`ApiSurface`]:
//!
//! | Trait | IR collection | Gating predicate |
//! |---|---|---|
//! | [`LifecycleHookEmitter`] | `lifecycle_hooks` | [`ApiSurface::has_lifecycle_hooks`] |
//! | [`WebSocketRouteEmitter`] | `websocket_routes` | [`ApiSurface::has_websocket_routes`] |
//! | [`SseRouteEmitter`] | `sse_routes` | [`ApiSurface::has_sse_routes`] |
//! | [`ErrorTypeEmitter`] | `error_types` | [`ApiSurface::has_error_types`] |
//!
//! ## Conventions for implementors
//!
//! - Walk the IR slice without panicking — all inputs come from validated IR.
//! - Log a `tracing::debug!` message when returning a stub/empty string so the
//!   Phase-C specialist knows which backend still needs work.
//! - Return an empty [`String`] (or `()` for side-effectful variants) from stubs
//!   so existing snapshot tests continue to pass unchanged.
//! - Switch to Jinja templates (via the backend's `template_env::render`) rather
//!   than Rust string-pushing when writing real emission — see the project rule on
//!   preferring templates.
//!
//! [`ApiSurface::has_lifecycle_hooks`]: super::surface::ApiSurface::has_lifecycle_hooks
//! [`ApiSurface::has_websocket_routes`]: super::surface::ApiSurface::has_websocket_routes
//! [`ApiSurface::has_sse_routes`]: super::surface::ApiSurface::has_sse_routes
//! [`ApiSurface::has_error_types`]: super::surface::ApiSurface::has_error_types

use super::application::{ErrorTypeDef, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};

/// Emit host-language lifecycle-hook registration methods.
///
/// Implementors generate one method per [`LifecycleHookDef`] entry.  The method
/// name follows the pattern `on_<name>` (or its language-idiomatic equivalent):
/// `on_request`, `pre_validation`, `pre_handler`, `on_response`, `on_error`.
///
/// # Stub contract
///
/// When a backend has not yet implemented real emission, the `emit` method must:
/// 1. Log `tracing::debug!("lifecycle hook emission not implemented for <backend>")`.
/// 2. Walk `hooks` without panicking (a simple `for _ in hooks {}` suffices).
/// 3. Return `String::new()`.
pub trait LifecycleHookEmitter {
    /// Generate code for all lifecycle hook registration methods and return it as
    /// a [`String`] to be inserted into the service class body.
    fn emit_lifecycle_hooks(&self, hooks: &[LifecycleHookDef]) -> String;
}

/// Emit a host-language WebSocket route registration method.
///
/// Implementors generate `app.websocket(path, handler_fn)` (or its idiomatic
/// equivalent) for each [`WebSocketRouteDef`] entry.
///
/// # Stub contract
///
/// When a backend has not yet implemented real emission, the `emit` method must:
/// 1. Log `tracing::debug!("WebSocket route emission not implemented for <backend>")`.
/// 2. Walk `routes` without panicking.
/// 3. Return `String::new()`.
pub trait WebSocketRouteEmitter {
    /// Generate code for all WebSocket route registration methods.
    fn emit_websocket_routes(&self, routes: &[WebSocketRouteDef]) -> String;
}

/// Emit a host-language SSE route registration method.
///
/// Implementors generate `app.sse(path, producer_fn)` (or its idiomatic
/// equivalent) for each [`SseRouteDef`] entry.
///
/// # Stub contract
///
/// When a backend has not yet implemented real emission, the `emit` method must:
/// 1. Log `tracing::debug!("SSE route emission not implemented for <backend>")`.
/// 2. Walk `routes` without panicking.
/// 3. Return `String::new()`.
pub trait SseRouteEmitter {
    /// Generate code for all SSE route registration methods.
    fn emit_sse_routes(&self, routes: &[SseRouteDef]) -> String;
}

/// Emit host-language native error/exception classes.
///
/// Implementors generate one error class per [`ErrorTypeDef`] entry, with a
/// `status_code()` method returning the mapped HTTP status and serialization
/// that produces an RFC 9457 ProblemDetails JSON body.
///
/// # Stub contract
///
/// When a backend has not yet implemented real emission, the `emit` method must:
/// 1. Log `tracing::debug!("error type emission not implemented for <backend>")`.
/// 2. Walk `types` without panicking.
/// 3. Return `String::new()`.
pub trait ErrorTypeEmitter {
    /// Generate code for all cross-binding error classes.
    fn emit_error_types(&self, types: &[ErrorTypeDef]) -> String;
}
