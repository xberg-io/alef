use crate::backends::zig::gen_bindings::errors::resolve_zig_error_type;
use crate::backends::zig::gen_bindings::helpers::emit_cleaned_zig_doc;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::{AsSnakeCase, AsUpperCamelCase};
use std::collections::HashMap;

use super::render;

pub(super) struct StreamingContext<'a> {
    pub(super) ty: &'a TypeDef,
    pub(super) prefix: &'a str,
    pub(super) type_snake: &'a str,
    pub(super) item_type: &'a str,
    pub(super) declared_errors: &'a [String],
    pub(super) streaming_item_types: &'a HashMap<String, String>,
}

/// Compute the Zig struct name for a streaming method's iterator type.
///
/// Defaults to `{item_type}Stream`. That name is only safe when a single streaming
/// method on `ty` yields `item_type`: the emitted struct's `next()`/`deinit()` bodies
/// hardcode the FFI symbols for one specific method (`{prefix}_{type_snake}_{method_snake}_next`,
/// `..._free`), not the item type. If a second streaming method on the same `ty` yields
/// the same `item_type` (e.g. `crawl_stream` and `batch_crawl_stream` both yielding
/// `CrawlEvent`), a bare `{item_type}Stream` name would be shared by both families —
/// whichever struct gets emitted first silently "wins" the name, and callers of the
/// *other* method would receive its own handle wrapped in a type whose `next`/`deinit`
/// dispatch to the *other* family's C symbols. That is a handle type-confusion bug, not
/// a naming cosmetic — every colliding method must get its own type so the mismatch is
/// unrepresentable rather than merely avoided by emission order.
///
/// Method names are unique within `ty`, so disambiguating by method name (with a
/// redundant trailing `_stream` stripped for readability) is always collision-free.
pub(super) fn stream_struct_name(
    ty: &TypeDef,
    method_snake: &str,
    item_type: &str,
    streaming_item_types: &HashMap<String, String>,
) -> String {
    let sibling_count = ty
        .methods
        .iter()
        .filter(|m| streaming_item_types.get(&m.name).map(String::as_str) == Some(item_type))
        .count();

    if sibling_count <= 1 {
        return format!("{item_type}Stream");
    }

    let trimmed = method_snake.strip_suffix("_stream").unwrap_or(method_snake);
    format!("{}Stream", AsUpperCamelCase(trimmed))
}

/// Emit a Zig struct type for a streaming iterator.
///
/// The struct holds a stream handle and provides `next()` and `deinit()` methods
/// to incrementally consume chunks without eagerly collecting them all into memory.
pub(super) fn emit_streaming_struct(method: &MethodDef, streaming: &StreamingContext<'_>, out: &mut String) {
    let StreamingContext {
        ty,
        prefix,
        type_snake,
        item_type,
        declared_errors,
        streaming_item_types,
    } = streaming;
    let method_snake = AsSnakeCase(&method.name).to_string();
    let item_snake = AsSnakeCase(item_type).to_string();
    let struct_name = stream_struct_name(ty, &method_snake, item_type, streaming_item_types);

    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors))
        .unwrap_or_else(|| "anyerror".to_string());

    out.push_str(&render(
        "opaque_stream_struct.jinja",
        crate::alef_context! {
            item_type => item_type,
            struct_name => &struct_name,
            zig_error_type => &zig_error_type,
            prefix => prefix,
            type_snake => type_snake,
            method_snake => &method_snake,
            item_snake => &item_snake,
            cancelled_code => crate::core::ir::ApiSurface::FFI_ERROR_CODE_CANCELLED,
        },
    ));
}

/// Emit a streaming method on an opaque handle wrapper struct.
///
/// Streaming methods use the iterator-handle pattern (`_start` / `_next` / `_free`)
/// and return a struct type that provides `next()` and `deinit()` methods for
/// incremental, backpressure-aware consumption. Callers can cancel by dropping
/// the struct early without draining the entire stream.
pub(super) fn emit_opaque_streaming_method(method: &MethodDef, streaming: &StreamingContext<'_>, out: &mut String) {
    let StreamingContext {
        ty,
        prefix,
        type_snake,
        item_type,
        declared_errors,
        streaming_item_types,
    } = streaming;
    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let method_snake = AsSnakeCase(&method.name).to_string();
    let struct_name = stream_struct_name(ty, &method_snake, item_type, streaming_item_types);
    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors))
        .unwrap_or_else(|| "anyerror".to_string());

    let req_param = method.params.first().map(|p| p.name.as_str()).unwrap_or("req");
    let req_param_lower = req_param.to_lowercase();
    let req_type_snake = if let Some(p) = method.params.first() {
        if let TypeRef::Named(n) = &p.ty {
            AsSnakeCase(n).to_string()
        } else {
            "chat_completion_request".to_string()
        }
    } else {
        "chat_completion_request".to_string()
    };

    out.push_str(&render(
        "opaque_stream_method.jinja",
        crate::alef_context! {
            method_name => &method.name,
            type_name => &ty.name,
            req_param => req_param,
            zig_error_type => &zig_error_type,
            struct_name => &struct_name,
            req_param_lower => &req_param_lower,
            prefix => prefix,
            req_type_snake => &req_type_snake,
            type_snake => type_snake,
            method_snake => &method_snake,
            c_handle => "handle",
        },
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn stream_template_checks_json_and_releases_every_native_value() {
        let rendered = crate::backends::zig::template_env::render(
            "opaque_stream_struct.jinja",
            crate::alef_context! {
                struct_name => "RecordStream",
                item_type => "Record",
                zig_error_type => "RequestError",
                prefix => "sample",
                type_snake => "client",
                method_snake => "stream_records",
                item_snake => "record",
                cancelled_code => 5,
            },
        );

        assert!(rendered.contains("if (_json == null) return _error_with_message(RequestError)"));
        assert!(rendered.contains("defer c.sample_record_free(_chunk)"));
        assert!(rendered.contains("defer c.sample_free_string(_json)"));
        assert!(rendered.contains("pub fn deinit"));
        assert!(rendered.contains("sample_client_stream_records_free"));
        assert!(rendered.contains("_handle: u64"));
        assert!(rendered.contains("if (_chunk == 0)"));
        assert!(
            rendered.contains("if (_code == 5) return error.Cancelled;"),
            "a cancelled stream must not surface as UnknownFfiError:\n{rendered}"
        );
        assert!(rendered.contains("if (_code != 0) return error.UnknownFfiError;"));
        assert!(rendered.contains("self._handle = 0"));
    }
}
