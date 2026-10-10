use crate::backends::rustler::template_env;

pub(super) fn render_public_nif_call(
    native_module: &str,
    function_name: &str,
    arguments: &str,
    unwrap_result: bool,
    multiline: bool,
    indent: &str,
) -> String {
    template_env::render(
        "elixir_public_nif_call.ex.jinja",
        crate::alef_context! {
            native_mod => native_module,
            func_name => function_name,
            args => arguments,
            unwrap_result => unwrap_result,
            multiline => multiline,
            indent => indent,
        },
    )
}
