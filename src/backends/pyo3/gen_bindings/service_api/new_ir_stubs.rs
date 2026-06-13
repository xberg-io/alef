//! Stub emission hooks for the new IR sections (Phase C seams).
//!
//! Each function walks the corresponding IR collection and returns an empty
//! string — keeping existing snapshot tests green — while logging a `debug!`
//! message so wave-2 specialists can identify the gap. When a specialist
//! implements real emission for this backend, they replace the body of the
//! relevant function with Jinja-template-driven code.

use crate::core::ir::{ApiSurface, ErrorTypeDef, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};

/// Emit PyO3 lifecycle-hook registration methods.
///
/// Stub: logs a debug message and returns `""` until the pyo3 Phase-C specialist
/// implements `app.on_request(fn)` / `app.pre_handler(fn)` / … generation via
/// Jinja templates.
pub(super) fn emit_lifecycle_hooks(hooks: &[LifecycleHookDef]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "lifecycle hook emission not implemented for pyo3 ({} hooks)",
        hooks.len()
    );
    // Walk without panicking so the caller's loop is exercised.
    for _hook in hooks {}
    String::new()
}

/// Emit PyO3 WebSocket route registration methods.
///
/// Stub: logs a debug message and returns `""` until the pyo3 Phase-C specialist
/// implements `app.websocket(path, handler_fn)` generation.
pub(super) fn emit_websocket_routes(routes: &[WebSocketRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!(
        "WebSocket route emission not implemented for pyo3 ({} routes)",
        routes.len()
    );
    for _route in routes {}
    String::new()
}

/// Emit PyO3 SSE route registration methods.
///
/// Stub: logs a debug message and returns `""` until the pyo3 Phase-C specialist
/// implements `app.sse(path, producer_fn)` generation.
pub(super) fn emit_sse_routes(routes: &[SseRouteDef]) -> String {
    if routes.is_empty() {
        return String::new();
    }
    tracing::debug!("SSE route emission not implemented for pyo3 ({} routes)", routes.len());
    for _route in routes {}
    String::new()
}

/// Emit PyO3 error classes.
///
/// Stub: logs a debug message and returns `""` until the pyo3 Phase-C specialist
/// implements native Python exception class generation.
pub(super) fn emit_error_types(types: &[ErrorTypeDef]) -> String {
    if types.is_empty() {
        return String::new();
    }
    tracing::debug!("error type emission not implemented for pyo3 ({} types)", types.len());
    for _ty in types {}
    String::new()
}

/// Gate and forward all four new-IR-section stub emitters.
///
/// Returns a concatenation of the four stubs (all empty until Phase-C work
/// lands). Called from the service-API generator so the stubs are exercised
/// even when no lifecycle hooks / WebSocket / SSE / error types are configured.
pub(super) fn emit_new_ir_sections(api: &ApiSurface) -> String {
    let mut out = String::new();
    out.push_str(&emit_lifecycle_hooks(&api.lifecycle_hooks));
    out.push_str(&emit_websocket_routes(&api.websocket_routes));
    out.push_str(&emit_sse_routes(&api.sse_routes));
    out.push_str(&emit_error_types(&api.error_types));
    out
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, ErrorTypeDef, HttpStatus, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};

    #[test]
    fn emit_new_ir_sections_returns_empty_for_default_surface() {
        let surface = ApiSurface::default();
        assert_eq!(
            emit_new_ir_sections(&surface),
            "",
            "all stubs should produce empty output"
        );
    }

    #[test]
    fn lifecycle_hook_stub_does_not_panic_on_non_empty_collection() {
        let hooks = vec![LifecycleHookDef {
            name: "on_request".to_owned(),
            callback_contract: "Hook".to_owned(),
            doc: String::new(),
            is_async: false,
        }];
        let out = emit_lifecycle_hooks(&hooks);
        assert_eq!(out, "", "stub must return empty string");
    }

    #[test]
    fn websocket_route_stub_does_not_panic() {
        let routes = vec![WebSocketRouteDef {
            handler_wrapper_type: "WsHandler".to_owned(),
            socket_type: "WsSocket".to_owned(),
            doc: String::new(),
        }];
        assert_eq!(emit_websocket_routes(&routes), "");
    }

    #[test]
    fn sse_route_stub_does_not_panic() {
        let routes = vec![SseRouteDef {
            producer_wrapper_type: "SseProducer".to_owned(),
            event_type: "SseEvent".to_owned(),
            doc: String::new(),
        }];
        assert_eq!(emit_sse_routes(&routes), "");
    }

    #[test]
    fn error_type_stub_does_not_panic() {
        let types = vec![ErrorTypeDef {
            name: "NotFoundError".to_owned(),
            http_status: HttpStatus::NotFound,
            problem_details_type: None,
            doc: String::new(),
        }];
        assert_eq!(emit_error_types(&types), "");
    }
}
