//! Stub emission hooks for the new IR sections (Phase C seams) — NAPI/TypeScript backend.
//!
//! Each function walks the corresponding IR collection and returns an empty
//! string so existing snapshot tests pass unchanged. A `debug!` message is
//! logged the first time a non-empty collection is encountered so wave-2
//! specialists can identify the gap.

use crate::core::ir::{ApiSurface, ErrorTypeDef, SseRouteDef, WebSocketRouteDef};

/// Emit NAPI WebSocket route registration methods.
///
/// Stub: logs a debug message and returns `""` until the napi Phase-C specialist
/// implements `app.websocket(path, handler)` TypeScript generation.
pub(super) fn emit_websocket_routes(routes: &[WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for napi ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit NAPI SSE route registration methods.
///
/// Stub: logs a debug message and returns `""` until the napi Phase-C specialist
/// implements `app.sse(path, producer)` TypeScript generation.
pub(super) fn emit_sse_routes(routes: &[SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!("SSE route emission not implemented for napi ({} routes)", routes.len());
    for _route in routes {}
    String::new()
}

/// Emit NAPI native error/exception classes.
///
/// Stub: logs a debug message and returns `""` until the napi Phase-C specialist
/// implements TypeScript error class generation.
pub(super) fn emit_error_types(types: &[ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for napi ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Emit WebSocket routes, SSE routes, and error types for the NAPI backend.
///
/// Lifecycle hooks are already handled in `typescript.rs` via `gen_lifecycle_hook_ts`.
/// This function covers the remaining three new IR sections.
pub(super) fn emit_new_ir_sections(api: &ApiSurface) -> String {
    let mut out = String::new();
    out.push_str(&emit_websocket_routes(&api.websocket_routes));
    out.push_str(&emit_sse_routes(&api.sse_routes));
    out.push_str(&emit_error_types(&api.error_types));
    out
}
