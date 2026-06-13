//! Lifecycle hooks, error classes, WebSocket and SSE emission for magnus (Ruby) backend.

use crate::core::ir::{ApiSurface, HttpStatus, LifecycleHookDef, SseRouteDef, WebSocketRouteDef};
use minijinja::context;

fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::magnus::template_env::render(template_name, ctx)
}

fn format_ruby_comment(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    let pad = " ".repeat(indent);
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push_str(&pad);
            out.push_str("#\n");
        } else {
            out.push_str(&pad);
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Emit lifecycle hook registration methods for the service class.
pub(super) fn gen_lifecycle_hooks_for_class(out: &mut String, hooks: &[LifecycleHookDef]) {
    for hook in hooks {
        let method_name = &hook.name;
        let doc_comment = format_ruby_comment(&hook.doc, 6);

        out.push_str(&render(
            "service_rb_lifecycle_hook.rb.jinja",
            context! {
                method_name => method_name,
                doc_comment => doc_comment,
            },
        ));
    }
}

/// Emit WebSocket route registration methods for the service class.
pub(super) fn gen_websocket_methods_for_class(out: &mut String, routes: &[WebSocketRouteDef]) {
    for ws in routes {
        let doc_comment = format_ruby_comment(&ws.doc, 6);

        out.push_str(&render(
            "service_rb_websocket_method.rb.jinja",
            context! {
                doc_comment => doc_comment,
            },
        ));
    }
}

/// Emit SSE route registration methods for the service class.
pub(super) fn gen_sse_methods_for_class(out: &mut String, routes: &[SseRouteDef]) {
    for sse in routes {
        let doc_comment = format_ruby_comment(&sse.doc, 6);

        out.push_str(&render(
            "service_rb_sse_method.rb.jinja",
            context! {
                doc_comment => doc_comment,
            },
        ));
    }
}

/// Emit the Spikard::Errors module with exception class hierarchy.
pub(super) fn gen_error_classes(api: &ApiSurface) -> String {
    let mut out = String::new();

    if api.error_types.is_empty() {
        return out;
    }

    out.push_str("module Spikard\n");
    out.push_str("  module Errors\n");

    // Emit base Error class
    out.push_str(&render("service_rb_error_base_class.rb.jinja", context! {}));

    // Emit error subclasses
    for error in &api.error_types {
        let class_name = &error.name;
        let http_status = match &error.http_status {
            HttpStatus::BadRequest => 400,
            HttpStatus::Unauthorized => 401,
            HttpStatus::Forbidden => 403,
            HttpStatus::NotFound => 404,
            HttpStatus::Conflict => 409,
            HttpStatus::UnprocessableEntity => 422,
            HttpStatus::TooManyRequests => 429,
            HttpStatus::InternalServerError => 500,
            HttpStatus::Custom(code) => *code as i32,
        };
        let problem_details_type = error
            .problem_details_type
            .as_deref()
            .map(|s| format!("\"{}\"", s))
            .unwrap_or_else(|| "nil".to_string());
        let doc_comment = format_ruby_comment(&error.doc, 4);

        out.push_str(&render(
            "service_rb_error_subclass.rb.jinja",
            context! {
                class_name => class_name,
                http_status => http_status,
                problem_details_type => problem_details_type,
                doc_comment => doc_comment,
            },
        ));
    }

    out.push_str("  end\n");
    out.push_str("end\n");

    out
}
