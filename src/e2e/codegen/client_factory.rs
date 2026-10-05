//! Shared resolution of `client_factory` construction arguments.
//!
//! Bindings whose API hangs off a client object are generated through a factory call
//! of the shape `factory(<credential>, <base URL>, <trailing args…>)`. The first two
//! slots are the generator's business — it owns the credential expression and knows
//! whether a mock server is in play — but everything after them is project-specific,
//! so it comes from configuration (`client_factory_trailing_args`) or, for a single
//! documentation snippet, from that fixture's `docs.client`.
//!
//! Every entry point here takes the fixture's documentation client as an explicit
//! `Option`, never reading it off the fixture itself. A renderer shared between the
//! docs path and the executable e2e suite therefore has to name the docs override at
//! its call site, and the e2e call site names `None` — the docs endpoint cannot reach
//! a real test by omission.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Fixture, FixtureDocsClient};

/// Verbatim argument expressions to emit after the credential and base-URL slots of a
/// `client_factory` call, resolved most-specific source first:
///
/// 1. `docs_client`'s own `args.<language>` — documentation snippets only,
/// 2. `[e2e.calls.<name>.overrides.<language>] client_factory_trailing_args`,
/// 3. `[e2e.call.overrides.<language>] client_factory_trailing_args`,
/// 4. `fallback`.
///
/// `fallback` is the list the caller hardcoded before this hook was wired up. Passing
/// it keeps every project that has not configured the override rendering byte-for-byte
/// what it renders today, so adopting the hook is not a breaking change.
pub fn trailing_args(
    docs_client: Option<&FixtureDocsClient>,
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    language: &str,
    fallback: &[&str],
) -> Vec<String> {
    if let Some(args) = docs_client.and_then(|client| client.args_for(language)) {
        return args.to_vec();
    }
    call_config
        .overrides
        .get(language)
        .map(|overrides| overrides.client_factory_trailing_args.clone())
        .filter(|args| !args.is_empty())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(language)
                .map(|overrides| overrides.client_factory_trailing_args.clone())
                .filter(|args| !args.is_empty())
        })
        .unwrap_or_else(|| fallback.iter().map(|arg| (*arg).to_string()).collect())
}

/// The base URL a documentation snippet constructs its client with, or `None` when the
/// fixture documents no particular endpoint and the generator should fall back to its
/// usual choice (the mock server, or nothing).
pub fn docs_base_url(docs_client: Option<&FixtureDocsClient>) -> Option<&str> {
    docs_client.and_then(|client| client.base_url.as_deref())
}

/// The JSON-string client factory `language` constructs a configured client with, resolved
/// the way `client_factory` is: the resolved call's override first, then the default
/// `[e2e.call]` override.
pub fn from_json_factory<'a>(
    e2e_config: &'a E2eConfig,
    call_config: &'a CallConfig,
    language: &str,
) -> Option<&'a str> {
    call_config
        .overrides
        .get(language)
        .and_then(|overrides| overrides.client_factory_from_json.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(language)
                .and_then(|overrides| overrides.client_factory_from_json.as_deref())
        })
}

/// A fixture's `docs.client.config` as the compact JSON text a snippet embeds as a string
/// literal. Object keys come out sorted, so the literal is stable across regenerations.
pub fn client_config_json(fixture: &Fixture) -> Option<String> {
    fixture.docs_client()?.config.as_ref().map(serde_json::Value::to_string)
}

/// The factory call a snippet constructs its client with for `language`, when the fixture
/// declares a `docs.client.config` and the language configures a JSON factory: the factory name
/// and the JSON text to pass it as a string literal. `None` means the plain
/// `client_factory` call applies.
///
/// A fixture with `docs.client.config` whose language has no JSON factory never reaches a
/// renderer; the snippet driver excludes it first (see [`client_config_exclusion`]).
pub fn client_config_call<'a>(
    fixture: &Fixture,
    e2e_config: &'a E2eConfig,
    call_config: &'a CallConfig,
    language: &str,
) -> Option<(&'a str, String)> {
    let json = client_config_json(fixture)?;
    Some((from_json_factory(e2e_config, call_config, language)?, json))
}

/// The name a JavaScript/TypeScript snippet imports and the statement that constructs its
/// client: the JSON factory when the fixture declares `docs.client.config`, otherwise the plain
/// `factory` with the documentation credential.
pub fn js_client_construction(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    language: &str,
    factory: &str,
) -> (String, String) {
    match client_config_call(fixture, e2e_config, call_config, language) {
        Some((from_json_factory, json)) => (
            from_json_factory.to_string(),
            format!(
                "const client = {from_json_factory}(\"{}\");",
                crate::e2e::escape::escape_js(&json)
            ),
        ),
        None => (
            factory.to_string(),
            format!("const client = {factory}(\"your-api-key\");"),
        ),
    }
}

/// Why a documentation snippet for `fixture` cannot be rendered for `language`, when the
/// fixture declares a `docs.client.config` the language has no way to apply.
///
/// Emitting the plain `client_factory` snippet instead would document a client with none
/// of the configuration the fixture exists to show.
pub fn client_config_exclusion(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    renders_client_config: bool,
) -> Option<String> {
    client_config_json(fixture)?;
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if !renders_client_config {
        return Some(format!(
            "fixture declares `docs.client.config`, which the `{language}` snippet generator cannot render"
        ));
    }
    if from_json_factory(e2e_config, call_config, language).is_none() {
        return Some(format!(
            "fixture declares `docs.client.config` but `[crates.e2e.call.overrides.{language}] client_factory_from_json` is not configured"
        ));
    }
    None
}

/// Fails when a fixture declares `docs.client.config` but the rendered snippet never calls the
/// language's JSON factory.
///
/// A backend resolves its client factory from several places (the call's own override, the
/// file-level one, a language-specific key), and not all of them agree with
/// [`client_config_exclusion`]. Checking the rendered text instead of re-deriving that
/// resolution is what keeps a backend that quietly falls back to the plain client from
/// publishing a snippet that documents none of the configuration. ~keep
pub fn ensure_client_config_applied(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    body: &str,
) -> anyhow::Result<()> {
    if client_config_json(fixture).is_none() {
        return Ok(());
    }
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let Some(factory) = from_json_factory(e2e_config, call_config, language) else {
        return Ok(());
    };
    // Generators spell the same configured name in their language's casing
    // (`CreateClientFromJSON`, `createClientFromJson`, `create_client_from_json`). ~keep
    let normalize = |text: &str| text.to_lowercase().replace('_', "");
    if normalize(body).contains(&normalize(factory)) {
        return Ok(());
    }
    anyhow::bail!(
        "fixture declares `docs.client.config`, but the rendered `{language}` snippet never calls `{factory}`: \
         its call resolves no `client_factory` to configure"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::CallOverride;

    const FALLBACK: [&str; 3] = ["None", "None", "None"];

    fn config_with_trailing_args(args: &[&str]) -> E2eConfig {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "rust".into(),
            CallOverride {
                client_factory_trailing_args: args.iter().map(|arg| (*arg).to_string()).collect(),
                ..CallOverride::default()
            },
        );
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    fn docs_client() -> FixtureDocsClient {
        FixtureDocsClient {
            base_url: Some("https://llm.internal.example.com/v1".into()),
            args: [("rust".to_string(), vec!["Some(60)".to_string(), "None".to_string()])]
                .into_iter()
                .collect(),
            ..FixtureDocsClient::default()
        }
    }

    #[test]
    fn unconfigured_projects_keep_the_argument_list_the_generator_hardcoded() {
        let e2e_config = E2eConfig::default();
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["None", "None", "None"],
            "an absent override must not shorten the factory's argument list"
        );
    }

    #[test]
    fn file_level_override_replaces_the_fallback() {
        let e2e_config = config_with_trailing_args(&["Some(30)", "Some(5)"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["Some(30)", "Some(5)"]
        );
    }

    #[test]
    fn override_is_scoped_to_its_own_language() {
        let e2e_config = config_with_trailing_args(&["Some(30)"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "java", &["null"]),
            vec!["null"],
            "a rust override must not leak into the java factory call"
        );
    }

    #[test]
    fn named_call_override_wins_over_the_file_level_one() {
        let e2e_config = config_with_trailing_args(&["file"]);
        let mut named = CallConfig::default();
        named.overrides.insert(
            "rust".into(),
            CallOverride {
                client_factory_trailing_args: vec!["call".into()],
                ..CallOverride::default()
            },
        );
        assert_eq!(
            trailing_args(None, &e2e_config, &named, "rust", &FALLBACK),
            vec!["call"]
        );
    }

    #[test]
    fn named_call_without_its_own_list_inherits_the_file_level_one() {
        let e2e_config = config_with_trailing_args(&["file"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &CallConfig::default(), "rust", &FALLBACK),
            vec!["file"],
            "a named call that omits the key must not fall through to the fallback"
        );
    }

    #[test]
    fn fixture_docs_client_outranks_every_configured_list() {
        let client = docs_client();
        let e2e_config = config_with_trailing_args(&["file"]);
        assert_eq!(
            trailing_args(Some(&client), &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["Some(60)", "None"]
        );
        assert_eq!(
            trailing_args(Some(&client), &e2e_config, &e2e_config.call, "java", &["null"]),
            vec!["null"],
            "a language the fixture says nothing about keeps the configured default"
        );
    }

    #[test]
    fn a_caller_that_declares_no_docs_client_reads_no_base_url() {
        assert_eq!(docs_base_url(None), None);
        assert_eq!(
            docs_base_url(Some(&docs_client())),
            Some("https://llm.internal.example.com/v1")
        );
        assert_eq!(
            docs_base_url(Some(&FixtureDocsClient::default())),
            None,
            "a docs client that only overrides trailing args must not invent an endpoint"
        );
    }

    fn config_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "enforced",
            "description": "a configured client",
            "docs": {
                "topic": "budget",
                "client": {"config": {"budget": {"global_limit": 0.0, "enforcement": "hard"}, "api_key": "k"}},
            },
        }))
        .expect("fixture parses")
    }

    fn config_with_factories(from_json: Option<&str>, plain: Option<&str>) -> E2eConfig {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory: plain.map(str::to_string),
                client_factory_from_json: from_json.map(str::to_string),
                ..CallOverride::default()
            },
        );
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    #[test]
    fn client_config_serializes_compactly_with_sorted_keys() {
        assert_eq!(
            client_config_json(&config_fixture()).as_deref(),
            Some(r#"{"api_key":"k","budget":{"enforcement":"hard","global_limit":0.0}}"#)
        );
        assert_eq!(client_config_json(&Fixture::default()), None);
    }

    #[test]
    fn from_json_factory_prefers_the_call_override_then_the_file_level_one() {
        let e2e_config = config_with_factories(Some("FromJson"), Some("Plain"));
        assert_eq!(
            from_json_factory(&e2e_config, &CallConfig::default(), "go"),
            Some("FromJson"),
            "a named call without its own override inherits the file-level factory"
        );
        let mut named = CallConfig::default();
        named.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory_from_json: Some("CallFromJson".into()),
                ..CallOverride::default()
            },
        );
        assert_eq!(from_json_factory(&e2e_config, &named, "go"), Some("CallFromJson"));
        assert_eq!(
            from_json_factory(&e2e_config, &named, "java"),
            None,
            "a factory configured for one language must not leak into another"
        );
    }

    #[test]
    fn a_fixture_without_client_config_never_selects_the_json_factory() {
        let e2e_config = config_with_factories(Some("FromJson"), Some("Plain"));
        assert!(client_config_call(&Fixture::default(), &e2e_config, &e2e_config.call, "go").is_none());
        let (factory, json) =
            client_config_call(&config_fixture(), &e2e_config, &e2e_config.call, "go").expect("config call");
        assert_eq!(factory, "FromJson");
        assert!(json.starts_with('{') && json.contains("\"enforcement\":\"hard\""));
    }

    #[test]
    fn a_language_that_cannot_apply_client_config_is_excluded_with_a_reason() {
        let fixture = config_fixture();
        let configured = config_with_factories(Some("FromJson"), Some("Plain"));
        assert_eq!(client_config_exclusion(&fixture, &configured, "go", true), None);
        assert_eq!(
            client_config_exclusion(&Fixture::default(), &E2eConfig::default(), "go", false),
            None,
            "a fixture without client_config is never excluded, whatever the language supports"
        );
        let unsupported = client_config_exclusion(&fixture, &configured, "go", false).expect("excluded");
        assert!(unsupported.contains("cannot render"), "{unsupported}");
        let unconfigured = config_with_factories(None, Some("Plain"));
        let reason = client_config_exclusion(&fixture, &unconfigured, "go", true).expect("excluded");
        assert!(reason.contains("client_factory_from_json"), "{reason}");
    }

    #[test]
    fn a_snippet_that_never_calls_the_json_factory_is_rejected() {
        let fixture = config_fixture();
        let e2e_config = config_with_factories(Some("CreateClientFromJSON"), Some("CreateClient"));
        assert!(
            ensure_client_config_applied(
                &fixture,
                &e2e_config,
                "go",
                "client, err := pkg.CreateClient(\"your-api-key\", nil, nil, nil, nil)"
            )
            .is_err(),
            "the plain factory must not satisfy a client_config fixture"
        );
        assert!(ensure_client_config_applied(&fixture, &e2e_config, "go", "pkg.CreateClientFromJSON(`{}`)").is_ok());
        assert!(
            ensure_client_config_applied(&fixture, &e2e_config, "go", "pkg.create_client_from_json(x)").is_ok(),
            "a generator may respell the configured name in its own casing"
        );
        assert!(
            ensure_client_config_applied(&Fixture::default(), &e2e_config, "go", "pkg.CreateClient()").is_ok(),
            "a fixture without client_config has nothing to apply"
        );
    }
}
