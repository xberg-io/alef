use anyhow::Result;

pub(super) fn ensure_success(
    has_stale_bindings: bool,
    has_version_issues: bool,
    snippet_coverage_issue_count: usize,
    report_only: bool,
) -> Result<()> {
    if report_only || (!has_stale_bindings && !has_version_issues && snippet_coverage_issue_count == 0) {
        return Ok(());
    }
    anyhow::bail!("generated bindings, versions, or snippet coverage are out of date")
}

/// Fail `alef verify` when the formatter was invoked and failed for some generated files.
///
/// Those files' formatted-output comparison never happened, so the run examined less than it
/// claims; passing would certify bytes it did not check (alef#934). A formatter that is merely
/// not installed stays a counted, non-fatal skip, as before. `report_only` downgrades it like
/// every other verify failure. ~keep
pub(super) fn ensure_formatter_ran(skipped_formatter_error: usize, report_only: bool) -> Result<()> {
    if report_only || skipped_formatter_error == 0 {
        return Ok(());
    }
    anyhow::bail!(
        "the formatter invocation failed, so {skipped_formatter_error} generated file(s) could not be \
         checked for formatted-output drift; fix the formatter (e.g. a `poly` older than the repo's \
         `poly.lock`) and rerun"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_formatter_invocation_fails_verification() {
        let error = ensure_formatter_ran(3, false).expect_err("skipped files must not pass");
        assert!(error.to_string().contains("3 generated file(s)"));
    }

    #[test]
    fn no_formatter_failures_pass_and_report_only_downgrades() {
        ensure_formatter_ran(0, false).expect("nothing skipped");
        ensure_formatter_ran(3, true).expect("report-only keeps a successful exit status");
    }

    #[test]
    fn report_only_succeeds_with_multi_crate_coverage_issues() {
        ensure_success(false, false, 2, true).expect("report-only keeps a successful exit status");
    }

    #[test]
    fn coverage_issue_fails_with_honest_diagnostic() {
        let error = ensure_success(false, false, 1, false).expect_err("coverage issue must fail verification");
        assert!(error.to_string().contains("snippet coverage"));
    }
}
