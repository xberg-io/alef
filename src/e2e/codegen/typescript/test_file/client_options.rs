use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

pub(super) fn expression(
    fixture: &Fixture,
    config: &E2eConfig,
    call: &CallConfig,
    language: &str,
    factory: &str,
    (api_key, base_url): (&str, Option<&str>),
) -> String {
    let Some((factory, json)) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, config, call, language)
    else {
        let url_arg = base_url.map(|url| format!(", {url}")).unwrap_or_default();
        return format!("{factory}({api_key}{url_arg})");
    };
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let prefix = crate::e2e::escape::escape_js(&prefix);
    let base_url = base_url.unwrap_or("null");
    let key_json = format!("JSON.stringify({api_key})");
    let url_json = format!("JSON.stringify(({base_url}) ?? null)");
    format!(r#"{factory}("{prefix}" + {key_json} + ",\"base_url\":" + {url_json} + "}}")"#)
}
