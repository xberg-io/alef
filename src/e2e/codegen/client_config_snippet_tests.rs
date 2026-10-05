//! A fixture's `docs.client.config` reaches every snippet backend that claims to render it: the
//! client is built by the language's `client_factory_from_json` from the configuration as a
//! string literal, and the plain factory call (with its credential read) is gone.
//!
//! One table drives all backends through [`E2eCodegen::render_snippet_body_with_functions`], so
//! a backend that opts in through [`E2eCodegen::renders_client_config`] without actually
//! rendering the configuration fails here rather than publishing a plain-client snippet.

use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::e2e::codegen::all_generators;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

const LANGUAGES: [(&str, &str, &str); 10] = [
    ("python", "create_client", "create_client_from_json"),
    ("go", "CreateClient", "CreateClientFromJSON"),
    ("node", "createClient", "createClientFromJson"),
    ("wasm", "createClient", "createClientFromJson"),
    ("java", "createClient", "createClientFromJson"),
    ("csharp", "CreateClient", "CreateClientFromJson"),
    ("ruby", "create_client", "create_client_from_json"),
    ("php", "createClient", "createClientFromJson"),
    ("elixir", "create_client", "create_client_from_json"),
    ("rust", "create_client", "create_client_from_json"),
];

fn config(from_json: bool) -> (E2eConfig, ResolvedCrateConfig) {
    let mut toml = String::from(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat"
module = "example_api"
args = [{ name = "prompt", field = "prompt", type = "string" }]
"#,
    );
    for (language, plain, from_json_factory) in LANGUAGES {
        toml.push_str(&format!(
            "[crates.e2e.call.overrides.{language}]\nclient_factory = \"{plain}\"\n"
        ));
        if from_json {
            toml.push_str(&format!("client_factory_from_json = \"{from_json_factory}\"\n"));
        }
    }
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn fixture(client_config: bool) -> Fixture {
    let mut value = serde_json::json!({
        "id": "enforced",
        "description": "a configured client",
        "input": {"prompt": "Hello"},
        "assertions": [{"type": "error"}],
    });
    if client_config {
        value["docs"] = serde_json::json!({
            "topic": "budget",
            "client": {"config": {
                "budget": {"global_limit": 0.0, "enforcement": "hard"},
                "api_key": "your-api-key",
            }},
        });
    }
    serde_json::from_value(value).expect("fixture parses")
}

fn render(language: &str, fixture: &Fixture, e2e: &E2eConfig, resolved: &ResolvedCrateConfig) -> String {
    let generator = all_generators()
        .into_iter()
        .find(|generator| generator.language_name() == language)
        .unwrap_or_else(|| panic!("no generator for {language}"));
    generator
        .render_snippet_body_with_functions(fixture, e2e, resolved, &[], &[], &[], &[])
        .unwrap_or_else(|error| panic!("{language} snippet failed to render: {error:#}"))
}

/// What each backend must show: the JSON factory called with the configuration in the
/// language's own string-literal spelling.
fn expected_call(language: &str) -> &'static str {
    match language {
        "python" => {
            r#"create_client_from_json("{\"api_key\":\"your-api-key\",\"budget\":{\"enforcement\":\"hard\",\"global_limit\":0.0}}")"#
        }
        "go" => {
            r#"CreateClientFromJSON(`{"api_key":"your-api-key","budget":{"enforcement":"hard","global_limit":0.0}}`)"#
        }
        "node" | "wasm" => {
            r#"createClientFromJson("{\"api_key\":\"your-api-key\",\"budget\":{\"enforcement\":\"hard\",\"global_limit\":0.0}}")"#
        }
        "java" => {
            r#"createClientFromJson("{\"api_key\":\"your-api-key\",\"budget\":{\"enforcement\":\"hard\",\"global_limit\":0.0}}")"#
        }
        "csharp" => {
            r#"CreateClientFromJson("{\"api_key\":\"your-api-key\",\"budget\":{\"enforcement\":\"hard\",\"global_limit\":0.0}}")"#
        }
        "ruby" => {
            r#"create_client_from_json('{"api_key":"your-api-key","budget":{"enforcement":"hard","global_limit":0.0}}')"#
        }
        "php" => {
            r#"::createClientFromJson('{"api_key":"your-api-key","budget":{"enforcement":"hard","global_limit":0.0}}')"#
        }
        "elixir" => {
            r#"create_client_from_json("{\"api_key\":\"your-api-key\",\"budget\":{\"enforcement\":\"hard\",\"global_limit\":0.0}}")"#
        }
        "rust" => {
            r##"create_client_from_json(r#"{"api_key":"your-api-key","budget":{"enforcement":"hard","global_limit":0.0}}"#)"##
        }
        other => panic!("no expectation for {other}"),
    }
}

/// The credential read the plain factory call needs and the JSON call must not carry: the
/// configuration already holds the credential, so reading one more from the environment would
/// document a variable the snippet never uses.
fn credential_read(language: &str) -> Option<&'static str> {
    match language {
        "python" => Some("os.environ"),
        "java" => Some("System.getenv"),
        "csharp" => Some("GetEnvironmentVariable"),
        "ruby" => Some("ENV.fetch"),
        "php" => Some("getenv("),
        "elixir" => Some("System.fetch_env!"),
        _ => None,
    }
}

#[test]
fn every_backend_builds_the_client_from_the_fixtures_configuration() {
    let (e2e, resolved) = config(true);
    for (language, _, from_json_factory) in LANGUAGES {
        let body = render(language, &fixture(true), &e2e, &resolved);
        assert!(
            body.contains(expected_call(language)),
            "{language} snippet does not construct the client from the fixture's client_config:\n{body}"
        );
        let plain = LANGUAGES
            .iter()
            .find(|entry| entry.0 == language)
            .expect("language row")
            .1;
        assert!(
            !body.contains(&format!("{plain}(")),
            "{language} snippet still calls the plain `{plain}` factory:\n{body}"
        );
        if let Some(read) = credential_read(language) {
            assert!(
                !body.contains(read),
                "{language} snippet reads a credential the configuration already carries:\n{body}"
            );
        }
        if language == "python" {
            assert!(
                !body.contains("import os"),
                "the credential read is gone, so its import must go with it:\n{body}"
            );
        }
        assert!(
            body.contains(from_json_factory),
            "{language} snippet never names `{from_json_factory}`:\n{body}"
        );
    }
}

/// The negative control for the table above: the same backends, given a fixture that declares
/// no `client_config`, keep their plain factory call, so the table cannot be passing because
/// every snippet happens to mention the JSON factory.
#[test]
fn a_fixture_without_client_config_keeps_the_plain_factory_call() {
    let (e2e, resolved) = config(true);
    for (language, plain, from_json_factory) in LANGUAGES {
        let body = render(language, &fixture(false), &e2e, &resolved);
        assert!(
            !body.contains(from_json_factory),
            "{language} snippet used the JSON factory without a client_config:\n{body}"
        );
        assert!(
            body.contains(&format!("{plain}(")),
            "{language} snippet lost its plain `{plain}` factory call:\n{body}"
        );
    }
}

#[test]
fn every_backend_that_renders_client_config_is_exercised_by_the_table() {
    let exercised: Vec<&str> = LANGUAGES.iter().map(|entry| entry.0).collect();
    for generator in all_generators() {
        assert_eq!(
            generator.renders_client_config(),
            exercised.contains(&generator.language_name()),
            "`{}` claims or disclaims client_config rendering without a row in LANGUAGES",
            generator.language_name()
        );
    }
}
