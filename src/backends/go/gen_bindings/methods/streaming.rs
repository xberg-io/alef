use super::super::cancellation;
use super::super::types::emit_type_doc;
use super::gen_param_to_c;
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::{go_optional_type, go_type};
use crate::codegen::naming::{go_param_name, go_type_name, to_go_name};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};

/// Generate a streaming wrapper for a method decorated with the `Streaming` adapter pattern.
///
/// The returned Go method consumes the FFI iterator-handle exports
/// (`<prefix>_<type>_<method>_start`, `_next`, `_free`) and exposes a `*<Recv><Method>Stream`
/// iterator to Go callers (see `streaming_stream_type.jinja`), following the `sql.Rows`
/// convention: `Chan()` for the item channel, `Err()` for the reason the stream ended. A
/// goroutine drives `_next` until null (clean end-of-stream) or a stream error is signalled --
/// distinguished by a `lastError()` read on the null-chunk path -- or a per-item conversion
/// failure, then frees the handle.
///
/// The cancellable `WithContext` method's FIRST parameter is always `ctx context.Context` (Go
/// convention). It is never passed to the native symbols itself: a cancel token tied to it is
/// handed to `_start_cancellable`, so ending `ctx` aborts the stream-open request and a `_next`
/// blocked on the network. It also unblocks the forwarding goroutine's channel send. See issue
/// #448: on an unbuffered channel, a consumer that `break`s out of
/// `for chunk := range stream.Chan()` early parks that goroutine on `ch <- chunk` forever,
/// leaking both the goroutine and the native stream handle (the deferred
/// `close(ch)`/`C.<fn_free>(handle)` never run).
/// `streaming_method_body.jinja` races the send against `<-ctx.Done()` to fix that. The original
/// channel-returning method remains as a compatibility wrapper around `context.Background()`.
/// ~keep
#[allow(clippy::too_many_arguments)]
pub(in crate::backends::go::gen_bindings) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    item_type: &str,
    data_enum_names: &std::collections::HashSet<&str>,
    opaque_names: &std::collections::HashSet<&str>,
    _value_only_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);
    let method_with_context_name = format!("{method_go_name}WithContext");
    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);
    let item_go_type = go_type_name(item_type);
    let stream_type_name = format!("{go_receiver_type}{method_go_name}Stream");

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_stream_type.jinja",
        minijinja::context! {
            stream_type_name => &stream_type_name,
            receiver_type => &go_receiver_type,
            method_name => &method_go_name,
            item_type => &item_go_type,
        },
    ));

    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a streaming method.");

    let item_is_sum_type = data_enum_names.contains(item_type);

    let method_params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type: String = if p.optional {
                go_optional_type(&p.ty).into_owned()
            } else if let TypeRef::Named(name) = &p.ty {
                if opaque_names.contains(name.as_str()) {
                    format!("*{}", go_type(&p.ty))
                } else {
                    go_type(&p.ty).into_owned()
                }
            } else {
                go_type(&p.ty).into_owned()
            };
            format!("{} {}", go_param_name(&p.name), param_type)
        })
        .collect();
    let param_args = method
        .params
        .iter()
        .map(|p| go_param_name(&p.name))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_method_compat.jinja",
        minijinja::context! {
            receiver_name => receiver_name,
            receiver_type => &go_receiver_type,
            method_name => &method_go_name,
            method_with_context_name => &method_with_context_name,
            params => method_params.join(", "),
            param_args => &param_args,
            item_type => &item_go_type,
        },
    ));

    let mut context_params = vec!["ctx context.Context".to_string()];
    context_params.extend(method_params);

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_method_signature.jinja",
        minijinja::context! {
            receiver_name => receiver_name,
            receiver_type => &go_receiver_type,
            method_name => &method_with_context_name,
            params => context_params.join(", "),
            item_type => &item_go_type,
            stream_type_name => &stream_type_name,
        },
    ));

    for param in &method.params {
        out.push_str(&gen_param_to_c(
            param,
            "nil, ",
            true,
            ffi_prefix,
            opaque_names,
            enum_names,
            ffi_param_enum_names,
        ));
    }

    let c_params: Vec<String> = method
        .params
        .iter()
        .flat_map(|p| -> Vec<String> {
            let c_name = go_param_name(&format!("c_{}", p.name));
            if matches!(p.ty, TypeRef::Bytes) {
                vec![c_name.clone(), format!("{}Len", c_name)]
            } else {
                vec![c_name]
            }
        })
        .collect();

    let fn_start = c_symbols::stream_adapter_symbol(ffi_prefix, &typ.name, &method.name, "start_cancellable");
    let fn_next = c_symbols::stream_adapter_symbol(ffi_prefix, &typ.name, &method.name, "next");
    let fn_free = c_symbols::stream_adapter_symbol(ffi_prefix, &typ.name, &method.name, "free");
    let item_to_json_fn = c_symbols::method_symbol(ffi_prefix, item_type, "to_json");
    let item_free_fn = c_symbols::method_symbol(ffi_prefix, item_type, "free");

    let c_receiver = format!("{}.ptr", receiver_name);
    let start_call = cancellation::with_token_arg(&if c_params.is_empty() {
        format!("C.{}({})", fn_start, c_receiver)
    } else {
        format!("C.{}({}, {})", fn_start, c_receiver, c_params.join(", "))
    });

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_method_body.jinja",
        minijinja::context! {
            cancel_prelude => cancellation::prelude_with_stop_closure(ffi_prefix, "nil, "),
            start_call => &start_call,
            ffi_prefix => ffi_prefix,
            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
            method_name => &method.name,
            fn_next => &fn_next,
            fn_free => &fn_free,
            item_to_json_fn => &item_to_json_fn,
            item_free_fn => &item_free_fn,
            item_type => &item_go_type,
            item_is_sum_type => item_is_sum_type,
            stream_type_name => &stream_type_name,
        },
    ));

    out
}
