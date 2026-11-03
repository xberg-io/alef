use crate::e2e::fixture::Fixture;

pub(super) fn setup(fixture: &Fixture, class_name: &str, factory: &str, json: &str) -> String {
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(json);
    let api_key_var = fixture.env.as_ref().and_then(|env| env.api_key_var.as_deref());
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key = if api_key_var.is_some() {
        "(clientApiKey == null || clientApiKey.isEmpty ? 'test-key' : clientApiKey)"
    } else {
        "'test-key'"
    };
    let url = format!("_fixtureUrl('{}')", super::escape_dart(&fixture.id));
    let base_url = if !has_mock {
        "null".to_string()
    } else if api_key_var.is_some() {
        format!("(clientApiKey == null || clientApiKey.isEmpty ? {url} : null)")
    } else {
        url
    };
    crate::e2e::template_env::render(
        "dart/client_options_setup.jinja",
        crate::alef_context! {
            api_key_var => api_key_var.map(super::escape_dart),
            has_mock => has_mock,
            class_name => class_name,
            factory => super::snake_to_lower_camel_case(factory),
            prefix => super::escape_dart(&prefix),
            api_key => api_key,
            base_url => base_url,
        },
    )
    .trim_end()
    .to_string()
}
