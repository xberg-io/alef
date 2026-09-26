//! Cross-cutting tests for the `mock.*` gate -- see `model` and `snippets` for grammar- and
//! table-scoped tests. These specifically prove the shared machinery cannot silently green a
//! backend that has no capture, per `docs/design/assertion-kinds.md`'s prohibition.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::inert_example::{self, InertCause};
use crate::e2e::codegen::{fail_on_unavailable_field_markers, take_skip_records};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, AssertionSkip, AssertionSkipDirective, AssertionSkipKind, Fixture, FixtureGroup};

use super::{ensure_capture_declared, snippets};

/// A real generator language name with no `mock.*` capture-table entry, for every test below.
const UNWIRED_LANGUAGE: &str = "go";

fn mock_total_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        source: format!("fixtures/{id}.json"),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("mock.requests.total".to_string()),
            value: Some(serde_json::json!(3)),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// An `E2eConfig` under which [`crate::e2e::codegen::fixture_inclusion`] includes a bare fixture
/// with no `call` override -- otherwise every scenario below would vacuously pass by having its
/// only fixture excluded before the gate ever inspects it.
fn included_e2e_config() -> E2eConfig {
    let mut config = E2eConfig::default();
    config.call.function = "sample_call".to_string();
    config
}

fn one_group(fixture: Fixture) -> [FixtureGroup; 1] {
    [FixtureGroup {
        category: "mock".to_string(),
        fixtures: vec![fixture],
    }]
}

/// `UNWIRED_LANGUAGE` really has no capture-table entry -- the whole scenario below is meaningless
/// if this drifts to a language `snippets::mock_capture` later grows an arm for. ~keep
#[test]
fn the_scenario_language_really_has_no_capture() {
    assert_eq!(snippets::mock_capture(UNWIRED_LANGUAGE), Err(snippets::MockCaptureGap));
}

/// The deliberate-failure test (written first, per the task contract): a fixture whose only
/// assertion is `mock.requests.total`, rendered for a backend with no capture-table entry, must
/// not be able to slip through as a silent, green no-op -- see `docs/design/assertion-kinds.md`'s
/// prohibition. All four checks below are load-bearing; a change that breaks any one of them
/// reopens the gap `mock_assertions` exists to close. ~keep
#[test]
fn a_backend_with_no_capture_cannot_silently_green_a_mock_assertion() {
    let _ = take_skip_records();
    let _ = inert_example::take_inert_examples();

    let fixture = mock_total_fixture("mock_total_no_capture");

    // (a) the marker a backend's own call site renders, once a later wave wires the interception,
    // carries the registered wording.
    let body = format!(
        "    // skipped: {}\n",
        FieldSkip::MockCaptureNotSupported.message("mock.requests.total")
    );
    assert!(
        body.contains("requires a capture this backend does not emit"),
        "got: {body}"
    );

    // (b) the field-skip funnel recognises it.
    assert_eq!(
        FieldSkip::extract_classified(&body),
        Some(("mock.requests.total", FieldSkip::MockCaptureNotSupported))
    );

    // (c) the inert-example verdict is `AwaitedOrLimited`, never `RenderedNothing` -- the marker
    // must reach the ledger first, exactly like every production call site.
    fail_on_unavailable_field_markers(&body, UNWIRED_LANGUAGE, &fixture.id, &fixture.assertions);
    let refusal = inert_example::inert_verdict(&body, UNWIRED_LANGUAGE, &fixture.id, &fixture.assertions)
        .expect("an all-markers body must be refused");
    assert_eq!(
        refusal.cause,
        InertCause::AwaitedOrLimited,
        "a GeneratorGap marker must never be misclassified as RenderedNothing"
    );

    // (d) the shared generation gate errors, naming the fixture, because the scenario language is
    // neither in the capture table nor acknowledged by the fixture's own `skip` declaration.
    let groups = one_group(fixture.clone());
    let error = ensure_capture_declared(&groups, &included_e2e_config(), UNWIRED_LANGUAGE)
        .expect_err("generation must fail for an unacknowledged, uncaptured mock.* assertion");
    let message = format!("{error:#}");
    assert!(
        message.contains(&fixture.id),
        "error must name the fixture, got: {message}"
    );
    assert!(message.contains(UNWIRED_LANGUAGE), "got: {message}");
    assert!(message.contains("mock.requests.total"), "got: {message}");

    let _ = take_skip_records();
    let _ = inert_example::take_inert_examples();
}

/// Negative control mirroring
/// `field_skip::tests::unsupported_assertion_type_wordings_stay_uncounted`: the assertion-*type*
/// funnel must not recognise the field-axis wording, even though both funnels drain onto one
/// shared skip ledger downstream. ~keep
#[test]
fn the_assertion_type_funnel_does_not_recognise_the_mock_capture_wording() {
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;

    let line = format!(
        "    // skipped: {}",
        FieldSkip::MockCaptureNotSupported.message("mock.requests.total")
    );
    assert_eq!(AssertionTypeSkip::extract_classified(&line), None, "got: {line}");
}

/// The escape hatch `ensure_capture_declared`'s own doc promises: acknowledging the gap on the
/// fixture's own assertion lets generation proceed instead of failing.
#[test]
fn an_acknowledged_gap_does_not_fail_generation() {
    let mut fixture = mock_total_fixture("mock_total_acknowledged");
    fixture.assertions[0].skip = Some(AssertionSkip::Scoped(AssertionSkipDirective {
        languages: vec![UNWIRED_LANGUAGE.to_string()],
        kind: AssertionSkipKind::LanguageLimitation,
        reason: Some("go capture not wired yet".to_string()),
    }));
    let groups = one_group(fixture);
    assert!(ensure_capture_declared(&groups, &included_e2e_config(), UNWIRED_LANGUAGE).is_ok());
}

/// An acknowledgement scoped to a DIFFERENT language must not excuse the scenario language --
/// otherwise the acknowledgement check would be a blanket bypass rather than a per-language one.
#[test]
fn an_acknowledgement_for_another_language_does_not_excuse_this_one() {
    let mut fixture = mock_total_fixture("mock_total_wrong_language_acknowledged");
    fixture.assertions[0].skip = Some(AssertionSkip::Scoped(AssertionSkipDirective {
        languages: vec!["java".to_string()],
        kind: AssertionSkipKind::LanguageLimitation,
        reason: Some("java capture not wired yet".to_string()),
    }));
    let groups = one_group(fixture);
    assert!(ensure_capture_declared(&groups, &included_e2e_config(), UNWIRED_LANGUAGE).is_err());
}

/// A capture-table language never triggers the gate, even with no acknowledgement at all -- this
/// is the "supported" path, not merely the "not tested yet" path.
#[test]
fn a_wired_language_never_needs_acknowledgement() {
    let groups = one_group(mock_total_fixture("mock_total_wired"));
    for language in ["rust", "python", "node"] {
        assert!(ensure_capture_declared(&groups, &included_e2e_config(), language).is_ok());
    }
}

/// A fixture that asserts no `mock.*` field at all must never trip the gate, whatever language is
/// asked -- the gate is scoped to the field, not to the fixture.
#[test]
fn a_fixture_without_a_mock_assertion_never_trips_the_gate() {
    let fixture = Fixture {
        id: "unrelated_fixture".to_string(),
        source: "fixtures/unrelated_fixture.json".to_string(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("metadata.title".to_string()),
            value: Some(serde_json::json!("x")),
            ..Default::default()
        }],
        ..Default::default()
    };
    let groups = one_group(fixture);
    assert!(ensure_capture_declared(&groups, &included_e2e_config(), UNWIRED_LANGUAGE).is_ok());
}
