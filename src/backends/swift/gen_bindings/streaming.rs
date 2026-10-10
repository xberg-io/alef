pub(super) fn render_streaming_chunk_decode(
    item_type: &str,
    item_type_from_json: &str,
    is_first_class: bool,
    indent: &str,
) -> String {
    crate::backends::swift::template_env::render(
        "swift_streaming_chunk_decode.swift.jinja",
        crate::alef_context! {
            item_type => item_type,
            item_type_from_json => item_type_from_json,
            is_first_class => is_first_class,
            indent => indent,
        },
    )
}
