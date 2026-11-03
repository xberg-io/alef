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
    let key_json = format!("JSON.stringify(({api_key}) || 'test-key')");
    let url_json = format!("JSON.stringify(({base_url}) ?? null)");
    format!(r#"{factory}("{prefix}" + {key_json} + ",\"base_url\":" + {url_json} + "}}")"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> (Fixture, E2eConfig) {
        let fixture = serde_json::from_value(serde_json::json!({
            "id": "options", "description": "Options", "input": {},
            "client_options": {"blocked_networks": ["10.0.0.0/8"], "large": u64::MAX}
        }))
        .expect("fixture");
        let config = serde_json::from_value(serde_json::json!({
            "call": {"function": "ping", "overrides": {"node": {
                "client_factory": "create_client", "client_factory_from_json": "create_client_from_json"
            }}}
        }))
        .expect("configuration");
        (fixture, config)
    }

    #[test]
    fn empty_runtime_credential_uses_mock_key_without_losing_options_or_url() {
        let (fixture, config) = inputs();
        let url = "http://127.0.0.1/quote\"slash\\雪";
        let call = expression(
            &fixture,
            &config,
            &config.call,
            "node",
            "create_client",
            ("apiKey ?? 'test-key'", Some("baseUrl")),
        );
        let url_literal = serde_json::to_string(url).expect("URL literal");
        let source = format!(
            "const apiKey = ''; const baseUrl = {url_literal}; let calls = 0; function create_client_from_json(json) {{ calls++; process.stdout.write(json); }}; {call}; process.stderr.write('calls=' + calls + '\\n');"
        );
        let mut command = crate::core::tool_command("node");
        command.args(["-e", &source]);
        let output = crate::snippets::validators::run_command_streams(&mut command, 30).expect("Node runtime");
        assert!(output.success, "{}", output.stderr);
        assert_eq!(output.stderr, "calls=1\n");
        let actual: serde_json::Value = serde_json::from_str(&output.stdout).expect("recorded options");
        assert_eq!(
            actual,
            serde_json::json!({
                "blocked_networks": ["10.0.0.0/8"], "large": u64::MAX,
                "api_key": "test-key", "base_url": url
            })
        );
    }

    #[test]
    fn absent_options_preserve_the_ordinary_factory_expression() {
        let (mut fixture, config) = inputs();
        fixture.client_options = None;
        assert_eq!(
            expression(
                &fixture,
                &config,
                &config.call,
                "node",
                "create_client",
                ("apiKey ?? 'test-key'", Some("baseUrl"))
            ),
            "create_client(apiKey ?? 'test-key', baseUrl)"
        );
    }
}
