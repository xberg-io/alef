//! `mock.*` request-count virtual field assertion rendering for R e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `kotlin::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! `r::test_file::render_test_file`), not a struct field, so interception happens before every
//! other branch in `assertions::render_assertion` -- including the synthetic/derived-field
//! dispatch (`chunks_synthetic::try_render` and the `embeddings`/`keywords`/etc. arms) and the
//! `is_valid_for_result` field-availability gate further down, which would otherwise misclassify
//! a `mock.*` field as `FieldSkip::NotAvailableOnResultType` (blaming the fixture) rather than
//! alef's own generator gap. The helper is a private, file-local R function using this backend's
//! own leading-dot naming convention (`.resolve_fixture`, `.alef_format_value` in
//! `r/setup_fixtures.jinja`/`r/test_case.rs`), so every call site here reads a bare function call
//! -- no package qualifier, no import to gate on.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// R expression for the mock server's base URL -- every other per-fixture mock-URL call site in
/// this backend (`r/args.rs`) already reads it through this same environment variable.
const MOCK_SERVER_URL_EXPR: &str = "Sys.getenv(\"MOCK_SERVER_URL\")";

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to return
/// immediately rather than fall through to ordinary field resolution.
pub(super) fn try_render_mock_capture_assertion(out: &mut String, assertion: &Assertion) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let capture = mock_capture("r").unwrap_or_else(|_| {
        panic!("R e2e generator: mock.* field '{field}' has no r capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an R integer) into the same `testthat` expectation family
/// every other numeric comparison in this backend already uses (`expect_equal`/`expect_true`),
/// with no `info=`/message argument -- matching every other assertion arm in `assertions.rs`,
/// none of which pass one either.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "R e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "R e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "  expect_equal({expr}, {n})");
        }
        // testthat has no `expect_not_equal`; every other backend's `not_equals` arm is a bare
        // comparison operator (see `python`/`java`/`kotlin::assertion_mock_capture`), so this
        // uses this backend's own `expect_true` comparison idiom (`assertions.rs`'s
        // `greater_than`/`less_than` arms) instead of inventing a new one. ~keep
        "not_equals" => {
            let _ = writeln!(out, "  expect_true({expr} != {n})");
        }
        "greater_than" => {
            let _ = writeln!(out, "  expect_true({expr} > {n})");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "  expect_true({expr} >= {n})");
        }
        "less_than" => {
            let _ = writeln!(out, "  expect_true({expr} < {n})");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "  expect_true({expr} <= {n})");
        }
        other => {
            panic!("R e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
