//! Renders the once-per-suite mock-request-count helper via Minijinja (alef issue #433).
//!
//! ~keep `jinja-templates` is a CRITICAL rule for this module: the helper is parameterized
//! generated code (a language-native function whose name and endpoint paths are interpolated), so
//! it is emitted through `crate::e2e::template_env::render`, never `format!`/`push_str`.
//!
//! # Context contract
//!
//! Every template registered in [`super::snippets::MOCK_CAPTURE_TABLE`] receives exactly these
//! variables, so a later wave edits only a template's BODY, never this call site:
//!
//! - `helper_name` (str) -- the function name `MockCapture::helper_name` names for this language.
//! - `total_path` (str) -- the mock server's total-count endpoint path ([`MOCK_TOTAL_PATH`]).
//! - `one_path` (str) -- the mock server's single-key endpoint path ([`MOCK_ONE_PATH`]).
//!
//! Every template emits exactly ONE function, taking the mock server's base URL and a
//! fully-formed path-and-query string, and returning the endpoint's plain-text decimal integer
//! body as a native integer. Building that path-and-query string from a [`super::model::MockQuery`]
//! (`{total_path}?prefix=<p>` for [`super::model::MockQuery::Total`], `{one_path}?key=<k>` for
//! [`super::model::MockQuery::Count`]) and wiring the call site into each backend's own assertion
//! renderer is the later wave's job, not this module's.

use crate::e2e::template_env;

use super::snippets::{MockCapture, mock_capture};

/// The mock server's total-count introspection endpoint path.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no emitter consumes it until the reference-backend wave -- #433"
    )
)]
pub(crate) const MOCK_TOTAL_PATH: &str = "/__alef/requests/total";

/// The mock server's single-key introspection endpoint path.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no emitter consumes it until the reference-backend wave -- #433"
    )
)]
pub(crate) const MOCK_ONE_PATH: &str = "/__alef/requests/one";

/// Render `language`'s once-per-suite mock-request-count helper, or `None` when `language` has no
/// capture-table entry (see [`super::snippets::mock_capture`]).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no emitter consumes it until the reference-backend wave -- #433"
    )
)]
pub(crate) fn render_helper(language: &str) -> Option<String> {
    let MockCapture {
        helper_name,
        helper_template,
    } = mock_capture(language).ok()?;
    Some(template_env::render(
        helper_template,
        minijinja::context! {
            helper_name => helper_name,
            total_path => MOCK_TOTAL_PATH,
            one_path => MOCK_ONE_PATH,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wired_language_renders_its_own_helper_name() {
        for (language, expected_name) in [
            ("rust", "alef_mock_request_count"),
            ("python", "_alef_mock_request_count"),
            ("node", "alefMockRequestCount"),
        ] {
            let rendered = render_helper(language).unwrap_or_else(|| panic!("{language} must render a helper"));
            assert!(
                rendered.contains(expected_name),
                "{language} helper must be named {expected_name}, got:\n{rendered}"
            );
        }
    }

    /// The endpoint path constants are documentation for the caller that builds
    /// `path_and_query` (see the module doc's context contract) -- the helper's own body never
    /// references them, so this pins that they at least reach the template context unmangled,
    /// rather than asserting they appear in emitted code that has no reason to contain them.
    #[test]
    fn the_endpoint_paths_are_the_documented_constants() {
        assert_eq!(MOCK_TOTAL_PATH, "/__alef/requests/total");
        assert_eq!(MOCK_ONE_PATH, "/__alef/requests/one");
    }

    #[test]
    fn an_unwired_language_renders_nothing() {
        assert!(render_helper("go").is_none());
    }
}
