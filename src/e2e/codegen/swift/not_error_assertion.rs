//! `not_error` assertion rendering, split out of `swift/assertions.rs` (already over the
//! repo's 1,000-line cap — see `file-modularization` in CLAUDE.md) so this fix's regression
//! coverage has somewhere to live without growing either oversized file. ~keep

use std::fmt::Write as FmtWrite;

/// Render the `not_error` assertion.
///
/// ~keep An uncaught exception already fails the test via `try` propagation, so this function's
/// only job is deciding whether a *further* `XCTAssertNotNil` would add anything. It never can:
/// every non-void call this renders for returns a Swift value declared NON-optional (`func
/// extract(...) async throws -> ExtractionResult`, `func listOcrBackends() throws -> [String]`,
/// the drained `chunks` array, ...), and Swift auto-promotes a non-optional to `Optional` at an
/// `XCTAssertNotNil` call site, so the assertion compiles and can never fail regardless of what
/// the call returned. An audit found this emitted at 40 sites across 15 generated Swift test
/// files, all tautological, and four of them (`testChunkingConfigAndOutput`,
/// `testChunkingRagHandoff`, `testLanguageDetectionConfig`, `testLanguageDetectionMultilingual`
/// in the swift e2e suite) had it as their ONLY assertion once every other assertion on the same
/// fixture was dropped as a `CountOnJsonBridgedLeafInSwift` skip — a document with zero chunks or
/// zero detected languages passed those tests, which is exactly the regression two of them exist
/// to catch.
///
/// - `returns_void` calls bind no `result` at all (see `test_method.rs`'s `if
///   call_config.returns_void` branch), so there's nothing to assert *on* the way the non-void
///   case below does. `test_method.rs` never even reaches this function for that case — its
///   assertion loop skips `not_error` on a void call outright and instead wraps the call itself
///   in `XCTAssertNoThrow` (sync) or a do/catch (async), via its `void_not_error` flag, so the
///   real check lives one level up from here. This branch stays as this function's own correct
///   behavior in isolation — asserting on an unbound variable would not compile — not because the
///   void case goes unchecked. ~keep
/// - Every other case (a bound `result`, the drained `chunks` array, or a bare `Optional<T>`
///   result that may legitimately be `nil` on success) renders the same comment: there is no
///   value it can bind that both compiles and can fail, so `not_error`'s entire contribution is
///   the `try` above it. Collapsing all three to one wording — rather than three call sites that
///   happen to agree — is what keeps a future fourth case from reinventing the tautology.
pub(super) fn render_not_error_assertion(out: &mut String, returns_void: bool) {
    if returns_void {
        // No variable to assert on; the exception path already covers this.
    } else {
        let _ = writeln!(out, "        // not_error: covered by try propagation");
    }
}

#[cfg(test)]
mod tests {
    use super::render_not_error_assertion;

    /// A bare `Optional<T>` result (no field path) must not get an `XCTAssertNotNil` from
    /// `not_error` — `nil` is a valid non-error outcome, and a paired `is_empty`/`is_true`
    /// assertion on the same bare result already emits `XCTAssertNil`.
    #[test]
    fn bare_optional_result_emits_no_not_nil_assertion() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, false);

        assert!(
            !out.contains("XCTAssertNotNil(result)"),
            "bare Optional result must not assert not-nil from `not_error`: {out}"
        );
    }

    /// A non-optional bound result gets no `XCTAssertNotNil` either: the return type is declared
    /// non-optional, so Swift's auto-promotion at the call site makes the assertion pass for
    /// every possible value, including ones the fixture should have rejected. The `try` above it
    /// is the only real check `not_error` can contribute here.
    #[test]
    fn non_optional_result_emits_no_tautological_assertion() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, false);

        assert!(
            !out.contains("XCTAssertNotNil"),
            "non-optional result must not get a tautological XCTAssertNotNil: {out}"
        );
        assert_eq!(out, "        // not_error: covered by try propagation\n");
    }

    /// The drained `chunks` array from a streaming call is declared non-optional too, so it gets
    /// the same treatment as any other non-void result: no assertion, just the comment.
    #[test]
    fn streaming_result_emits_no_tautological_assertion() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, false);

        assert!(!out.contains("XCTAssertNotNil(chunks)"));
    }

    #[test]
    fn void_result_emits_nothing() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, true);

        assert!(out.is_empty());
    }
}
