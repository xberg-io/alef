//! R e2e test file rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::test_case::render_test_case;

/// Generation context threaded through to `render_test_case` for every fixture in the category —
/// unchanged across the loop, so bundling it keeps `render_test_file` at 5 parameters. ~keep
pub(super) struct TestFileContext<'a> {
    pub(super) e2e_config: &'a E2eConfig,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) errors: &'a [crate::core::ir::ErrorDef],
}

pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    result_is_simple: bool,
    result_is_r_list: bool,
    context: &TestFileContext<'_>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::e2e_header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(
            &mut out,
            fixture,
            context.e2e_config,
            result_is_simple,
            result_is_r_list,
            context.config,
            context.type_defs,
            context.errors,
        );
        // ~keep R's error path renders `expect_error(...)` and returns, so every other assertion
        // on an error fixture — most often an `equals` against `error.status_code` — leaves no
        // trace in the generated file. The marker sits after the emitted `test_that(...)` call
        // rather than inside its body: the body is built in `test_case.rs`, which another change
        // owns.
        crate::e2e::codegen::error_path_assertions::emit(&mut out, fixture, "# ", "r");
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Emit the `mock.*` once-per-suite request-count helper (alef issue #443) iff this file's
    // rendered fixtures actually call it (`assertion_mock_capture::try_render_mock_capture_assertion`
    // writes a bare call to `capture.helper_name`, no qualifier). Appended after every fixture's
    // test_that() block so it lands once per file, not once per fixture. ~keep
    if let Ok(capture) = crate::e2e::codegen::mock_assertions::mock_capture("r")
        && out.contains(capture.helper_name)
        && let Some(rendered) = crate::e2e::codegen::mock_assertions::render_helper("r")
    {
        let _ = writeln!(out);
        out.push_str(&rendered);
    }

    // Clean up trailing newlines.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod error_path_marker_tests {
    use super::{TestFileContext, render_test_file};
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn render_with_errors(
        extra: Vec<Assertion>,
        declared_value: Option<&str>,
        errors: &[crate::core::ir::ErrorDef],
    ) -> String {
        let mut assertions = vec![Assertion {
            assertion_type: "error".into(),
            value: declared_value.map(|v| serde_json::Value::String(v.to_string())),
            ..Assertion::default()
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
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_file(
            "error",
            &[&fixture],
            false,
            false,
            &TestFileContext {
                e2e_config: &e2e_config,
                config: &ResolvedCrateConfig::default(),
                type_defs: &[],
                errors,
            },
        )
    }

    fn render(extra: Vec<Assertion>) -> String {
        render_with_errors(extra, None, &[])
    }

    /// R's error path renders `expect_error(...)` and returns, so every other assertion on the
    /// fixture used to leave no trace in the generated file at all.
    #[test]
    fn r_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render(vec![Assertion {
            assertion_type: "equals".into(),
            field: Some("error.status_code".into()),
            ..Assertion::default()
        }]);

        // Positive first: the error block really rendered.
        assert!(out.contains("expect_error("), "the error block must render:\n{out}");
        assert!(
            out.contains(
                "# skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "got:\n{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "r");
        assert_eq!(records[0].field, "equals");
    }

    /// Negative control: a lone `error` assertion must leave the generated file marker-free.
    #[test]
    fn r_a_lone_error_assertion_renders_no_marker() {
        let out = render(Vec::new());

        assert!(out.contains("expect_error("), "the error block must render:\n{out}");
        assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    }

    /// The defect this fix closes, driven through the real per-fixture-file rendering entry
    /// point: a declared value naming a real `ErrorVariant` — the R backend has no
    /// error-conversion code at all — must render the registered skip, not a `grepl` check that
    /// can never pass.
    #[test]
    fn r_skips_a_known_variant_it_cannot_substantiate() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let errors = vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Authentication".to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];
        let out = render_with_errors(Vec::new(), Some("Authentication"), &errors);

        assert!(out.contains("expect_error("), "the error block must render:\n{out}");
        assert!(
            out.contains(
                "# skipped: declared error variant 'Authentication' not yet preserved as a distinct identity by \
                 this backend's generator"
            ),
            "got:\n{out}"
        );
        assert!(
            !out.contains("tryCatch"),
            "must not render a check that can never pass, got:\n{out}"
        );
    }
}
