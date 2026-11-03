use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;
use heck::ToLowerCamelCase;

pub(super) fn expression(
    fixture: &Fixture,
    config: &E2eConfig,
    call: &CallConfig,
    language: &str,
    class_name: &str,
    mock_url: &str,
) -> Option<String> {
    let (factory, json) =
        crate::e2e::codegen::client_factory::client_options_json_call(fixture, config, call, language)?;
    let factory = factory.to_lower_camel_case();
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let prefix = crate::e2e::escape::escape_kotlin(&prefix);
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let credential = fixture
        .env
        .as_ref()
        .and_then(|env| env.api_key_var.as_deref())
        .map(|name| {
            let name = crate::e2e::escape::escape_kotlin(name);
            format!("System.getenv(\"{name}\")?.takeIf {{ it.isNotEmpty() }}")
        });
    let api_key = credential
        .as_ref()
        .map_or_else(|| "\"test-key\"".into(), |key| format!("({key} ?: \"test-key\")"));
    let base_url = if has_mock {
        credential.map_or_else(
            || mock_url.to_string(),
            |key| format!("(if ({key} == null) {mock_url} else null)"),
        )
    } else {
        "null".into()
    };
    let serializer = "com.fasterxml.jackson.databind.ObjectMapper().writeValueAsString";
    let key_json = format!("{serializer}({api_key})");
    let url_json = format!("{serializer}({base_url})");
    Some(format!(
        r#"{class_name}.{factory}("{prefix}" + {key_json} + ",\"base_url\":" + {url_json} + "}}")"#
    ))
}
