use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

pub(super) fn render(fixture: &Fixture, e2e_config: &E2eConfig, call: &CallConfig) -> Option<String> {
    let (factory, json) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, e2e_config, call, "r")?;
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    Some(crate::e2e::template_env::render(
        "r/client_options.jinja",
        crate::alef_context! {
            factory => factory, prefix => crate::e2e::escape::escape_r(&prefix), fixture_id => fixture.id,
            has_mock => fixture.needs_mock_server(), host_root => fixture.has_host_root_route(),
            env_key => format!("MOCK_SERVER_{}",fixture.id.to_ascii_uppercase().replace('-', "_")),
            api_key_var => fixture.env.as_ref().and_then(|env|env.api_key_var.as_deref()),
        },
    ))
}
