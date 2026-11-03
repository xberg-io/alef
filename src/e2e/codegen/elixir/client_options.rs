use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call: &CallConfig,
    module: &str,
    api_key: &str,
    base_url: &str,
) -> Option<String> {
    let (factory, json) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, e2e_config, call, "elixir")?;
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    Some(crate::e2e::template_env::render(
        "elixir/client_options.jinja",
        crate::alef_context! {
            factory => factory, module => module, api_key => api_key, base_url => base_url,
            prefix => crate::e2e::escape::escape_elixir(&prefix),
        },
    ))
}
