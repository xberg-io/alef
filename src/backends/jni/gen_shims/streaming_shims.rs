/// Emit Start/Next/Free streaming shims for one adapter.
#[allow(clippy::too_many_arguments)]
fn emit_streaming_shims(
    out: &mut String,
    start_sym: &str,
    next_sym: &str,
    free_sym: &str,
    ty: &TypeDef,
    adapter: &crate::core::config::AdapterConfig,
    _api: &ApiSurface,
) {
    let type_name = &ty.name;
    let adapter_pascal = internal_class_component(&adapter.name);
    let stream_handle_type = format!("{type_name}{adapter_pascal}StreamHandle");
    let adapter_method = adapter.name.replace('-', "_");

    let item_type = adapter
        .item_type
        .as_deref()
        .map(|t| format!("core_crate::{t}"))
        .unwrap_or_else(|| "serde_json::Value".to_string());

    let stream_item_alias = format!("{stream_handle_type}Item");
    let stream_box_alias = format!("{stream_handle_type}Stream");
    let mut request_unmarshal = String::new();
    let stream_call_block;
    if let Some(first_param) = adapter.params.first() {
        let param_type = first_param.ty.rsplit("::").next().unwrap_or(&first_param.ty);
        request_unmarshal.push_str(&template_env::render(
            "stream_request_unmarshal.rs.jinja",
            context! {
                param_type => param_type,
            },
        ));
        stream_call_block = template_env::render(
            "stream_call_block.rs.jinja",
            context! {
                adapter_method => adapter_method,
                request_arg => "request",
            },
        );
    } else {
        stream_call_block = template_env::render(
            "stream_call_block.rs.jinja",
            context! {
                adapter_method => adapter_method,
                request_arg => "",
            },
        );
    }

    out.push_str(&template_env::render(
        "streaming_shims.rs.jinja",
        context! {
            stream_item_alias => stream_item_alias,
            stream_box_alias => stream_box_alias,
            stream_handle_type => stream_handle_type,
            item_type => item_type,
            start_sym => start_sym,
            next_sym => next_sym,
            free_sym => free_sym,
            type_name => type_name,
            request_unmarshal => request_unmarshal,
            stream_call_block => stream_call_block,
        },
    ));
}
