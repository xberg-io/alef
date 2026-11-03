use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call: &CallConfig,
    language: &str,
    namespace: &str,
    class_name: &str,
) -> Option<String> {
    let (factory, json) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, e2e_config, call, language)?;
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let mock_url = if fixture.has_host_root_route() {
        format!(
            "(getenv('MOCK_SERVER_{}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{}')",
            fixture.id.to_ascii_uppercase().replace('-', "_"),
            fixture.id
        )
    } else {
        format!("getenv('MOCK_SERVER_URL') . '/fixtures/{}'", fixture.id)
    };
    Some(crate::e2e::template_env::render(
        "php/client_options.jinja",
        crate::alef_context! {
            factory => factory, namespace => namespace, class_name => class_name,
            prefix => crate::e2e::escape::escape_php_single(&prefix), mock_url => mock_url,
            has_mock => fixture.needs_mock_server(),
            api_key_var => fixture.env.as_ref().and_then(|env| env.api_key_var.as_deref()),
        },
    ))
}
