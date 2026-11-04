//! `alef all`'s pre-flight snippet-coverage check, split out of `all_commands.rs` so the
//! deferred-failure change (task #186) does not push that file over the repo's 1,000-line cap.
//!
//! This runs before the main per-crate generation loop and answers "would this run's fixtures
//! produce complete documentation-snippet coverage" -- including the mock-harness guard
//! (`e2e::snippets::mock_harness_guard`), which rejects a fixture whose generated snippet body
//! still points at e2e test-harness scaffolding. A rejection here is a real, attributable defect
//! that must fail the run, but it must not fail the run *before every other stage has had a
//! chance to run* -- one rejected fixture out of hundreds used to abort `alef all` before the
//! main loop's bindings/e2e/test-apps/docs stages ever started, writing nothing at all. This
//! function records a failure per crate and lets the caller continue into the main loop
//! regardless. ~keep

use crate::cli::pipeline;
use crate::core::config::ResolvedCrateConfig;

use super::stage_failures::StageFailures;

pub(crate) fn run_snippet_coverage_preflight(
    crates_to_process: &[&ResolvedCrateConfig],
    config_path: &std::path::Path,
    failures: &mut StageFailures,
) {
    for resolved_cfg in crates_to_process {
        let Some(e2e_config) = &resolved_cfg.e2e else {
            continue;
        };
        let api = match pipeline::extract(resolved_cfg, config_path, false) {
            Ok(api) => api,
            Err(error) => {
                failures.record(
                    &format!("[{}] snippet coverage precondition (extract)", resolved_cfg.name),
                    error,
                );
                continue;
            }
        };
        let coverage =
            match crate::e2e::evaluate_snippet_coverage_for_manifest(resolved_cfg, e2e_config, &api, config_path) {
                Ok(coverage) => coverage,
                Err(error) => {
                    failures.record(&format!("[{}] snippet coverage precondition", resolved_cfg.name), error);
                    continue;
                }
            };
        let Some(coverage) = coverage else {
            continue;
        };
        if let Err(error) = crate::e2e::ensure_fresh_snippet_coverage_complete(&coverage) {
            failures.record(&format!("[{}] snippet coverage precondition", resolved_cfg.name), error);
        }
    }
}
