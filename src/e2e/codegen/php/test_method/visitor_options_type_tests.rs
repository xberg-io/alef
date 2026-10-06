use super::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{CallbackAction, Fixture, VisitorSpec};
use std::collections::HashMap;

/// Regression test for alef task #86: a `visitor` fixture whose options type resolves
/// from neither `[e2e.call]` nor any `[[crates.trait_bridges]]` entry used to re-render
/// `php/test_method.jinja` with `skip_test => true` — byte-identical to the output the
/// sanctioned `skip_languages` branch emits. A config failure was therefore
/// indistinguishable from an author-declared skip, and the emitted PHPUnit suite went
/// green while exercising none of the visitor behavior it claimed. It must now fail at
/// generation time, naming the fixture and the missing options type — mirroring
/// `c/assertions.rs` and `kotlin/args.rs`, which already refuse to emit for an
/// unresolvable trait bridge.
#[test]
#[should_panic(expected = "PHP e2e generator: fixture `visitor_smoke` declares a `visitor`")]
fn visitor_fixture_without_trait_bridge_options_type_fails_loudly_instead_of_emitting_a_skip() {
    let fixture = Fixture {
        id: "visitor_smoke".into(),
        description: "Visitor smoke".into(),
        visitor: Some(VisitorSpec {
            callbacks: [("visit_element".to_string(), CallbackAction::Skip)]
                .into_iter()
                .collect(),
        }),
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "convert".into();
    e2e_config.call.result_var = "result".into();

    // No `[[crates.trait_bridges]]` entries declared — nothing supplies an `options_type`.
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut trait_bridge_imports: Vec<String> = Vec::new();
    render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "php",
        "Sample",
        "SampleClient",
        &[],
        &[],
        &[],
        &super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]),
        &HashMap::new(),
        false,
        None,
        "",
        &[],
        "",
        &config,
        &[],
        &mut trait_bridge_imports,
    );
}

fn render_php_error_method(extra: Vec<crate::e2e::fixture::Assertion>, declared: Option<&str>) -> String {
    let mut assertions = vec![crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        value: declared.map(|v| serde_json::Value::String(v.to_string())),
        ..Default::default()
    }];
    assertions.extend(extra);
    let fixture = Fixture {
        id: "rate_limited".into(),
        description: "Rejects the request".into(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parse".into();
    e2e_config.call.result_var = "result".into();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    let mut trait_bridge_imports: Vec<String> = Vec::new();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "php",
        "Sample",
        "SampleClient",
        &[],
        &[],
        &[],
        &super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]),
        &HashMap::new(),
        false,
        None,
        "",
        &[],
        "",
        &config,
        &[],
        &mut trait_bridge_imports,
    );
    out
}

/// PHP's error path renders one `expectException` / try-catch and returns, so every other
/// assertion on the fixture used to leave no trace in the generated test at all.
#[test]
fn php_equals_on_an_error_field_is_named_instead_of_dropped() {
    let out = render_php_error_method(
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".into(),
            field: Some("error.status_code".into()),
            ..Default::default()
        }],
        Some("BadRequest"),
    );

    // Positive first: the error block really rendered.
    assert!(
        out.contains("catch (\\Exception $e)"),
        "the error block must render: {out}"
    );
    assert!(
        out.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this \
                 backend"
        ),
        "{out}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "php");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated method marker-free.
#[test]
fn php_a_lone_error_assertion_renders_no_marker() {
    let out = render_php_error_method(Vec::new(), None);

    assert!(
        out.contains("$this->expectException(\\Exception::class);"),
        "the error block must render: {out}"
    );
    assert!(!out.contains("has no accessor for error field"), "{out}");
}
