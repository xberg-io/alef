//! `[e2e]` (and registry-mode test-apps) stage dispatch for `alef all`, split out of
//! `all_commands.rs` so the task #362 fix -- deferring a hard e2e codegen abort instead of
//! propagating it with `?` -- does not push that file over the 1,000-line
//! file-modularization cap.
//!
//! Both sub-stages below run inside their own immediately-invoked closure, so a fatal error
//! anywhere in either one (a malformed `[e2e.call(s)]` config, a bad fixtures directory, a
//! write or formatter failure) is captured here instead of propagating out of `handle` with
//! `?`. Bindings, stubs, public API and scaffold output for this crate are already written to
//! disk by the time this stage runs, and are stamped exactly once, at the very end of the
//! crate's iteration in `all_commands.rs`, by `finalize_hashes_sweeping` -- a `?` in this
//! stage that reached `handle`'s own `Result` used to skip that final stamp entirely, leaving
//! already-written, already-formatted binding output on disk with no `alef:hash:` line at
//! all: present, but invisible to `alef verify` and indistinguishable from hand-authored
//! content. See `E2eStageOutcome::error`'s doc comment for how the caller must fold this back
//! in. ~keep

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cli::{cache, pipeline};
use crate::core::config::{E2eConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::e2e::format::DeferredFormatting;

/// What this stage produced for one crate, folded back into `all_commands.rs`'s own
/// per-crate accumulators by its caller.
pub(crate) struct E2eStageOutcome {
    /// Files the `e2e` and `test-apps` sub-stages actually wrote this run (0 when both were
    /// cache hits or both failed before writing anything).
    pub e2e_count: usize,
    /// Whether either sub-stage wrote anything, for the caller's own `any_output_changed` gate.
    pub any_output_changed: bool,
    /// The first fatal error either sub-stage hit, if any -- a bad `[e2e.call(s)]` config, or
    /// a `?` out of either sub-stage's own closure (a malformed fixtures directory, a write or
    /// formatter failure). Distinct from a `generator_error` a single backend reports through
    /// its own `Result` tuple, which this function already survives internally the way it did
    /// before this wrapping existed.
    ///
    /// The caller must defer this into its own run-wide `e2e_stage_error`, exactly like it
    /// already defers `generator_error` -- never `?`/`return` it directly, or the whole point
    /// of this module (reaching the terminal format+stamp pass regardless) is lost again one
    /// call frame up. ~keep
    pub error: Option<anyhow::Error>,
}

/// Runs the `[e2e]` block's call validation, the `e2e` sub-stage and the registry-mode
/// `test-apps` sub-stage for one crate, deferring every fatal error into the returned
/// [`E2eStageOutcome::error`] instead of returning it with `?`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    resolved_cfg: &ResolvedCrateConfig,
    api: &ApiSurface,
    base_dir: &Path,
    config_toml: &str,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
    clean: bool,
    strict: bool,
    refusals: &mut pipeline::WriteReport,
    current_gen_paths: &mut HashSet<PathBuf>,
    deferred_formatting: &mut Vec<DeferredFormatting>,
) -> E2eStageOutcome {
    let mut outcome = E2eStageOutcome {
        e2e_count: 0,
        any_output_changed: false,
        error: None,
    };
    let Some(e2e_config) = &resolved_cfg.e2e else {
        return outcome;
    };

    // A bad `[e2e.call(s)]` config used to `bail!` straight out of this crate's whole
    // iteration -- before either sub-stage below, and before this crate's terminal
    // format+stamp pass, ever ran -- same hazard as an e2e codegen or write failure. Deferred
    // and gated behind `calls_valid` instead, so an already-written, already-formatted
    // binding tree still gets stamped even when this crate's e2e config is broken. ~keep
    let mut calls_valid = true;
    let all_calls =
        std::iter::once(("_default", &e2e_config.call)).chain(e2e_config.calls.iter().map(|(k, v)| (k.as_str(), v)));
    for (call_name, call_config) in all_calls {
        if call_config.function.is_empty() || call_config.module.is_empty() {
            continue;
        }
        let module_path = call_config.module.replace('-', "_");
        let function_name = &call_config.function;
        match crate::extract::validate_call_export(api, &module_path, function_name) {
            crate::extract::ExportValidation::Ok => {}
            crate::extract::ExportValidation::NotFound { function } => {
                calls_valid = false;
                record_error(
                    resolved_cfg,
                    &mut outcome.error,
                    "e2e call validation",
                    anyhow::anyhow!(
                        "e2e call '{call_name}': function '{function}' is not in the extracted API \
                         surface. Fix: declare it `pub` and list its source file in [[crate.sources]] \
                         or [[crate.source_crates]]."
                    ),
                );
            }
            crate::extract::ExportValidation::WrongPath {
                function,
                declared_module,
                actual_paths,
            } => {
                calls_valid = false;
                let paths = actual_paths.join(", ");
                record_error(
                    resolved_cfg,
                    &mut outcome.error,
                    "e2e call validation",
                    anyhow::anyhow!(
                        "e2e call '{call_name}': function '{function}' is not exported at module path \
                         '{declared_module}' -- codegen would emit `use {declared_module}::{function};`. \
                         Actual rust_path(s): {paths}. \
                         Fix: add `pub use <path>::{function};` at the crate root, or point `module` \
                         in [e2e.calls.{call_name}] at one of those paths."
                    ),
                );
            }
        }
    }

    if !calls_valid {
        return outcome;
    }

    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
    let ir_json = match serde_json::to_string(api) {
        Ok(json) => json,
        Err(error) => {
            record_error(resolved_cfg, &mut outcome.error, "e2e codegen", error.into());
            return outcome;
        }
    };

    // Both substages below render the same crate's fixtures, differing only in `dep_mode`,
    // which no e2e validator reads -- so the second recomputes a bit-identical diagnostic set.
    // One log between them reports each finding once per crate. ~keep
    let diagnostic_log = crate::e2e::diagnostic_log::DiagnosticLog::new();

    run_e2e_substage(
        resolved_cfg,
        api,
        e2e_config,
        base_dir,
        &ir_json,
        config_toml,
        sources_hash,
        alef_toml_bytes,
        &fixture_hash,
        clean,
        strict,
        refusals,
        current_gen_paths,
        deferred_formatting,
        &diagnostic_log,
        &mut outcome,
    );
    run_test_apps_substage(
        resolved_cfg,
        api,
        e2e_config,
        base_dir,
        &ir_json,
        config_toml,
        sources_hash,
        alef_toml_bytes,
        &fixture_hash,
        clean,
        strict,
        refusals,
        current_gen_paths,
        deferred_formatting,
        &diagnostic_log,
        &mut outcome,
    );

    outcome
}

/// Records `error` into `slot` if it is the first one this crate's e2e stage has hit, and logs
/// every later one under `label` so a second (or later) failure in the same run is never
/// silently dropped -- mirrors the run-wide `e2e_stage_error`/`docs_stage_error` pattern in
/// `all_commands.rs` at the per-crate scope this module owns. ~keep
fn record_error(
    resolved_cfg: &ResolvedCrateConfig,
    slot: &mut Option<anyhow::Error>,
    label: &str,
    error: anyhow::Error,
) {
    if slot.is_some() {
        tracing::error!("[{}] {label} failed: {error:#}", resolved_cfg.name);
    }
    slot.get_or_insert(error);
}

#[allow(clippy::too_many_arguments)]
fn run_e2e_substage(
    resolved_cfg: &ResolvedCrateConfig,
    api: &ApiSurface,
    e2e_config: &E2eConfig,
    base_dir: &Path,
    ir_json: &str,
    config_toml: &str,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
    fixture_hash: &[u8],
    clean: bool,
    strict: bool,
    refusals: &mut pipeline::WriteReport,
    current_gen_paths: &mut HashSet<PathBuf>,
    deferred_formatting: &mut Vec<DeferredFormatting>,
    diagnostic_log: &crate::e2e::diagnostic_log::DiagnosticLog,
    outcome: &mut E2eStageOutcome,
) {
    let e2e_stage_hash = cache::compute_stage_hash(ir_json, "e2e", config_toml, fixture_hash);

    // Everything fallible in this sub-stage returns into this closure's own `Result` instead
    // of out of the whole crate iteration -- see this module's doc comment. The
    // `generator_error` handling inside is unchanged: it is a distinct, softer failure this
    // closure already knew how to survive before this wrapping existed. ~keep
    let stage_result: anyhow::Result<()> = (|| {
        if !clean && cache::is_stage_cached(&resolved_cfg.name, "e2e", &e2e_stage_hash) {
            tracing::info!("  [e2e] up to date (skipping)");
            let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
            deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                &cached_paths,
                base_dir,
                e2e_config,
                strict,
            )?);
            for path in cached_paths {
                current_gen_paths.insert(path);
            }
        } else {
            tracing::info!("Generating e2e test suites...");
            let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
            let (files, generator_error) = crate::e2e::generate_e2e_with_log(
                resolved_cfg,
                e2e_config,
                None,
                &api.types,
                &api.enums,
                &api.functions,
                &api.errors,
                diagnostic_log,
            )?;
            let e2e_report = pipeline::write_scaffold_files_report(&files, base_dir, true)?;
            refusals.absorb_unwritten(&e2e_report);
            outcome.e2e_count = e2e_report.changed_count();
            if outcome.e2e_count > 0 {
                outcome.any_output_changed = true;
            }
            let managed_files: Vec<_> = files
                .iter()
                .filter(|file| file.carries_alef_marker())
                .cloned()
                .collect();
            deferred_formatting.extend(crate::e2e::format::run_formatters(&managed_files, e2e_config, strict)?);

            let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
            let path_set: HashSet<PathBuf> = output_paths.iter().cloned().collect();

            // The one pre-`format_generated_reporting` stamp this loop keeps, and the only one
            // that is genuinely post-format: `run_formatters` immediately above is this
            // subtree's own formatting pass. ~keep
            pipeline::finalize_hashes(&path_set, sources_hash, alef_toml_bytes)?;

            // A generator failure here must not reach either line below -- see this module's
            // doc comment for the two-part hazard (cache poisoning that hides the failure next
            // run, and orphan-sweeping the last known-good backend output). Write, format and
            // hash finalisation above still ran unconditionally, so this run's partial output
            // still carries a provenance marker for the ownership guard. ~keep
            if let Some(error) = generator_error {
                record_error(resolved_cfg, &mut outcome.error, "e2e codegen", error);
            } else {
                let e2e_output_root = base_dir.join(&e2e_config.output);
                pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[e2e_output_root], &[])?;
                cache::write_stage_hash(&resolved_cfg.name, "e2e", e2e_stage_hash.as_str(), &output_paths)?;
            }

            for path in output_paths {
                current_gen_paths.insert(path);
            }
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        record_error(resolved_cfg, &mut outcome.error, "e2e codegen", error);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_test_apps_substage(
    resolved_cfg: &ResolvedCrateConfig,
    api: &ApiSurface,
    e2e_config: &E2eConfig,
    base_dir: &Path,
    ir_json: &str,
    config_toml: &str,
    sources_hash: &str,
    alef_toml_bytes: &[u8],
    fixture_hash: &[u8],
    clean: bool,
    strict: bool,
    refusals: &mut pipeline::WriteReport,
    current_gen_paths: &mut HashSet<PathBuf>,
    deferred_formatting: &mut Vec<DeferredFormatting>,
    diagnostic_log: &crate::e2e::diagnostic_log::DiagnosticLog,
    outcome: &mut E2eStageOutcome,
) {
    let test_apps_stage_hash = cache::compute_stage_hash(ir_json, "test-apps", config_toml, fixture_hash);

    // Same wrapping, same rationale, as `run_e2e_substage` above. ~keep
    let stage_result: anyhow::Result<()> = (|| {
        if !clean && cache::is_stage_cached(&resolved_cfg.name, "test-apps", &test_apps_stage_hash) {
            tracing::info!("  [test-apps] up to date (skipping)");
            let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
            let mut registry_e2e_config = e2e_config.clone();
            registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
            deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                &cached_paths,
                base_dir,
                &registry_e2e_config,
                strict,
            )?);
            for path in cached_paths {
                current_gen_paths.insert(path);
            }
        } else {
            tracing::info!("Generating registry-mode test apps...");
            let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
            let mut registry_e2e_config = e2e_config.clone();
            registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
            let registry_e2e_ref = &registry_e2e_config;

            let (files, generator_error) = crate::e2e::generate_e2e_with_log(
                resolved_cfg,
                registry_e2e_ref,
                None,
                &api.types,
                &api.enums,
                &api.functions,
                &api.errors,
                diagnostic_log,
            )?;
            let test_apps_report = pipeline::write_scaffold_files_report(&files, base_dir, true)?;
            refusals.absorb_unwritten(&test_apps_report);
            let test_apps_count = test_apps_report.changed_count();
            outcome.e2e_count += test_apps_count;
            if test_apps_count > 0 {
                outcome.any_output_changed = true;
            }
            let managed_files: Vec<_> = files
                .iter()
                .filter(|file| file.carries_alef_marker())
                .cloned()
                .collect();
            deferred_formatting.extend(crate::e2e::format::run_formatters(
                &managed_files,
                registry_e2e_ref,
                strict,
            )?);

            let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
            let path_set: HashSet<PathBuf> = output_paths.iter().cloned().collect();

            // Kept for the same reason as the `e2e` stage's stamp above: it follows that
            // subtree's own native formatting pass, not precedes it. ~keep
            pipeline::finalize_hashes(&path_set, sources_hash, alef_toml_bytes)?;

            // Same hazard, same gate, as the `e2e` stage above. ~keep
            if let Some(error) = generator_error {
                record_error(resolved_cfg, &mut outcome.error, "test-apps codegen", error);
            } else {
                let test_apps_root = base_dir.join(registry_e2e_ref.effective_output());
                pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[test_apps_root], &[])?;
                cache::write_stage_hash(
                    &resolved_cfg.name,
                    "test-apps",
                    test_apps_stage_hash.as_str(),
                    &output_paths,
                )?;
            }

            for path in output_paths {
                current_gen_paths.insert(path);
            }
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        record_error(resolved_cfg, &mut outcome.error, "test-apps codegen", error);
    }
}
