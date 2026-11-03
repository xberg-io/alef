use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;
use heck::ToLowerCamelCase;

pub(super) fn expression(
    fixture: &Fixture,
    config: &E2eConfig,
    call: &CallConfig,
    class_name: &str,
    api_key: &str,
    base_url: &str,
) -> Option<String> {
    let (factory, json) = crate::e2e::codegen::client_factory::client_options_json_call(fixture, config, call, "java")?;
    let factory = factory.to_lower_camel_case();
    let prefix = crate::e2e::codegen::client_factory::client_options_json_prefix(&json);
    let prefix = crate::e2e::escape::escape_java(&prefix);
    let serializer = "new com.fasterxml.jackson.databind.ObjectMapper().writeValueAsString";
    let key_json = format!("{serializer}({api_key})");
    let url_json = format!("{serializer}({base_url})");
    Some(format!(
        r#"{class_name}.{factory}("{prefix}" + {key_json} + ",\"base_url\":" + {url_json} + "}}")"#
    ))
}

pub(super) fn setup(
    fixture: &Fixture,
    config: &E2eConfig,
    call: &CallConfig,
    class_name: &str,
    factory: Option<&str>,
) -> (Vec<String>, String) {
    let Some(factory) = factory else {
        return (Vec::new(), class_name.to_string());
    };
    let factory = factory.to_lower_camel_case();
    let (mut lines, key, url) = runtime_context(fixture);
    let default_args = url
        .as_ref()
        .map(|url| format!("{key}, {url}, null, null, null"))
        .unwrap_or_else(|| key.to_string());
    let client = expression(
        fixture,
        config,
        call,
        class_name,
        &key,
        url.as_deref().unwrap_or("null"),
    )
    .unwrap_or_else(|| format!("{class_name}.{factory}({default_args})"));
    lines.push(format!("var client = {client};"));
    (lines, "client".to_string())
}

fn runtime_context(fixture: &Fixture) -> (Vec<String>, String, Option<String>) {
    let fixture_id = &fixture.id;
    let mut lines = Vec::new();
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let variable = fixture.env.as_ref().and_then(|env| env.api_key_var.as_deref());
    if let Some(var) = variable.filter(|_| has_mock) {
        lines.push(format!("String apiKey = System.getenv(\"{var}\");"));
        lines.push(format!("String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String baseUrl = (apiKey != null && !apiKey.isEmpty()) ? null : (mockServerUrl != null ? mockServerUrl + \"/fixtures/{fixture_id}\" : \"http://localhost:8000/fixtures/{fixture_id}\");"));
        lines.push(format!("System.out.println(\"{fixture_id}: \" + (baseUrl == null ? \"using real API ({var} is set)\" : \"using mock server ({var} not set)\"));"));
        return (
            lines,
            "baseUrl == null ? apiKey : \"test-key\"".into(),
            Some("baseUrl".into()),
        );
    }
    if has_mock {
        if fixture.has_host_root_route() {
            lines.push(format!("String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String defaultUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\"; String mockUrl = System.getProperty(\"mockServer.{fixture_id}\", defaultUrl);"));
        } else {
            lines.push(format!("String mockServerUrl = System.getProperty(\"mockServerUrl\"); if (mockServerUrl == null) {{ mockServerUrl = System.getenv(\"MOCK_SERVER_URL\"); }} String mockUrl = (mockServerUrl != null ? mockServerUrl : \"http://localhost:8000\") + \"/fixtures/{fixture_id}\";"));
        }
        return (lines, "\"test-key\"".into(), Some("mockUrl".into()));
    }
    if let Some(var) = variable {
        lines.push(format!("String apiKey = System.getenv(\"{var}\");"));
        lines.push(format!(
            "org.junit.jupiter.api.Assumptions.assumeTrue(apiKey != null && !apiKey.isEmpty(), \"{var} not set\");"
        ));
        return (lines, "apiKey".into(), None);
    }
    (lines, "\"test-key\"".into(), None)
}
