//! Tests for the unavailable-field skip ledger in [`super`].
//!
//! Split out of `mod.rs` alongside `assertion_type_marker_tests`: the two inline test modules
//! were 538 of that file's 1,519 lines. `mod.rs` already carries `streaming_result_binding_tests`
//! and `working_directory_guard_tests` as sibling files, so this is the file's own established
//! shape rather than a new one.

use super::{
    SkipVerdict, fail_on_unavailable_field_markers, is_falsy_flag, skip_summary, strict_assertion_failure,
    take_skip_records,
};
use crate::e2e::fixture::{Assertion, AssertionSkip, AssertionSkipDirective, AssertionSkipKind};

/// Record `body` and return the verdicts, so a wording's *classification* can be asserted
/// directly rather than inferred from whether generation failed. ~keep
fn verdicts_for(body: &str, language: &str, assertions: &[Assertion]) -> Vec<SkipVerdict> {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(body, language, "smoke", assertions);
    take_skip_records().into_iter().map(|r| r.verdict).collect()
}

/// The strict-mode error for one rendered body, or `None` if it is generatable.
fn strict_error_for(body: &str, language: &str, fixture_id: &str, assertions: &[Assertion]) -> Option<String> {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(body, language, fixture_id, assertions);
    strict_assertion_failure(&take_skip_records(), true).map(|error| format!("{error:#}"))
}

fn assertion_on(field: &str, skip: Option<AssertionSkip>) -> Assertion {
    Assertion {
        field: Some(field.to_string()),
        skip,
        ..Assertion::default()
    }
}

/// The escape hatch must still generate: with strict off, a marker body is not an error.
#[test]
fn non_strict_is_a_noop_even_on_a_marker_body() {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(
        "    # skipped: field 'chunks' not available on result type\n",
        "python",
        "widget_smoke",
        &[],
    );
    assert!(strict_assertion_failure(&take_skip_records(), false).is_none());
}

#[test]
fn strict_fails_loudly_naming_fixture_and_field() {
    let error = strict_error_for(
        "    # skipped: field 'chunks' not available on result type\n",
        "python",
        "widget_smoke",
        &[],
    )
    .expect("an unresolved field must fail under strict");
    assert!(
        error.contains("[python] fixture `widget_smoke`: field `chunks`"),
        "got: {error}"
    );
}

/// The headline requirement: an unmappable field must fail generation *by default*, with no
/// environment variable armed. This reads the real default rather than a hard-coded `true`,
/// so flipping the default back would fail this test. ~keep
#[test]
fn an_unmappable_field_is_fatal_by_default() {
    let _ = take_skip_records();
    let body = "    // skipped: field 'strategy.crawl_order' not available on result type\n";
    fail_on_unavailable_field_markers(
        body,
        "go",
        "traversal_order",
        &[assertion_on("strategy.crawl_order", None)],
    );
    let error = strict_assertion_failure(&take_skip_records(), super::strict_assertions_enabled())
        .expect("an unmappable field must fail generation by default");
    assert!(
        format!("{error:#}").contains("field `strategy.crawl_order`"),
        "got: {error:#}"
    );
}

/// Every offender is listed, not just the first — one regeneration must surface the whole
/// backlog. ~keep
#[test]
fn strict_error_lists_every_offender() {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(
        "    // skipped: field 'alpha' not available on result type\n\
         \x20   // skipped: field 'beta' not available on result type\n",
        "go",
        "smoke",
        &[],
    );
    let error = strict_assertion_failure(&take_skip_records(), true).expect("two gaps must fail");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("field `alpha`"), "got: {rendered}");
    assert!(rendered.contains("field `beta`"), "got: {rendered}");
    assert!(rendered.starts_with("2 e2e assertion(s)"), "got: {rendered}");
}

/// The opt-in half: the same body, with the fixture declaring the skip, generates cleanly and
/// is recorded as an acknowledged skip rather than silently vanishing. ~keep
#[test]
fn an_explicitly_opted_out_field_skips_and_is_counted() {
    let _ = take_skip_records();
    let body = "    // skipped: field 'strategy.crawl_order' not available on result type\n";
    let assertions = [assertion_on(
        "strategy.crawl_order",
        Some(AssertionSkip::Scoped(AssertionSkipDirective {
            languages: vec!["go".to_string()],
            kind: AssertionSkipKind::LanguageLimitation,
            reason: Some("traversal order is not exposed on the Go result".to_string()),
        })),
    )];
    fail_on_unavailable_field_markers(body, "go", "traversal_order", &assertions);

    let records = take_skip_records();
    assert!(
        strict_assertion_failure(&records, true).is_none(),
        "an acknowledged skip must not fail generation"
    );
    assert_eq!(records.len(), 1, "the acknowledged skip must still be recorded");
    assert_eq!(
        records[0].verdict,
        SkipVerdict::Acknowledged(AssertionSkipKind::LanguageLimitation)
    );
    assert_eq!(records[0].field, "strategy.crawl_order");
    let summary = skip_summary(&records).expect("an acknowledged skip must still produce a summary");
    assert_eq!(
        summary,
        "1 assertion(s) skipped across 1 fixture(s): 0 awaiting alef support, \
         1 language/ABI limitation(s), 0 unresolved field path(s)"
    );
}

/// An opt-out declared `not_representable` lands in alef's backlog, not the binding's, so the
/// summary attributes it to alef rather than filing it as a consumer limitation. ~keep
#[test]
fn a_not_representable_opt_out_is_attributed_to_alef() {
    let _ = take_skip_records();
    let body = "    // skipped: field 'is_error' not available on result type\n";
    let assertions = [assertion_on(
        "is_error",
        Some(AssertionSkip::Scoped(AssertionSkipDirective {
            languages: Vec::new(),
            kind: AssertionSkipKind::NotRepresentable,
            reason: Some("`is_error` is an assertion kind, not a field path".to_string()),
        })),
    )];
    fail_on_unavailable_field_markers(body, "go", "error_smoke", &assertions);
    let records = take_skip_records();
    assert!(strict_assertion_failure(&records, true).is_none());
    let summary = skip_summary(&records).expect("summary");
    assert!(summary.contains("1 awaiting alef support"), "got: {summary}");
    assert!(summary.contains("0 language/ABI limitation(s)"), "got: {summary}");
}

/// A `"skip": true` opt-out covers every language, and defaults to alef's backlog — the
/// observed common case is an assertion shape alef cannot express, not a binding limit.
#[test]
fn a_bare_true_skip_covers_every_language() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    let assertions = [assertion_on("chunks", Some(AssertionSkip::All(true)))];
    let expected = [SkipVerdict::Acknowledged(AssertionSkipKind::NotRepresentable)];
    assert_eq!(verdicts_for(body, "dart", &assertions), expected);
    assert_eq!(verdicts_for(body, "ruby", &assertions), expected);
}

/// A language-scoped opt-out must not silence the same field in a language it does not name.
#[test]
fn a_scoped_skip_does_not_cover_other_languages() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    let assertions = [assertion_on(
        "chunks",
        Some(AssertionSkip::Scoped(AssertionSkipDirective {
            languages: vec!["dart".to_string()],
            kind: AssertionSkipKind::LanguageLimitation,
            reason: None,
        })),
    )];
    assert_eq!(
        verdicts_for(body, "dart", &assertions),
        vec![SkipVerdict::Acknowledged(AssertionSkipKind::LanguageLimitation)]
    );
    assert_eq!(
        verdicts_for(body, "go", &assertions),
        vec![SkipVerdict::UnacknowledgedGap],
        "an opt-out scoped to dart must leave go fatal"
    );
}

/// `"skip": false` is an explicit *refusal* to opt out and must behave as if absent.
#[test]
fn a_false_skip_does_not_opt_out() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    let assertions = [assertion_on("chunks", Some(AssertionSkip::All(false)))];
    assert_eq!(
        verdicts_for(body, "go", &assertions),
        vec![SkipVerdict::UnacknowledgedGap]
    );
}

/// An opt-out on a *different* field must not silence this one.
#[test]
fn a_skip_on_another_field_does_not_opt_this_one_out() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    let assertions = [assertion_on("usage", Some(AssertionSkip::All(true)))];
    assert_eq!(
        verdicts_for(body, "go", &assertions),
        vec![SkipVerdict::UnacknowledgedGap]
    );
}

#[test]
fn a_body_with_no_marker_records_nothing() {
    assert!(verdicts_for("    assert result.count == 1\n", "python", &[]).is_empty());
}

/// Regression control: a synthetic field's "unsupported assertion type" comment
/// is a different defect (bad assertion shape) and must not trip this check.
#[test]
fn unsupported_assertion_type_comments_are_not_recorded() {
    assert!(
        verdicts_for(
            "    // skipped: unsupported assertion type on synthetic field 'embeddings'\n",
            "go",
            &[]
        )
        .is_empty()
    );
}

/// The language-suffixed variants (`not available on Python ProcessingResult`,
/// `not available on PHP result type`, ...) resolve against a *generated* binding type, so
/// they are gaps and stay fatal.
#[test]
fn language_suffixed_not_available_comments_stay_fatal() {
    let error = strict_error_for(
        "\t// skipped: field 'keywords' not available on Go ProcessingResult\n",
        "go",
        "smoke",
        &[],
    )
    .expect("a binding-type resolution miss is a gap");
    assert!(error.contains("field `keywords`"), "got: {error}");
}

/// ~keep This is the variant that hid whole expected-event-sequence assertions, so it is
/// tempting to make it fatal. It must not be: a streaming call returns an event sequence, not
/// a struct, so no field mapping can express the assertion and a consumer cannot fix it from
/// their own config. Failing their build would force a blanket opt-out — the silent skip
/// again, with ceremony. It is loudly *counted* against alef's backlog instead.
#[test]
fn streaming_field_assertions_await_alef_support_rather_than_failing() {
    let body = "    // streaming assertion on unsupported field 'has_page_event'\n";
    assert_eq!(
        verdicts_for(body, "csharp", &[]),
        vec![SkipVerdict::AwaitingGeneratorSupport]
    );
    assert!(
        strict_error_for(body, "csharp", "stream_smoke", &[]).is_none(),
        "a missing generator feature must not fail a consumer's build"
    );
}

/// The same holds for python's streaming-accessor wording and the streaming result type.
#[test]
fn every_streaming_wording_awaits_alef_support() {
    for (body, language) in [
        (
            "    # skipped: streaming field 'stream.items': no python accessor\n",
            "python",
        ),
        (
            "    // skipped: field 'stream.items' not available on streaming result type\n",
            "go",
        ),
    ] {
        assert_eq!(
            verdicts_for(body, language, &[]),
            vec![SkipVerdict::AwaitingGeneratorSupport],
            "{language} streaming wording must be alef's debt, not the consumer's"
        );
    }
}

/// The summary must keep alef's backlog and the binding's backlog in separate buckets, or the
/// number stops being actionable and stops being read. ~keep
#[test]
fn summary_separates_alef_backlog_from_binding_limits() {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(
        "    // streaming assertion on unsupported field 'has_page_event'\n",
        "csharp",
        "stream_smoke",
        &[],
    );
    fail_on_unavailable_field_markers(
        "        // skipped: field 'usage.tokens' references a field or type excluded from \
         the Swift binding\n",
        "swift",
        "excluded_smoke",
        &[],
    );
    let summary = skip_summary(&take_skip_records()).expect("summary");
    assert_eq!(
        summary,
        "2 assertion(s) skipped across 2 fixture(s): 1 awaiting alef support, \
         1 language/ABI limitation(s), 0 unresolved field path(s)"
    );
}

/// ~keep The wordings below are all still *recognised* — that is the invariant the shared
/// `FieldSkip` table exists to hold — but they name real language/ABI limits, so recognition
/// now means "counted in the summary", not "fails the build". Asserting the verdict rather
/// than a panic is what keeps that distinction honest: if one of them were ever reclassified
/// as a gap, these assertions fail rather than silently changing the default's blast radius.
#[test]
fn tagged_union_boundary_wordings_are_counted_not_fatal() {
    let dart = "    // skipped: field 'payload.tags' crosses a tagged-union variant boundary \
                (not expressible in Dart)\n";
    assert_eq!(verdicts_for(dart, "dart", &[]), vec![SkipVerdict::Limitation]);
    let swift = "    // skipped: field 'payload.tags' crosses a tagged-union variant boundary \
                 (not expressible in Swift)\n";
    assert_eq!(verdicts_for(swift, "swift", &[]), vec![SkipVerdict::Limitation]);
}

/// ~keep The swift-bridge count guard fires only on fields `is_valid_for_result` already
/// ACCEPTED, so it must never produce an unacknowledged gap: the backend refusing an
/// assertion as an ABI limit and the gate failing the same assertion as a broken field path
/// is two mechanisms contradicting each other about one fact, which is the defect this
/// verdict pins shut.
#[test]
fn the_swift_json_bridged_count_wording_is_counted_not_fatal() {
    let body = format!(
        "        // skipped: {}\n",
        super::field_skip::FieldSkip::CountOnJsonBridgedLeafInSwift.message("metadata.headings.length")
    );
    assert_eq!(verdicts_for(&body, "swift", &[]), vec![SkipVerdict::Limitation]);
    assert!(
        strict_error_for(&body, "swift", "metadata_headings", &[]).is_none(),
        "a resolvable field refused for an ABI reason must not fail a consumer's build"
    );
}

#[test]
fn ruby_serialized_enum_accessor_wording_is_counted_not_fatal() {
    let body = "    # skipped: enum variant accessor 'metadata.format.excel' not available on Ruby \
                (serialized to Hash)\n";
    // Reclassified LanguageLimitation -> GeneratorGap in 0.77.0: magnus DOES generate a native
    // per-variant accessor, but the enum-registration loop skips every internally-tagged enum, so
    // the accessor is never wired to Ruby. That is alef's unfinished wiring, not a Ruby property.
    // The bucket changed; the property this test exists for did not -- a GeneratorGap is still
    // never fatal, because a consumer cannot fix alef's debt from their own `alef.toml`. ~keep
    assert_eq!(
        verdicts_for(body, "ruby", &[]),
        vec![SkipVerdict::AwaitingGeneratorSupport]
    );
    assert!(
        strict_error_for(body, "ruby", "metadata_format", &[]).is_none(),
        "a generator gap must never fail a consumer's build"
    );
}

#[test]
fn result_is_simple_template_wording_is_counted_not_fatal() {
    let body = "        // skipped: result_is_simple, field 'metadata.title' not on simple result type\n";
    assert_eq!(verdicts_for(body, "php", &[]), vec![SkipVerdict::Limitation]);
}

#[test]
fn not_applicable_for_simple_result_wording_is_counted_not_fatal() {
    let body = "    # skipped: field 'structure.headings' not applicable for simple result type\n";
    assert_eq!(verdicts_for(body, "python", &[]), vec![SkipVerdict::Limitation]);
}

#[test]
fn swift_binding_exclusion_wording_is_counted_not_fatal() {
    let body = "        // skipped: field 'usage.tokens' references a field or type excluded from \
                the Swift binding\n";
    assert_eq!(verdicts_for(body, "swift", &[]), vec![SkipVerdict::Limitation]);
}

/// `result_is_simple for field '<x>' not available on result type` is the *resolver* talking
/// despite the prefix, so it stays a gap while the other `result_is_simple` wordings do not.
#[test]
fn the_result_is_simple_resolver_wording_stays_a_gap() {
    let body = "  # skipped: result_is_simple for field 'metadata' not available on result type\n";
    assert_eq!(verdicts_for(body, "ruby", &[]), vec![SkipVerdict::UnacknowledgedGap]);
}

/// gleam and brew resolve fields against a hand-maintained TOML list rather than the IR, so a
/// rejection there is not trustworthy enough to fail a build on. ~keep
#[test]
fn coarse_oracle_backends_downgrade_gaps_to_limitations() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    assert_eq!(verdicts_for(body, "gleam", &[]), vec![SkipVerdict::Limitation]);
    assert_eq!(verdicts_for(body, "brew", &[]), vec![SkipVerdict::Limitation]);
    assert_eq!(
        verdicts_for(body, "go", &[]),
        vec![SkipVerdict::UnacknowledgedGap],
        "the same wording must stay fatal on an IR-wired backend"
    );
}

#[test]
fn summary_is_none_when_nothing_was_skipped() {
    assert_eq!(skip_summary(&[]), None);
}

#[test]
fn summary_counts_distinct_fixtures_not_markers() {
    let _ = take_skip_records();
    let body = "    // skipped: field 'usage.tokens' references a field or type excluded from \
                the Swift binding\n";
    fail_on_unavailable_field_markers(body, "swift", "alpha", &[]);
    fail_on_unavailable_field_markers(body, "swift", "alpha", &[]);
    fail_on_unavailable_field_markers(body, "swift", "beta", &[]);
    let summary = skip_summary(&take_skip_records()).expect("three markers must summarise");
    assert!(
        summary.starts_with("3 assertion(s) skipped across 2 fixture(s):"),
        "got: {summary}"
    );
}

#[test]
fn is_falsy_flag_accepts_zero_and_case_insensitive_false_only() {
    assert!(is_falsy_flag("0"));
    assert!(is_falsy_flag("false"));
    assert!(is_falsy_flag("FALSE"));
    assert!(!is_falsy_flag("1"));
    assert!(!is_falsy_flag("true"));
    assert!(!is_falsy_flag(""));
    assert!(!is_falsy_flag("no"));
}

/// The diagnostic has to be actionable on its own: it must name where to look and how to
/// declare the skip, or a consumer hitting it has no path forward but to disable the gate.
#[test]
fn diagnostic_names_language_fixture_field_and_the_opt_in() {
    let message = strict_error_for(
        "    # skipped: field 'usage' not available on result type\n",
        "ruby",
        "batch_smoke",
        &[],
    )
    .expect("an unresolved field must fail under strict");
    assert!(message.contains("[ruby]"), "got: {message}");
    assert!(message.contains("`batch_smoke`"), "got: {message}");
    assert!(message.contains("`usage`"), "got: {message}");
    assert!(message.contains("\"skip\""), "must name the opt-in: {message}");
    assert!(
        message.contains(super::STRICT_ASSERTIONS_ENV),
        "must name the escape hatch: {message}"
    );
}

/// A census across five consumer repos found this exact wording on 15
/// backends, `rust` and `brew` among them, and initially read that as 15 independent backend
/// gaps. It is not — every backend renderer calls the same `FieldResolver::is_valid_for_result`
/// (grep `is_valid_for_result` across `e2e/codegen/*/assertions.rs`: one predicate, ~18 call
/// sites), so a field flagged in several languages is one resolution answered once. `rust` is
/// the strongest single data point because, unlike `gleam`/`brew` (see
/// `COARSE_FIELD_ORACLE_LANGUAGES`), it is always IR-wired and resolves an accessor for every
/// field its own IR sees — a rust refusal cannot be a rust-specific capability limit. This pins
/// the diagnostic that now says so, matched exactly (not `contains`) because the wording and its
/// placement — right after the offender list, before the "how to fix it" guidance — is the thing
/// under test. ~keep
#[test]
fn strict_error_names_the_shared_oracle_and_flags_rust_specifically() {
    let message = strict_error_for(
        "    # skipped: field 'chunks' not available on result type\n",
        "rust",
        "widget_smoke",
        &[],
    )
    .expect("an unresolved field must fail under strict");
    let expected = [
        "1 e2e assertion(s) reference a field the availability oracle cannot resolve; they would \
         have been dropped and the generated tests would have passed asserting nothing:",
        "  [rust] fixture `widget_smoke`: field `chunks`",
        "",
        "All backends share one field-availability oracle: a field listed for several languages is \
         one unresolved field, not several backend gaps. `rust` is listed, and rust resolves an \
         accessor for every field its own IR sees — check the fixture path or the \
         field-availability config, not any one backend.",
        "",
        "Fix the field path or the field-availability config, or declare a skip on the assertion:",
        "  \"skip\": { \"kind\": \"not_representable\", \"reason\": \"...\" }  — alef cannot express \
         this shape",
        "  \"skip\": { \"kind\": \"language_limitation\", \"languages\": [\"<lang>\"], \"reason\": \
         \"...\" }  — this binding cannot reach the field",
        "Set ALEF_E2E_STRICT_ASSERTIONS=0 to downgrade this to a warning for one run.",
    ]
    .join("\n");
    assert_eq!(message, expected);
}

/// The mirror control: when `rust` is NOT among the offenders, the rust-specific clause must not
/// appear at all — only the general shared-oracle sentence does. Without this, a fix that always
/// appended the rust clause (rather than gating it on `gaps` actually naming rust) would pass the
/// test above unnoticed and print a claim about rust for languages where rust never even ran.
#[test]
fn strict_error_omits_the_rust_clause_when_rust_is_not_among_the_gaps() {
    let message = strict_error_for(
        "    # skipped: field 'usage' not available on result type\n",
        "python",
        "batch_smoke",
        &[],
    )
    .expect("an unresolved field must fail under strict");
    let expected = [
        "1 e2e assertion(s) reference a field the availability oracle cannot resolve; they would \
         have been dropped and the generated tests would have passed asserting nothing:",
        "  [python] fixture `batch_smoke`: field `usage`",
        "",
        "All backends share one field-availability oracle: a field listed for several languages is \
         one unresolved field, not several backend gaps.",
        "",
        "Fix the field path or the field-availability config, or declare a skip on the assertion:",
        "  \"skip\": { \"kind\": \"not_representable\", \"reason\": \"...\" }  — alef cannot express \
         this shape",
        "  \"skip\": { \"kind\": \"language_limitation\", \"languages\": [\"<lang>\"], \"reason\": \
         \"...\" }  — this binding cannot reach the field",
        "Set ALEF_E2E_STRICT_ASSERTIONS=0 to downgrade this to a warning for one run.",
    ]
    .join("\n");
    assert_eq!(message, expected);
    assert!(
        !message.contains("rust"),
        "the rust-specific clause must not appear when rust never gapped: {message}"
    );
}
