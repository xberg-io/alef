//! Stub emission hooks for the new IR sections (Phase C seams) — Rustler/Elixir backend.

use crate::core::ir::{ApiSurface, ErrorTypeDef, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};

/// Emit Rustler lifecycle-hook registration methods. Stub.
pub(super) fn emit_lifecycle_hooks(hooks: &[LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for rustler ({} hooks)",
        hooks.len()
    );
    for _hook in hooks {}
    String::new()
}

/// Emit Rustler WebSocket route registration methods. Stub.
pub(super) fn emit_websocket_routes(routes: &[WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for rustler ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Rustler SSE route registration methods. Stub.
pub(super) fn emit_sse_routes(routes: &[SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "SSE route emission not implemented for rustler ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit Rustler native error types. Stub.
pub(super) fn emit_error_types(types: &[ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "error type emission not implemented for rustler ({} types)",
        types.len()
    );
    for _ty in types {}
    String::new()
}

/// Aggregate stub — forwards all four new IR sections for the Rustler/Elixir backend.
pub(super) fn emit_new_ir_sections(api: &ApiSurface) -> String {
    let mut out = String::new();
    out.push_str(&emit_lifecycle_hooks(&api.lifecycle_hooks));
    out.push_str(&emit_websocket_routes(&api.websocket_routes));
    out.push_str(&emit_sse_routes(&api.sse_routes));
    out.push_str(&emit_error_types(&api.error_types));
    out
}
