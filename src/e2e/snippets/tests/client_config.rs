//! `docs.client.config` through the snippet driver: which languages get a snippet that builds
//! the client from the configuration, and what the ledger records for the ones that cannot.
//!
//! Driven through [`generate_snippet_report_with_extensions`] rather than the per-language
//! renderers, because the property under test is the driver's: a language that cannot apply the
//! configuration must surface as a recorded gap with a reason, and never as a plain-client
//! snippet that looks like the real thing. ~keep

use super::*;
use crate::core::config::NewAlefConfig;

fn config(python_override: &str) -> (E2eConfig, ResolvedCrateConfig) {
    let cfg_str = format!(
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
args = [{{ name = "prompt", field = "prompt", type = "string" }}]
[crates.e2e.call.overrides.python]
{python_override}
[crates.e2e.call.overrides.go]
client_factory = "CreateClient"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&cfg_str).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn fixture(coverage_exceptions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "budget_enforced",
        "description": "a configured client",
        "input": {"prompt": "Hello"},
        "docs": {
            "topic": "budget",
            "client": {"config": {"api_key": "your-api-key", "budget": {"enforcement": "hard", "global_limit": 0.0}}},
            "coverage_exceptions": coverage_exceptions,
        },
    }))
    .expect("fixture parses")
}

fn report(python_override: &str, languages: &[&str], fixture: Fixture) -> SnippetGenerationReport {
    let (e2e, crate_config) = config(python_override);
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };
    let languages: Vec<String> = languages.iter().map(|language| (*language).to_string()).collect();
    generate_snippet_report_with_extensions(&[fixture], &languages, &snippet_config, &context, &[])
        .expect("snippet report renders")
}

fn missing_reason(report: &SnippetGenerationReport, language: &str) -> String {
    report
        .coverage
        .missing
        .iter()
        .find(|entry| entry.key.language == language)
        .unwrap_or_else(|| panic!("no missing entry for {language}: {:?}", report.coverage.missing))
        .reason
        .clone()
}

const PYTHON_WITH_JSON_FACTORY: &str =
    "client_factory = \"create_client\"\nclient_factory_from_json = \"create_client_from_json\"";

#[test]
fn a_configured_language_gets_a_snippet_that_builds_the_client_from_the_configuration() {
    let report = report(PYTHON_WITH_JSON_FACTORY, &["python"], fixture(serde_json::json!({})));

    assert!(report.coverage.missing.is_empty(), "{:?}", report.coverage.missing);
    let content = &report.snippets.first().expect("one snippet").file.content;
    assert!(
        content.contains("create_client_from_json(\"{\\\"api_key\\\":\\\"your-api-key\\\""),
        "{content}"
    );
}

#[test]
fn a_language_without_a_json_factory_is_a_recorded_gap_not_a_plain_client_snippet() {
    let report = report(
        PYTHON_WITH_JSON_FACTORY,
        &["python", "go"],
        fixture(serde_json::json!({})),
    );

    assert_eq!(report.snippets.len(), 1, "go must not render a snippet");
    assert_eq!(report.snippets[0].language, "python");
    let reason = missing_reason(&report, "go");
    assert!(reason.contains("client_factory_from_json"), "{reason}");
}

#[test]
fn a_language_whose_generator_cannot_render_client_config_is_a_recorded_gap() {
    let report = report(
        PYTHON_WITH_JSON_FACTORY,
        &["python", "zig"],
        fixture(serde_json::json!({})),
    );

    assert_eq!(report.snippets.len(), 1);
    let reason = missing_reason(&report, "zig");
    assert!(reason.contains("cannot render"), "{reason}");
}

#[test]
fn a_documented_exception_turns_the_gap_into_a_recorded_exception() {
    let exceptions = serde_json::json!({
        "go": {"reason": "no JSON factory in this binding", "documentation": "docs/go.md"}
    });
    let report = report(PYTHON_WITH_JSON_FACTORY, &["python", "go"], fixture(exceptions));

    assert!(report.coverage.missing.is_empty(), "{:?}", report.coverage.missing);
    assert_eq!(report.coverage.documented_exceptions.len(), 1);
    assert_eq!(report.coverage.documented_exceptions[0].key.language, "go");
}

/// The backend resolves no client for this call (only the JSON factory is configured), so it
/// renders the plain function call. That must be caught from the rendered text: the gate that
/// runs before rendering sees a configured JSON factory and cannot know the renderer will
/// ignore it.
#[test]
fn a_renderer_that_ignores_the_configuration_is_caught_after_rendering() {
    let report = report(
        "client_factory_from_json = \"create_client_from_json\"",
        &["python"],
        fixture(serde_json::json!({})),
    );

    assert!(report.snippets.is_empty(), "no snippet may be published");
    let reason = missing_reason(&report, "python");
    assert!(reason.contains("never calls `create_client_from_json`"), "{reason}");
}

/// Negative control: a fixture without `docs.client.config` renders through the plain factory
/// whether or not the language configures a JSON one.
#[test]
fn a_fixture_without_client_config_is_unaffected_by_a_configured_json_factory() {
    let mut plain = fixture(serde_json::json!({}));
    plain.docs.as_mut().expect("docs").client = None;
    let report = report(PYTHON_WITH_JSON_FACTORY, &["python", "go"], plain);

    assert!(report.coverage.missing.is_empty(), "{:?}", report.coverage.missing);
    assert_eq!(report.snippets.len(), 2);
    for snippet in &report.snippets {
        assert!(
            !snippet.file.content.contains("_from_json") && !snippet.file.content.contains("FromJSON"),
            "{}",
            snippet.file.content
        );
    }
}

#[test]
fn the_executable_suite_excludes_a_fixture_that_declares_client_config() {
    let (e2e, _) = config(PYTHON_WITH_JSON_FACTORY);
    assert_eq!(
        crate::e2e::codegen::fixture_inclusion(&fixture(serde_json::json!({})), "python", &e2e),
        crate::e2e::codegen::InclusionDecision::Exclude("client config is not rendered by the executable suite")
    );
    let mut plain = fixture(serde_json::json!({}));
    plain.docs.as_mut().expect("docs").client = None;
    assert!(crate::e2e::codegen::fixture_inclusion(&plain, "python", &e2e).is_included());
}
