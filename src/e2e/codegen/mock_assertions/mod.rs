//! Shared `mock.*` request-count virtual-field namespace for e2e test codegen (alef issue #433).
//!
//! Follows the pattern `docs/design/assertion-kinds.md` and `assertion_kinds_design.rs` establish:
//! a virtual-field namespace whose accessor reads state the generator arranges around the call,
//! with `streaming_assertions` as the working instance. `mock.*` asserts on the generated mock
//! HTTP server's own request log -- a property of the test harness, not a field of any result type
//! -- through that same pattern (`is_valid_for_result` never sees it) rather than a new assertion
//! `type` on the fixture schema.
//!
//! - [`model`] -- the field grammar (`mock.requests.total`, `mock.requests["<key>"]`).
//! - [`snippets`] -- which languages can emit the once-per-suite capture, and which cannot.
//! - [`helper`] -- renders the capture as a language-native function, through Minijinja.
//! - [`ensure_capture_declared`] -- the shared `E2eCodegen::generate_gated` gate: a backend that is
//!   neither in the capture table nor acknowledged by the assertion's own `skip` declaration fails
//!   generation naming the fixture, rather than silently falling through to the wrong oracle (the
//!   fate every backend's `render_assertion` has today, since none intercepts `mock.*` yet).

pub(crate) mod helper;
pub(crate) mod model;
pub(crate) mod snippets;

#[cfg(test)]
mod tests;

pub(crate) use model::{MOCK_RECIPE, is_mock_virtual_field};
pub(crate) use snippets::mock_capture;

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

/// Fail generation when `language` will actually render a fixture asserting a `mock.*` field, and
/// `language` neither has a capture-table entry ([`snippets::mock_capture`]) nor is acknowledged by
/// that assertion's own `skip` declaration.
///
/// Modelled on `assertion_types::ensure_supported_assertion_types` -- the one other cross-backend
/// gate `E2eCodegen::generate_gated` already runs before any per-language code is emitted -- with
/// one addition that function has no equivalent for: a fixture author can acknowledge the gap
/// per-language the same way they acknowledge any other unresolved field
/// (`"skip": { "kind": "language_limitation", "languages": ["<lang>"], "reason": "..." }`), because
/// unlike an unsupported assertion *type* this is a missing generator *capability* a future alef
/// release can add with no fixture edit -- exactly `FieldSkip::MockCaptureNotSupported`'s
/// `GeneratorGap` classification.
///
/// ~keep Intentionally unconditional -- it does not consult `ALEF_E2E_STRICT_ASSERTIONS`, matching
/// `ensure_supported_assertion_types`. Without this gate, a backend with no `mock.*` interception
/// (every backend, until a later wave wires `rust`/`python`/`node`'s own `render_assertion`) would
/// fall through to its ordinary field-availability oracle, which rejects `mock.requests.total` as
/// `FieldSkip::NotAvailableOnResultType` -- an `AuthoringGap` that blames the fixture for a gap that
/// is alef's own. Gating here, before generation, is what keeps that misattribution from ever
/// reaching a rendered file.
pub(crate) fn ensure_capture_declared(
    groups: &[FixtureGroup],
    e2e_config: &E2eConfig,
    language: &str,
) -> anyhow::Result<()> {
    if mock_capture(language).is_ok() {
        return Ok(());
    }
    let mut offenders: Vec<String> = Vec::new();
    for group in groups {
        for fixture in &group.fixtures {
            if !super::fixture_inclusion(fixture, language, e2e_config).is_included() {
                continue;
            }
            for assertion in &fixture.assertions {
                let Some(field) = assertion.field.as_deref() else {
                    continue;
                };
                if !is_mock_virtual_field(field) {
                    continue;
                }
                let acknowledged = assertion.skip.as_ref().is_some_and(|skip| skip.should_skip(language));
                if acknowledged {
                    continue;
                }
                offenders.push(format!(
                    "fixture '{}' ({}) asserts '{}'",
                    fixture.id, fixture.source, field
                ));
            }
        }
    }
    if offenders.is_empty() {
        return Ok(());
    }
    offenders.sort();
    offenders.dedup();
    anyhow::bail!(
        "e2e backend '{language}' cannot capture {} mock-request-count assertion(s): {}. \
         `mock_assertions::snippets::mock_capture` has no entry for '{language}'; add one, or \
         acknowledge the gap on the assertion with \
         \"skip\": {{ \"kind\": \"language_limitation\", \"languages\": [\"{language}\"], \
         \"reason\": \"...\" }}.",
        offenders.len(),
        offenders.join("; "),
    );
}
