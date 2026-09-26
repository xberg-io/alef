//! Per-language capture table for the `mock.*` virtual-field namespace (alef issue #433).
//!
//! ~keep Returns `Result<MockCapture, MockCaptureGap>`, never `Option`: `assertion_kinds_design.rs`
//! documents that an `Option` at exactly this boundary
//! (`StreamingFieldResolver::accessor_with_streaming_context`) is what let six backends collapse
//! three different reasons into one silent `None` and render nothing at all. This table only has
//! one reason a language can lack the capture -- alef has not written that language's helper
//! template yet -- but the `Result` shape is kept anyway so a caller pattern-matches instead of
//! reflexively `.unwrap_or`-ing past a `None`, the habit that produced the streaming gap.

/// What a language needs to emit the once-per-suite mock-request-count helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MockCapture {
    /// Language-native function name for the emitted helper.
    pub(crate) helper_name: &'static str,
    /// Minijinja template name registered in `crate::e2e::template_env`, rendering the helper's
    /// full source. See `super::helper` for the context contract every entry must honor.
    pub(crate) helper_template: &'static str,
}

/// `language` has no capture-table entry.
///
/// [`crate::e2e::codegen::field_skip::FieldSkip::MockCaptureNotSupported`] is the wording a caller
/// renders for this; class [`crate::e2e::codegen::field_skip::SkipClass::GeneratorGap`] -- a
/// consumer cannot add the capture from their own `alef.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MockCaptureGap;

/// The per-language mock-request-count capture table.
///
/// ~keep Keyed by `E2eCodegen::language_name()`, not by template-directory name: the TypeScript
/// generator's templates live under `templates/typescript/` but its `language_name()` (and every
/// other table this codegen keys on, e.g. `assertion_types::BACKEND_UNSUPPORTED_ASSERTION_TYPES`)
/// returns `"node"`. `"typescript"` as a key here would silently never match anything, which is
/// exactly the failure mode `tests::every_capture_table_key_names_a_real_generator` pins shut.
///
/// Populated for `rust`, `python`, `node` only -- the three backends a later wave wires into their
/// own `render_assertion`. Every other language returns [`MockCaptureGap`]; see
/// `mock_assertions::ensure_capture_declared` for what a fixture author does about that today.
pub(crate) const MOCK_CAPTURE_TABLE: &[(&str, MockCapture)] = &[
    (
        "rust",
        MockCapture {
            helper_name: "alef_mock_request_count",
            helper_template: "rust/mock_request_helper.rs.jinja",
        },
    ),
    (
        "python",
        MockCapture {
            helper_name: "_alef_mock_request_count",
            helper_template: "python/mock_request_helper.py.jinja",
        },
    ),
    (
        "node",
        MockCapture {
            helper_name: "alefMockRequestCount",
            helper_template: "typescript/mock_request_helper.jinja",
        },
    ),
];

/// Look up `language`'s capture, or the gap when none is registered.
pub(crate) fn mock_capture(language: &str) -> Result<MockCapture, MockCaptureGap> {
    MOCK_CAPTURE_TABLE
        .iter()
        .find(|(name, _)| *name == language)
        .map(|(_, capture)| *capture)
        .ok_or(MockCaptureGap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_wired_languages_resolve() {
        for language in ["rust", "python", "node"] {
            assert!(
                mock_capture(language).is_ok(),
                "expected a capture entry for {language}"
            );
        }
    }

    #[test]
    fn an_unwired_language_returns_the_gap() {
        assert_eq!(mock_capture("go"), Err(MockCaptureGap));
    }

    /// `"typescript"` is a template-directory name, never a `language_name()` value -- pinning
    /// this negative guards against silently keying the table on the wrong string. ~keep
    #[test]
    fn typescript_is_not_a_language_name() {
        assert_eq!(mock_capture("typescript"), Err(MockCaptureGap));
    }

    /// The table-key test: a key naming no real generator would silently disable its own row,
    /// exactly the failure `assertion_types::tests::every_unsupported_key_names_a_real_generator`
    /// guards against for `BACKEND_UNSUPPORTED_ASSERTION_TYPES`. ~keep
    #[test]
    fn every_capture_table_key_names_a_real_generator() {
        let languages: Vec<&str> = crate::e2e::codegen::all_generators()
            .iter()
            .map(|generator| generator.language_name())
            .collect();
        for (language, _) in MOCK_CAPTURE_TABLE {
            assert!(
                languages.contains(language),
                "'{language}' is not a generator language name, so its row would never apply"
            );
        }
    }
}
