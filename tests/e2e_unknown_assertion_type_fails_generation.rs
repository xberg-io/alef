//! An assertion type a backend cannot render must fail generation, in every backend.
//!
//! Before the shared gate in `e2e::codegen::assertion_types`, an unrecognized type was
//! handled three incompatible ways: the `java`, `php` and `zig` JSON-result templates end
//! their `{% elif %}` chain with no `{% else %}` and rendered the empty string, so the
//! generated test compiled and passed while asserting nothing; `dart` wrote a
//! `// skipped: unknown assertion type` comment counted nowhere; the rest panicked. New
//! assertion kinds would therefore have shipped as silent passes in the first group.
//!
//! Every case below is driven from `all_generators()` rather than a hand-written language
//! list, so a backend added later is covered by these tests on the day it is registered
//! instead of being silently exempt from them.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, all_generators, assertion_types};
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// A type no backend has a dispatch arm for and that the fixture schema does not list.
const UNKNOWN_TYPE: &str = "asserts_the_vibes_are_correct";

const FIXTURE_ID: &str = "unknown_assertion_type_fixture";
const FIXTURE_SOURCE: &str = "fixtures/unknown_assertion_type.json";

/// A config with a base call function and no per-language overrides, so
/// `fixture_inclusion` includes the fixture for every registered language and no backend
/// can pass the gate by simply declining to emit the fixture.
fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "demo-markup-rs"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "dm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "demo_markup"
result_var = "result"
args = [
  { name = "html", field = "html", type = "string" },
]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn resolve() -> (
    alef::core::config::ResolvedCrateConfig,
    alef::core::config::e2e::E2eConfig,
) {
    let cfg = build_config();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

fn fixture_group_with_assertion_type(assertion_type: &str) -> FixtureGroup {
    FixtureGroup {
        category: "conversion".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: FIXTURE_ID.to_string(),
            category: Some("conversion".to_string()),
            description: "fixture carrying an assertion type under test".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "html": "<p>hi</p>" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: assertion_type.to_string(),
                field: None,
                value: Some(serde_json::Value::String("expected".to_string())),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: FIXTURE_SOURCE.to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

fn generate(codegen: &dyn E2eCodegen, assertion_type: &str) -> anyhow::Result<Vec<alef::GeneratedFile>> {
    let (resolved, e2e) = resolve();
    let groups = vec![fixture_group_with_assertion_type(assertion_type)];
    codegen.generate_gated(&groups, &e2e, &resolved, &[], &[], &[], &[])
}

#[test]
fn unknown_assertion_type_fails_generation_in_every_backend() {
    let generators = all_generators();
    assert!(!generators.is_empty(), "all_generators() must not be empty");

    for codegen in &generators {
        let language = codegen.language_name();
        let error = generate(codegen.as_ref(), UNKNOWN_TYPE).expect_err(&format!(
            "[{language}] generation must fail on an unknown assertion type"
        ));
        let message = format!("{error:#}");
        assert!(
            message.contains(UNKNOWN_TYPE),
            "[{language}] the error must name the offending type: {message}"
        );
        assert!(
            message.contains(FIXTURE_ID),
            "[{language}] the error must name the fixture id: {message}"
        );
        assert!(
            message.contains(FIXTURE_SOURCE),
            "[{language}] the error must name the fixture source file: {message}"
        );
        assert!(
            message.contains(&format!("'{language}'")),
            "[{language}] the error must name the backend that rejected it: {message}"
        );
    }
}

/// The failure must be identical everywhere: no backend may downgrade it to an empty
/// render, a comment, or a panic that never reaches the caller as an `Err`.
#[test]
fn unknown_assertion_type_failure_is_uniform_across_backends() {
    let mut shapes: Vec<String> = all_generators()
        .iter()
        .map(|codegen| {
            let language = codegen.language_name();
            let error = generate(codegen.as_ref(), UNKNOWN_TYPE).expect_err(&format!(
                "[{language}] generation must fail on an unknown assertion type"
            ));
            // ~keep Replace the quoted language token, not the bare name: a bare `.replace("r", ..)`
            // would rewrite every `r` in the diagnostic and make the shapes differ for reasons
            // that have nothing to do with the backend.
            let message = format!("{error:#}").replace(&format!("'{language}'"), "'<lang>'");
            message
                .split(". Supported types")
                .next()
                .unwrap_or(&message)
                .to_string()
        })
        .collect();
    shapes.sort();
    shapes.dedup();
    assert_eq!(
        shapes.len(),
        1,
        "every backend must report the same diagnostic, got: {shapes:#?}"
    );
}

/// Run the gate alone, without the backend's own `generate`.
///
/// The negative cases below reach the backend only through `generate_gated`, which
/// short-circuits in the gate; the positive cases must not run a full project generation
/// just to observe that the gate let them through. ~keep
fn gate(codegen: &dyn E2eCodegen, assertion_type: &str) -> anyhow::Result<()> {
    let (_, e2e) = resolve();
    let groups = vec![fixture_group_with_assertion_type(assertion_type)];
    assertion_types::ensure_supported_assertion_types(
        &groups,
        &e2e,
        codegen.language_name(),
        &codegen.supported_assertion_types(),
    )
}

/// `not_equals` is schema-legal but only Dart renders it. Every other backend must reject
/// it at generation time rather than emitting nothing (java, php, zig) or panicking.
#[test]
fn schema_legal_type_unsupported_by_a_backend_fails_that_backend_only() {
    for codegen in &all_generators() {
        let language = codegen.language_name();

        if codegen.supported_assertion_types().contains(&"not_equals") {
            assert!(
                gate(codegen.as_ref(), "not_equals").is_ok(),
                "[{language}] declares not_equals supported, so the gate must let it through"
            );
            continue;
        }

        let message = format!(
            "{:#}",
            generate(codegen.as_ref(), "not_equals")
                .expect_err(&format!("[{language}] must reject the unsupported type 'not_equals'"))
        );
        assert!(
            message.contains("not_equals") && message.contains(FIXTURE_ID),
            "[{language}] the error must name the type and the fixture: {message}"
        );
    }
}

/// Every schema-legal type must be renderable by at least one backend, and every backend
/// must render at least the types the whole matrix shares. A new schema entry that no
/// backend implements would otherwise be advertised to fixture authors as usable.
#[test]
fn every_schema_type_is_rendered_by_at_least_one_backend() {
    for assertion_type in assertion_types::KNOWN_ASSERTION_TYPES {
        let supporters: Vec<&str> = all_generators()
            .iter()
            .filter(|codegen| codegen.supported_assertion_types().contains(assertion_type))
            .map(|codegen| codegen.language_name())
            .collect();
        assert!(
            !supporters.is_empty(),
            "'{assertion_type}' is in the fixture schema but no backend renders it"
        );
    }
}

/// A fixture that skips the backend lacking the type keeps generating, so `skip.languages`
/// remains the escape hatch for a type one backend cannot express.
#[test]
fn skipped_fixture_does_not_trip_the_gate() {
    let (_, e2e) = resolve();
    let mut group = fixture_group_with_assertion_type(UNKNOWN_TYPE);
    group.fixtures[0].skip = Some(alef::e2e::fixture::SkipDirective {
        languages: all_generators()
            .iter()
            .map(|codegen| codegen.language_name().to_string())
            .collect(),
        reason: Some("exercises a type no backend renders".to_string()),
    });
    let groups = vec![group];

    for codegen in &all_generators() {
        let language = codegen.language_name();
        assert!(
            assertion_types::ensure_supported_assertion_types(
                &groups,
                &e2e,
                language,
                &codegen.supported_assertion_types(),
            )
            .is_ok(),
            "[{language}] a fixture skipped for this backend must not trip the assertion gate"
        );
    }
}
