//! Regression coverage for the `not_error` presence-assertion defect (alef #165, Elixir arm).
//!
//! `render_assertion`'s `not_error` arm unconditionally emitted `refute is_nil(result)`, even
//! when a sibling `is_empty` assertion on the same bare result declared the call's success path
//! can legitimately return nothing (`Option<T> -> None -> Elixir nil`). A fixture pairing the
//! two produced a contradictory `refute is_nil(result)` + `assert is_nil(result) or ... == ""`
//! pair that can never both pass.
//!
//! ~keep WHETHER `not_error` may assert presence is no longer decided here: it comes in as a
//! single already-resolved `not_error_may_assert_presence` boolean, computed once by
//! `not_error_presence::may_assert_presence` (shared with typescript, csharp, java, kotlin —
//! see that module's doc for why this was reinvented independently seven times). These tests
//! drive `render_assertion` — the real generator, not a hand-written mirror of it — with flags
//! produced by the real shared function, so a regression in either the shared decision or this
//! backend's use of it fails a test here.
//!
//! Lives in its own file rather than growing `assertions.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::assertions::render_assertion;
use crate::e2e::codegen::not_error_presence::may_assert_presence;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }
}

fn is_empty_assertion() -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        ..Default::default()
    }
}

fn fixture_with(assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        assertions,
        ..Default::default()
    }
}

fn render(assertion: &Assertion, not_error_may_assert_presence: bool) -> String {
    let resolver = empty_resolver();
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        &resolver,
        "SampleModule",
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        false,
        true,
        not_error_may_assert_presence,
    );
    out
}

/// The regression this file exists for: a fixture whose assertions are `not_error` +
/// `is_empty` on a bare result must not assert presence from `not_error` — that would
/// contradict `is_empty`'s `assert is_nil(result) or ...` check on the same variable and the
/// pair could never pass.
#[test]
fn not_error_paired_with_is_empty_does_not_assert_presence() {
    let fixture = fixture_with(vec![not_error_assertion(), is_empty_assertion()]);
    let may_assert = may_assert_presence(&fixture, false);
    let out = render(&not_error_assertion(), may_assert);
    assert_eq!(
        out, "",
        "not_error must render nothing when a sibling assertion exists; got: {out}"
    );
}

/// Control: a fixture whose only assertion is `not_error` on a non-`Option` result must still
/// emit a real, non-vacuous check — the guard must not silence the fallback unconditionally.
#[test]
fn not_error_as_sole_assertion_on_non_option_result_still_asserts_presence() {
    let fixture = fixture_with(vec![not_error_assertion()]);
    let may_assert = may_assert_presence(&fixture, false);
    let out = render(&not_error_assertion(), may_assert);
    assert_eq!(out, "      refute is_nil(result)\n");
}

/// New coverage this unification closes: before centralizing the decision, Elixir only
/// suppressed `not_error`'s presence assertion when a *sibling* assertion existed — a fixture
/// whose *sole* assertion was `not_error` on an `Option<T>`-returning call still got an
/// unconditional `refute is_nil(result)`, which fails every time the call's success path
/// legitimately returns `None` (Elixir `nil`). `may_assert_presence` closes that gap by also
/// consulting `result_is_option`, independent of sibling count. (The corresponding
/// `actual_result_var` underscore-prefixing — needed so `result` isn't left unreferenced when
/// this renders nothing — is covered separately in `test_case.rs`'s own tests.)
#[test]
fn not_error_as_sole_assertion_on_option_result_does_not_assert_presence() {
    let fixture = fixture_with(vec![not_error_assertion()]);
    let may_assert = may_assert_presence(&fixture, true);
    let out = render(&not_error_assertion(), may_assert);
    assert_eq!(
        out, "",
        "not_error on a bare Option<T> result must not assert non-nil even as the sole \
         assertion; got: {out}"
    );
}

/// End-to-end: rendering `not_error` then `is_empty` (the actual fixture order) produces only
/// the `is_empty` check, not both.
#[test]
fn not_error_then_is_empty_in_sequence_emits_only_is_empty() {
    let fixture = fixture_with(vec![not_error_assertion(), is_empty_assertion()]);
    let may_assert = may_assert_presence(&fixture, false);
    let mut out = String::new();
    let resolver = empty_resolver();
    for assertion in [&not_error_assertion(), &is_empty_assertion()] {
        render_assertion(
            &mut out,
            assertion,
            "result",
            &resolver,
            "SampleModule",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            true,
            may_assert,
        );
    }
    assert!(
        !out.contains("refute is_nil(result)"),
        "not_error must not assert presence alongside is_empty; got: {out}"
    );
    assert!(
        out.contains("assert result in [nil, \"\", [], %{}]"),
        "is_empty must still render its own nil/empty check; got: {out}"
    );
}
