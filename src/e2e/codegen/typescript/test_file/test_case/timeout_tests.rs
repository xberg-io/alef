use super::*;
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Assertion;

fn with_timeout(mut fixture: Fixture, timeout_ms: u64) -> Fixture {
    fixture
        .tags
        .push(format!("{}{timeout_ms}", Fixture::TEST_TIMEOUT_TAG_PREFIX));
    fixture
}

fn render_timeout(fixture: &Fixture) -> String {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "probe".to_string(),
            module: "probe".to_string(),
            returns_void: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut out = String::new();
    let mut referenced_enums = std::collections::BTreeSet::new();
    render_test_case(
        &mut out,
        fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &[],
        &[],
        &[],
        "",
        &crate::core::config::ResolvedCrateConfig::default(),
        &mut referenced_enums,
        &[],
    );
    out
}

fn renderable_fixture() -> Fixture {
    Fixture {
        id: "bounded_call".to_string(),
        description: "bounded call".to_string(),
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn fixture_test_timeout_keeps_the_existing_default() {
    let fixture = renderable_fixture();
    assert_eq!(fixture_timeout_ms(&fixture), 30_000);
    assert!(render_timeout(&fixture).contains("}, 30000);"));
}

#[test]
fn fixture_test_timeout_overrides_every_inferred_timeout_class() {
    for fixture in [
        Fixture::default(),
        Fixture {
            tags: vec!["embeddings".to_string()],
            ..Default::default()
        },
        Fixture {
            input: serde_json::json!({"config": {"language": "perl"}}),
            ..Default::default()
        },
    ] {
        let fixture = with_timeout(fixture, 60_000);
        assert_eq!(fixture_timeout_ms(&fixture), 60_000);
    }
}

#[test]
fn fixture_test_timeout_is_emitted_for_only_that_test() {
    let fixture = with_timeout(renderable_fixture(), 60_000);
    let ordinary = renderable_fixture();

    assert!(render_timeout(&fixture).contains("}, 60000);"));
    assert!(render_timeout(&ordinary).contains("}, 30000);"));
}
