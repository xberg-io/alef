use crate::cli::pipeline::helpers::{
    check_precondition, precondition_passes, run_before, run_command_captured_with_env, run_command_with_env,
};
use crate::cli::registry;
use crate::core::config::{BuildCommandConfig, Language, ResolvedCrateConfig};
use crate::process::capture::{OUTPUT_DRAIN_GRACE, collect_output_within, output_reader_tee};
use crate::process::timed::{Deadline, GroupChild};
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info, warn};

mod build_command;
mod frb_bridge_coverage;
mod frb_cache;
mod frb_cfg_gates;
mod frb_version_check;
mod observability;
mod staging_profile;

use build_command::{build_command_for, output_path_for, resolve_crate_dir};
pub use staging_profile::StagingProfile;

#[cfg(test)]
mod build_command_tests;
#[cfg(all(test, unix))]
mod build_orchestration_tests;
#[cfg(test)]
mod ffi_stage_post_build_tests;
#[cfg(all(test, unix))]
mod napi_js_ownership_tests;
#[cfg(all(test, unix))]
mod napi_package_json_path_tests;
#[cfg(test)]
mod python_maturin_tests;
#[cfg(test)]
mod readiness_tests;
#[cfg(test)]
mod record_post_build_outcome_tests;
#[cfg(all(test, unix))]
mod run_command_tests;

// Re-exported for `alef verify`'s frb-gate-drift check (`bin_cli::core_commands::verify`), which
// needs the identical canonicalization this crate's own `CarryFrbCfgGates` post-build step uses
// to write the file, so the two can never disagree about what "up to date" means. See alef #179.
pub(crate) use frb_cfg_gates::canonical_frb_generated;

pub fn build(config: &ResolvedCrateConfig, languages: &[Language], release: bool, strict: bool) -> anyhow::Result<()> {
    build_with_environment(config, languages, release, &[], strict)
}

pub(crate) fn build_with_environment(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    release: bool,
    environment: &[(&str, &str)],
    strict: bool,
) -> anyhow::Result<()> {
    let crate_name = &config.name;
    let base_dir = std::env::current_dir()?;
    // The profile this invocation's own `cargo build`s just produced -- `StageFfiLibrary` must
    // look for exactly this profile's uplifted artifact, never the other one left over from an
    // earlier, unrelated run. ~keep
    let just_built_profile = StagingProfile::JustBuilt(if release {
        crate::publish::package::BuildProfile::Release
    } else {
        crate::publish::package::BuildProfile::Debug
    });

    let mut independent = Vec::new();
    let mut ffi_dependent = Vec::new();
    let mut need_ffi = false;

    let mut rust_langs: Vec<Language> = Vec::new();

    // Reconciled against `dispatched_count` at the end of this function: every
    // announced language must be accounted for as skipped, blocked on an unmet
    // precondition, or dispatched below. ~keep
    let total_announced = languages.len();
    let mut skipped_count = 0_usize;
    // Kept apart from `failures` all the way to the exit code: these languages were never
    // compiled, so folding them in would assert something about generated code that this run
    // never tested. ~keep
    let mut unmet: Vec<String> = Vec::new();
    // Subset of the languages folded into `skipped_count` above: specifically those skipped
    // because their toolchain was not on `PATH`, as opposed to a structural skip (no backend, no
    // build config for this target). Tracked separately so `--strict` can fail on exactly the
    // "never examined" case this rule targets, without also failing on a target this checkout
    // was never going to build regardless of environment. ~keep
    let mut toolchain_missing: Vec<String> = Vec::new();

    for &lang in languages {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        match backend_readiness(lang, &build_cmd_cfg) {
            BackendReadiness::Ready => {}
            BackendReadiness::ToolchainMissing { precondition } => {
                observability::skipped(lang, "required tool is not on PATH");
                skipped_count += 1;
                toolchain_missing.push(format!("{lang} (precondition failed: {precondition})"));
                continue;
            }
            BackendReadiness::DependenciesUnfetched { check, remediation } => {
                observability::unmet_precondition(
                    lang,
                    &format!("dependency precondition failed ({check})"),
                    &remediation,
                );
                unmet.push(format!("{lang} (run `{remediation}`)"));
                continue;
            }
        }
        if lang == Language::Rust {
            rust_langs.push(lang);
            continue;
        }
        // `try_get_backend`, not `get_backend`: the latter panics for docs-only/
        // consumer-only targets (Rust, C). Rust is already routed above; a
        // language like C configured in `[workspace] languages` must be skipped
        // gracefully here rather than crashing the whole build. ~keep
        let Some(backend) = registry::try_get_backend(lang) else {
            info!("No binding backend for {lang}, skipping");
            observability::skipped(lang, "no binding backend");
            skipped_count += 1;
            continue;
        };
        if let Some(bc) = backend.build_config_with_config(config) {
            if bc.depends_on_ffi() {
                ffi_dependent.push((lang, bc));
                need_ffi = true;
            } else {
                independent.push((lang, bc));
            }
        } else {
            info!("No build config for {lang}, skipping");
            observability::skipped(lang, "no build config");
            skipped_count += 1;
        }
    }
    let dispatched_count = rust_langs.len() + independent.len() + ffi_dependent.len();

    // Every stage below records its own per-language failures into `failures`
    // instead of bailing out with `?`. ~keep A missing/misconfigured recipe or a
    // real compile failure in one backend must not erase build signal for every
    // other, unrelated backend — see the "false" command-substitution incident:
    // one unconfigured backend used to fail-fast the whole build for languages
    // that had nothing to do with it. The run still fails overall, but only
    // after every backend got a chance to run and report its own outcome.
    let mut failures: Vec<String> = Vec::new();
    // Populated by `record_post_build_outcome` -- see its doc comment for the discard bug this
    // closes. Non-fatal (never folded into `failures`), but named in the summary log below so a
    // fallback to stale committed output is never silent. ~keep
    let mut skipped_post_build_tools: Vec<String> = Vec::new();

    for &lang in &rust_langs {
        let result = observability::observe(lang, || {
            let build_cmd_cfg = config.build_command_config_for_language(lang);
            run_before(lang, build_cmd_cfg.before.as_ref())?;
            let cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = cmds {
                for cmd in cmd_list.commands() {
                    info!("Building {lang}: {cmd}");
                    run_command_with_env(cmd, environment).with_context(|| format!("failed to build {lang}"))?;
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            failures.push(format!("{lang}: {err:#}"));
        }
    }

    if need_ffi
        && !independent
            .iter()
            .any(|(_, bc)| bc.tool == "cargo" && bc.crate_suffix == "-ffi")
    {
        // Same workspace-membership trap as the `"cargo"` arm of `build_command_for`: a `-p`
        // package spec resolves only for a workspace member, while the emitted FFI crate is
        // standalone unless the consumer lists it. Prefer the crate's own manifest, which is
        // correct either way. ~keep
        let ffi_crate_root = output_path_for(Language::Ffi, config)
            .map(resolve_crate_dir)
            .and_then(|p| p.to_str())
            .map(str::to_string)
            .or_else(|| crate::core::config::resolve_helpers::default_binding_crate_root(crate_name, "ffi"))
            .unwrap_or_else(|| format!("crates/{crate_name}-ffi"));
        info!("Building FFI crate: {ffi_crate_root}");
        let mut cmd = format!("cargo build --manifest-path {ffi_crate_root}/Cargo.toml");
        if release {
            cmd.push_str(" --release");
        }
        let result = observability::observe(Language::Ffi, || {
            run_command_with_env(&cmd, environment).context("failed to build FFI crate")
        });
        if let Err(err) = result {
            failures.push(format!("{}: {err:#}", Language::Ffi));
        }
    }

    // Before-hooks run sequentially (they may touch shared resources like a
    // lockfile) but a failing hook only takes its own language out of the
    // parallel dispatch below — it does not stop the remaining before-hooks
    // or the rest of the build. `before` is rare in practice, so we only pay
    // for its own started/completed observability pair when one is actually
    // configured; the language's real build attempt is observed separately
    // once it reaches the parallel dispatch. ~keep
    let mut independent_ready = Vec::with_capacity(independent.len());
    for (lang, bc) in independent {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        let before = build_cmd_cfg.before;
        let before_result = if before.is_some() {
            observability::observe(lang, || run_before(lang, before.as_ref()))
        } else {
            Ok(())
        };
        match before_result {
            Ok(()) => independent_ready.push((lang, bc)),
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }
    let independent = independent_ready;

    let build_results: Vec<anyhow::Result<(String, String)>> = independent
        .par_iter()
        .map(|(lang, bc)| {
            observability::observe(*lang, || {
                let build_cmd_cfg = config.build_command_config_for_language(*lang);
                let override_cmds = if release {
                    build_cmd_cfg.build_release.as_ref()
                } else {
                    build_cmd_cfg.build.as_ref()
                };
                if let Some(cmd_list) = override_cmds
                    && config.build_commands.contains_key(&lang.to_string())
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured_with_env(cmd, environment)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
                info!("Building {lang} ({})...", bc.tool);
                let build_cmd = build_command_for(*lang, bc, config, release);
                run_command_captured_with_env(&build_cmd, environment)
                    .with_context(|| format!("failed to build language bindings for {lang}"))
            })
        })
        .collect();

    for ((lang, bc), result) in independent.iter().zip(build_results) {
        match result {
            Ok((stdout, stderr)) => {
                if !stdout.is_empty() {
                    info!("[{lang} build] {stdout}");
                }
                if !stderr.is_empty() {
                    debug!("[{lang} build] {stderr}");
                }
                record_post_build_outcome(
                    *lang,
                    run_post_build(*lang, bc, config, &base_dir, just_built_profile),
                    &mut failures,
                    &mut skipped_post_build_tools,
                );
            }
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }

    // ffi_dependent backends are attempted unconditionally, even if the FFI
    // crate build above failed: attempting them still yields a true,
    // per-backend outcome (they'll fail for a real reason if the FFI crate is
    // genuinely broken), which is strictly more informative than skipping
    // them and losing their signal entirely. ~keep
    let mut ffi_dependent_ready = Vec::with_capacity(ffi_dependent.len());
    for (lang, bc) in ffi_dependent {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        let before = build_cmd_cfg.before;
        let before_result = if before.is_some() {
            observability::observe(lang, || run_before(lang, before.as_ref()))
        } else {
            Ok(())
        };
        match before_result {
            Ok(()) => ffi_dependent_ready.push((lang, bc)),
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }
    let ffi_dependent = ffi_dependent_ready;

    let build_results: Vec<anyhow::Result<(String, String)>> = ffi_dependent
        .par_iter()
        .map(|(lang, bc)| {
            observability::observe(*lang, || {
                let build_cmd_cfg = config.build_command_config_for_language(*lang);
                let override_cmds = if release {
                    build_cmd_cfg.build_release.as_ref()
                } else {
                    build_cmd_cfg.build.as_ref()
                };
                if let Some(cmd_list) = override_cmds
                    && config.build_commands.contains_key(&lang.to_string())
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured_with_env(cmd, environment)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
                info!("Building {lang} ({})...", bc.tool);
                let build_cmd = build_command_for(*lang, bc, config, release);
                run_command_captured_with_env(&build_cmd, environment)
                    .with_context(|| format!("failed to build language bindings for {lang}"))
            })
        })
        .collect();

    for ((lang, bc), result) in ffi_dependent.iter().zip(build_results) {
        match result {
            Ok((stdout, stderr)) => {
                if !stdout.is_empty() {
                    info!("[{lang} build] {stdout}");
                }
                if !stderr.is_empty() {
                    debug!("[{lang} build] {stderr}");
                }
                record_post_build_outcome(
                    *lang,
                    run_post_build(*lang, bc, config, &base_dir, just_built_profile),
                    &mut failures,
                    &mut skipped_post_build_tools,
                );
            }
            Err(err) => failures.push(format!("{lang}: {err:#}")),
        }
    }

    // Reconciliation, not just a status line: `dispatched_count` is exactly
    // `rust_langs.len() + independent.len() + ffi_dependent.len()` captured
    // right after classification, before any before-hook filtering or `?`
    // could shrink it — so if this doesn't equal `total_announced -
    // skipped_count`, some announced language fell through the classification
    // loop without either a skip or a dispatch, which is a bug in the loop
    // above, not downstream. Every dispatched language is guaranteed a
    // terminal observability event by construction: rust_langs, independent,
    // and ffi_dependent are each fully drained by an unconditional loop or
    // `.par_iter().collect()` (no `?` early-return anywhere in between), so
    // silently losing one after this point cannot happen without also
    // failing this assertion. ~keep
    debug_assert_eq!(
        skipped_count + unmet.len() + dispatched_count,
        total_announced,
        "every announced language must be skipped, blocked on a precondition, or dispatched"
    );
    // Not `dispatched_count - failures.len()`: `failures` can also include the
    // implicit FFI-crate auto-build (see `need_ffi` above), which fires as a
    // side effect for backends that depend on it and isn't itself one of the
    // `dispatched_count` entries when "ffi" wasn't explicitly requested — so
    // that subtraction could under-report. Report what's exact instead. ~keep
    info!(
        "Backend build summary: {total_announced} announced, {skipped_count} skipped ({} missing toolchain), {} \
         blocked on unmet preconditions, {dispatched_count} dispatched, {} failure(s), {} post-build tool(s) \
         skipped (not on PATH)",
        toolchain_missing.len(),
        unmet.len(),
        failures.len(),
        skipped_post_build_tools.len()
    );
    if !toolchain_missing.is_empty() {
        if strict {
            warn!(
                "--strict is set: {} language(s) skipped for a missing toolchain will fail this run: {}",
                toolchain_missing.len(),
                toolchain_missing.join(", ")
            );
        } else {
            info!(
                "{} language(s) skipped for a missing toolchain (non-fatal; pass --strict in CI to fail on this): \
                 {}",
                toolchain_missing.len(),
                toolchain_missing.join(", ")
            );
        }
    }

    build_outcome(&failures, &unmet, &toolchain_missing, strict)
}

/// Turn the two per-language buckets into this command's exit status.
///
/// Both buckets are fatal, and they are reported in separate sentences that never merge counts.
/// The reasoning behind each half:
///
/// - Unmet preconditions are fatal because the alternative is worse. Nothing was built for those
///   languages, so exiting 0 would let anything reading this exit code — CI, a release script,
///   the snippet validation that links these very artifacts — proceed as though the artifacts
///   exist. The remediation is one command in this same checkout, so failing costs the developer
///   a single retry and buys everyone downstream a truthful signal.
/// - They are nonetheless not `failures`: the count, the wording, and the per-language outcome all
///   say "not built" rather than "built and broken", which is the distinction that makes
///   "run `mix deps.get`" actionable where a bare failure was not. ~keep
///
/// A *missing toolchain* is deliberately absent from both buckets and stays a non-fatal skip by
/// default: it is a statement about the machine, not about this checkout, and a developer without
/// `gradle` installed must still be able to build the languages they do have. That default is
/// exactly what makes it invisible in CI, though: a language whose toolchain is absent there was
/// never built, its bindings were never produced, and nothing downstream (including snippet
/// validation) can tell that apart from "nothing to build" without reading the log by hand. `strict`
/// closes that gap without changing the default: passed, `toolchain_missing` becomes a third fatal
/// bucket with its own sentence, same shape as `unmet`. ~keep
fn build_outcome(
    failures: &[String],
    unmet: &[String],
    toolchain_missing: &[String],
    strict: bool,
) -> anyhow::Result<()> {
    let mut parts = Vec::new();
    if !failures.is_empty() {
        parts.push(format!(
            "backend build failed for {} language(s): {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    if !unmet.is_empty() {
        parts.push(format!(
            "{} language(s) not built -- preconditions are unmet, no build attempted (not a compile \
             failure): {}",
            unmet.len(),
            unmet.join("; ")
        ));
    }
    if strict && !toolchain_missing.is_empty() {
        parts.push(format!(
            "--strict is set: {} language(s) skipped, toolchain not on PATH, no build attempted: {}",
            toolchain_missing.len(),
            toolchain_missing.join(", ")
        ));
    }
    if parts.is_empty() {
        return Ok(());
    }
    anyhow::bail!("{}", parts.join(" | "));
}

/// Whether a backend can be built here at all, and if not, which kind of "not" it is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendReadiness {
    Ready,
    /// The tool this backend builds with is not installed on this machine. Not the checkout's
    /// fault and not fixable from inside it — skipped, non-fatal by default. Fatal under
    /// `--strict` (see `build_outcome`), for CI runs where "not built" must not read as success.
    /// `precondition` is the failing check itself (e.g. `command -v gradle`), carried through so
    /// a `--strict` failure names what was missing rather than just which language was skipped.
    /// ~keep
    ToolchainMissing {
        precondition: String,
    },
    /// The tool is here and the checkout is not prepared for it: dependencies were never fetched,
    /// or the interpreter environment the build installs into does not exist. Fixable by
    /// `remediation`, and fatal so that nothing downstream mistakes the missing artifact for a
    /// built one. ~keep
    DependenciesUnfetched {
        check: String,
        remediation: String,
    },
}

/// Classify a backend before dispatching it.
///
/// The tool check runs first: a dependency check phrased against a missing tool's project layout
/// would report the wrong cause. ~keep
fn backend_readiness(lang: Language, build_cmd_cfg: &BuildCommandConfig) -> BackendReadiness {
    if !check_precondition(lang, build_cmd_cfg.precondition.as_deref()) {
        // `check_precondition` only returns `false` when a precondition was declared and it
        // failed, so this is never the "no precondition configured" case -- `unwrap_or_default`
        // exists to satisfy the borrow checker, not to paper over a real `None`. ~keep
        return BackendReadiness::ToolchainMissing {
            precondition: build_cmd_cfg.precondition.clone().unwrap_or_default(),
        };
    }
    let Some(check) = build_cmd_cfg.dependency_precondition.as_deref() else {
        return BackendReadiness::Ready;
    };
    if precondition_passes(&lang.to_string(), check) {
        return BackendReadiness::Ready;
    }
    // Config validation rejects a `dependency_precondition` without a `dependency_remediation`, so
    // this fallback is unreachable through a loaded config — it exists so a future built-in that
    // forgets the pair degrades into a vague message rather than a panic. ~keep
    let remediation = build_cmd_cfg
        .dependency_remediation
        .clone()
        .unwrap_or_else(|| format!("(no `dependency_remediation` declared for {lang})"));
    BackendReadiness::DependenciesUnfetched {
        check: check.to_string(),
        remediation,
    }
}

/// What happened while running one language's post-build steps.
///
/// A [`PostBuildStep::RunCommand`] whose tool isn't on `PATH` is treated as success (falling
/// back to whatever generated output is already committed is deliberate -- see
/// `run_run_command`'s doc comment), but that made a skip indistinguishable from a step that
/// actually ran and produced current output. `skipped_missing_tools` names every such tool so a
/// caller can report it instead of silently treating the run as fully up to date.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PostBuildOutcome {
    /// Commands that were skipped because they were not found on `PATH`, in the order their
    /// `RunCommand` steps ran.
    pub skipped_missing_tools: Vec<String>,
}

/// Fold one language's `run_post_build` result into `build_with_environment`'s shared
/// `failures`/`skipped_post_build_tools` buckets.
///
/// Before this helper existed, both call sites in `build_with_environment` matched only the
/// `Err` arm of `run_post_build` (`if let Err(err) = run_post_build(...) { failures.push(...) }`)
/// and threw the `Ok` arm away wholesale -- including `PostBuildOutcome::skipped_missing_tools`.
/// That made `alef build` report a clean success for a language whose post-build tool (e.g.
/// `flutter_rust_bridge_codegen`) was missing from `PATH` and silently fell back to stale
/// committed generated output, with zero warning and zero exit-code signal: `alef generate`/
/// `alef all` already surfaced this via `bin_cli::helpers::post_build::run_resolved_post_builds`,
/// but `alef build` has its own independent call sites and inherited none of that. Kept
/// non-fatal, matching `run_run_command`'s doc comment and `BackendReadiness::ToolchainMissing`'s
/// reasoning just above `build_outcome`: a missing toolchain is a statement about the machine,
/// not the checkout, so it stays a warning rather than a hard error. ~keep
fn record_post_build_outcome(
    lang: Language,
    result: anyhow::Result<PostBuildOutcome>,
    failures: &mut Vec<String>,
    skipped_post_build_tools: &mut Vec<String>,
) {
    match result {
        Ok(outcome) => {
            for tool in outcome.skipped_missing_tools {
                warn!(
                    "[{lang}] post-build completed but skipped '{tool}' (not on PATH) -- falling back to \
                     committed generated files"
                );
                skipped_post_build_tools.push(format!("{lang}: {tool}"));
            }
        }
        Err(err) => failures.push(format!("{lang}: post-build failed: {err:#}")),
    }
}

/// Run post-build processing steps (e.g., patching .d.ts files).
pub fn run_post_build(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    staging_profile: StagingProfile,
) -> anyhow::Result<PostBuildOutcome> {
    use crate::core::backend::PostBuildStep;

    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .unwrap_or(Path::new(""));

    let mut skipped_missing_tools: Vec<String> = Vec::new();

    for step in &bc.post_build {
        match step {
            PostBuildStep::PatchFile { path, find, replace } => {
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-build patch target {}", file_path.display()))?;
                    if content.contains(replace) {
                        debug!("Post-build patch target already patched: {}", file_path.display());
                        continue;
                    }
                    let patched = content.replace(find, replace);
                    if patched != content {
                        std::fs::write(&file_path, &patched)
                            .with_context(|| format!("failed to write patched file {}", file_path.display()))?;
                        info!("Patched {}: replaced '{}' → '{}'", file_path.display(), find, replace);
                    }
                } else {
                    debug!("Post-build patch target not found: {}", file_path.display());
                }
            }
            PostBuildStep::RunCommand { cmd, args } => {
                let work_dir = base_dir.join(crate_dir);
                // `[build_commands.<lang>].timeout_seconds` (alef.toml) overrides the built-in
                // `RUN_COMMAND_TIMEOUT` ceiling for exactly this step -- see that config field's
                // doc comment for why a language-scoped override exists (alef #364: a cold
                // Swift `cargo build --release` in a large workspace legitimately exceeds 30
                // minutes while still making progress). ~keep
                let timeout = config
                    .build_command_config_for_language(lang)
                    .timeout_seconds
                    .map(std::time::Duration::from_secs)
                    .unwrap_or(RUN_COMMAND_TIMEOUT);
                let ran = run_run_command(cmd, args, &work_dir, &config.name, timeout)
                    .with_context(|| format!("post-build RunCommand '{cmd}' failed"))?;
                if !ran {
                    skipped_missing_tools.push((*cmd).to_string());
                }
            }
            PostBuildStep::VerifyFrbCodegenVersion { expected_version } => {
                frb_version_check::run(frb_version_check::FLUTTER_RUST_BRIDGE_CODEGEN, expected_version)
                    .context("post-build VerifyFrbCodegenVersion failed")?;
            }
            PostBuildStep::PostProcessFile { path, processor } => {
                use crate::core::backend::PostProcessor;
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-process target {}", file_path.display()))?;
                    let processed = match processor {
                        PostProcessor::FrbDartSealedVariants => {
                            crate::backends::dart::rewrite_frb_sealed_variants(&content, &config.dart_pubspec_name())
                        }
                        PostProcessor::FrbDartExcludeFunctions(excluded) => {
                            let exclude_set: std::collections::HashSet<&str> =
                                excluded.iter().map(|s| s.as_str()).collect();
                            crate::backends::dart::filter_excluded_functions(&content, &exclude_set)
                        }
                        PostProcessor::FrbDartOptionalFieldsWithDefaults => {
                            crate::backends::dart::make_struct_fields_with_defaults_optional(&content)
                        }
                        PostProcessor::FrbDartFixHandlerExecutorCalls => {
                            crate::backends::dart::fix_handler_executor_calls(&content)
                        }
                        PostProcessor::FrbDartInjectTextMethods(type_names) => {
                            crate::backends::dart::inject_display_as_text_methods(&content, type_names)
                        }
                        PostProcessor::DartStripTrailingWhitespace => {
                            crate::backends::dart::strip_trailing_whitespace(&content)
                        }
                    };
                    if processed != content {
                        std::fs::write(&file_path, &processed)
                            .with_context(|| format!("failed to write post-processed file {}", file_path.display()))?;
                        info!("PostProcessed {}: {:?}", file_path.display(), processor);
                    } else {
                        debug!(
                            "PostProcessFile {}: no changes (already rewritten or absent variants)",
                            file_path.display()
                        );
                    }
                } else {
                    debug!("PostProcessFile target not found: {}", file_path.display());
                }
            }
            PostBuildStep::CarryFrbCfgGates {
                source_path,
                target_path,
            } => {
                let source_file = base_dir.join(crate_dir).join(source_path);
                let target_file = base_dir.join(crate_dir).join(target_path);
                frb_cfg_gates::run(&source_file, &target_file)?;
            }
            PostBuildStep::StageDartNatives { lib_stem } => {
                let package_root = base_dir.join("packages/dart");
                // Same `staging_profile` dispatch as `StageFfiLibrary` below -- a stale artifact
                // from the *other* profile must never silently satisfy this run's staging step.
                // ~keep
                let status = match staging_profile.just_built() {
                    Some(profile) => crate::publish::dart_native::stage_dart_native_libraries(
                        base_dir,
                        &package_root,
                        lib_stem,
                        profile,
                    ),
                    None => crate::publish::dart_native::stage_dart_native_libraries_preferring_release(
                        base_dir,
                        &package_root,
                        lib_stem,
                    ),
                }
                .with_context(|| format!("failed to stage Dart native libraries for stem '{lib_stem}'"))?;
                match status {
                    crate::publish::dart_native::NativeLibraryStageStatus::Staged => {
                        info!("Staged native libraries for Dart package from build output (stem: '{lib_stem}')");
                    }
                    crate::publish::dart_native::NativeLibraryStageStatus::Missing => {
                        debug!("No Dart native libraries available to stage for development stem '{lib_stem}'");
                    }
                }
            }
            PostBuildStep::StageFfiLibrary => {
                let target = crate::publish::platform::host_target()
                    .with_context(|| format!("failed to detect host Rust target for {lang} FFI staging"))?;
                // Which profile(s) to look for, and the matching build command to suggest in the
                // warning below if none is found -- kept together so the two can never name
                // different profiles. ~keep
                let (profile_description, build_hint) = match staging_profile.just_built() {
                    Some(profile) => (profile.to_string(), format!("alef build{}", profile.cargo_flag())),
                    None => ("release or debug".to_string(), "alef build --release".to_string()),
                };
                // `ffi_artifact_built*` (not a bare `stage_ffi*` + match-on-error) so this step
                // can tell "nothing was built this run" -- expected when this fires from `alef
                // generate`'s post-build pass, which never invokes `cargo build` -- apart from a
                // real copy failure once an artifact is known to exist. Only the latter is
                // allowed to fail this backend's build; the former is always a warning, never a
                // silent no-op. Deliberately does not fall back to `deps/`: see
                // `crate::publish::package::find_built_artifact`'s doc comment. ~keep
                let artifact_built = match staging_profile.just_built() {
                    Some(profile) => crate::publish::ffi_stage::ffi_artifact_built(config, &target, base_dir, profile),
                    None => crate::publish::ffi_stage::ffi_artifact_built_preferring_release(config, &target, base_dir),
                };
                if artifact_built {
                    let stage_result = match staging_profile.just_built() {
                        Some(profile) => crate::publish::ffi_stage::stage_ffi(config, lang, &target, base_dir, profile),
                        None => {
                            crate::publish::ffi_stage::stage_ffi_preferring_release(config, lang, &target, base_dir)
                        }
                    };
                    let dest = stage_result.with_context(|| format!("failed to stage FFI library for {lang}"))?;
                    info!(
                        "[{lang}] staged FFI library ({profile_description}) to {}",
                        dest.display()
                    );
                    match crate::publish::ffi_stage::stage_header(config, lang, &target, base_dir) {
                        Ok(Some(header)) => debug!("[{lang}] staged FFI header to {}", header.display()),
                        Ok(None) => {}
                        Err(err) => warn!("[{lang}] failed to stage FFI header: {err:#}"),
                    }
                } else {
                    let dest_description = crate::publish::ffi_stage::staging_dir(config, lang, &target, base_dir)
                        .map(|dir| dir.display().to_string())
                        .unwrap_or_else(|_| format!("{lang} native-library directory"));
                    // A miss is only news when the caller expected a build to have produced the
                    // artifact. `alef generate`/`alef all` never ask for one, so on any unbuilt
                    // tree this warning fired for a condition the invoked command was never
                    // going to satisfy and advised a build it deliberately does not run -- one
                    // unavoidable line per FFI-dependent language, on every generation-only run.
                    // Demoted to DEBUG for that caller alone: `alef test --e2e` is about to link
                    // this library, so its miss keeps the warning and the build hint. ~keep
                    if staging_profile.build_was_expected() {
                        warn!(
                            "[{lang}] no built FFI shared library found for target {} ({profile_description}); \
                             skipping staging into {} (run `{build_hint}` to produce one)",
                            target.triple, dest_description
                        );
                    } else {
                        debug!(
                            "[{lang}] no built FFI shared library on disk for target {} ({profile_description}); \
                             nothing to stage into {} -- no build was requested by this command",
                            target.triple, dest_description
                        );
                    }
                }
            }
            PostBuildStep::MaterializeSwiftBridge {
                binding_crate_name,
                package_root,
            } => {
                let package_root = base_dir.join(package_root);
                let materialized = crate::backends::swift::gen_bindings::bridge_artifacts::emit_swift_bridge_files(
                    "",
                    binding_crate_name,
                    &package_root,
                    true,
                )
                .with_context(|| format!("failed to re-materialize swift-bridge files for '{binding_crate_name}'"))?;
                if let Some(files) = materialized {
                    crate::backends::swift::gen_bindings::bridge_artifacts::write_materialized_files(files)
                        .with_context(|| format!("failed to write swift-bridge files for '{binding_crate_name}'"))?;
                }
                info!("Re-materialized swift-bridge files for '{binding_crate_name}' from fresh build output");
            }
            PostBuildStep::VerifyFrbBridgeCoverage {
                facade_path,
                bridge_path,
                exclude_functions,
            } => {
                let facade_file = base_dir.join(crate_dir).join(facade_path);
                let bridge_file = base_dir.join(crate_dir).join(bridge_path);
                frb_bridge_coverage::verify(&facade_file, &bridge_file, exclude_functions)?;
            }
            PostBuildStep::RewriteWasmPackageName {
                package_json_path,
                package_name,
            } => {
                // Unlike every other step above, `package_json_path` is already relative to
                // `base_dir` (not `crate_dir`): the wasm crate directory itself may be
                // `config.package_dir(Language::Wasm)`'s default-formula fallback rather than
                // `crate_dir` (empty whenever `[crates.output] wasm` isn't set explicitly — see
                // `build_command_for`'s "wasm-pack" arm), so the caller resolves the full path
                // up front instead of relying on this function's `crate_dir`. ~keep
                let file_path = base_dir.join(package_json_path);
                if file_path.exists() {
                    rewrite_wasm_package_json_name(&file_path, package_name)
                        .with_context(|| format!("failed to rewrite wasm package name in {}", file_path.display()))?;
                } else {
                    debug!(
                        "wasm-pack package.json not found for name rewrite: {}",
                        file_path.display()
                    );
                }
            }
        }
    }

    Ok(PostBuildOutcome { skipped_missing_tools })
}

/// Rewrite the `"name"` field of a wasm-pack-generated `package.json` in place.
///
/// wasm-pack always writes `"name"` as a plain top-level string field, but the value itself
/// (derived from the wasm crate's `Cargo.toml`) is not known until build time, so this can't
/// be a static [`PostBuildStep::PatchFile`] find/replace — the "find" half would have to be
/// discovered from the very file being patched. A regex on the `"name": "..."` field is a
/// minimal, order- and formatting-preserving edit; a full `serde_json` parse+reserialize would
/// risk reordering keys or changing indentation on every build. ~keep
fn rewrite_wasm_package_json_name(path: &Path, new_name: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let name_field = regex::Regex::new(r#""name"\s*:\s*"[^"]*""#).expect("static regex is valid");
    let escaped_name = new_name.replace('\\', "\\\\").replace('"', "\\\"");
    let replacement = format!("\"name\": \"{escaped_name}\"");
    let rewritten = name_field.replacen(&content, 1, replacement.as_str());
    if rewritten != content {
        std::fs::write(path, rewritten.as_ref()).with_context(|| format!("failed to write {}", path.display()))?;
        info!("Rewrote wasm package name in {} to '{new_name}'", path.display());
    } else {
        debug!(
            "wasm package.json {}: name already '{new_name}' or no name field found",
            path.display()
        );
    }
    Ok(())
}

/// Default hard upper bound on how long a post-build `RunCommand` may run before alef
/// considers it hung and kills it. Cold-cache `cargo build --release` for the
/// swift binding crate against a polyglot project's full feature set
/// legitimately takes 10-20 minutes; FRB codegen on a warm cache finishes in
/// under a minute. 30 minutes accommodates both without false-positiving
/// slow first-runs on cold CI caches.
///
/// A consumer whose cold Swift release build genuinely exceeds this (a large workspace can run
/// well past 30 minutes while still making progress -- alef #364) overrides it per language via
/// `[build_commands.<lang>].timeout_seconds` in `alef.toml`; see that field's doc comment. This
/// constant remains the ceiling whenever that field is unset. ~keep
const RUN_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

/// Execute a `RunCommand` post-build step.
///
/// Spawns `cmd` with `args` in `base_dir`, streaming stdout/stderr through
/// alef's own stdio so interactive subprocess progress is visible. Enforces `timeout` as a
/// ceiling; on timeout the child's whole process group is SIGKILL'd and the call returns an
/// error. Returns an error on non-zero exit status.
///
/// Returns `Ok(true)` when `cmd` actually ran (and exited zero), `Ok(false)` when it was
/// skipped -- either via the `ALEF_SKIP_COMMANDS` escape hatch below or because `cmd` isn't
/// on `PATH`. Both skips are deliberately non-fatal (falling back to whatever generated output
/// is already committed is the point), but the caller needs to tell "skipped" apart from "ran
/// and produced current output" -- see [`PostBuildOutcome`].
///
/// Escape hatch: the env var `ALEF_SKIP_COMMANDS` accepts a comma-separated
/// list of `cmd` names to skip without running. Useful in environments where
/// a post-build tool is unavailable, hangs (e.g. `flutter_rust_bridge_codegen`
/// installing Flutter via FVM under CI), or simply isn't desired this run.
/// Each skipped command logs a `warn!` so the omission is visible.
fn run_run_command(
    cmd: &str,
    args: &[&str],
    base_dir: &Path,
    cache_scope: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<bool> {
    if let Ok(skip_list) = std::env::var("ALEF_SKIP_COMMANDS")
        && skip_list.split(',').any(|s| s.trim() == cmd)
    {
        warn!("[{cmd}] skipped via ALEF_SKIP_COMMANDS env var");
        return Ok(false);
    }
    let mut command = std::process::Command::new(cmd);
    command
        .args(args)
        .current_dir(base_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    frb_cache::configure(&mut command, cmd, cache_scope)?;

    let mut child = match GroupChild::spawn(&mut command) {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "[{cmd}] not on PATH -- post-build step skipped, using the committed generated files. Install \
                 '{cmd}' to regenerate at build time."
            );
            return Ok(false);
        }
        Err(err) => return Err(anyhow::Error::new(err).context(format!("failed to spawn '{cmd}'"))),
    };

    // Mirrored to alef's own stderr as they arrive (so a cold release build still looks alive)
    // and captured at the same time, so a failure below can quote what the child actually said
    // instead of just its exit code. Taken before the wait, not after: a child that fills the OS
    // pipe buffer blocks on the write and never exits if nothing is draining it concurrently.
    let stdout = child.take_stdout().map(output_reader_tee);
    let stderr = child.take_stderr().map(output_reader_tee);

    // `flutter_rust_bridge_codegen` and the cargo builds it drives are trees, not processes:
    // killing the direct child leaves whatever it started running past this ceiling. ~keep
    let waited = child
        .wait_within(timeout, &cmd)
        .with_context(|| format!("failed to wait for '{cmd}'"))?;
    let Deadline::Exited(status) = waited else {
        anyhow::bail!("'{cmd}' exceeded {}s timeout; killed", timeout.as_secs());
    };

    let drained = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE)
        .with_context(|| format!("failed to read the output of '{cmd}'"))?;
    if !drained.complete {
        warn!(
            command = cmd,
            grace_seconds = OUTPUT_DRAIN_GRACE.as_secs(),
            "a descendant outlived the command still holding its output pipes; killing the process group"
        );
        child.kill_tree();
    }

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        // Both streams, not just stderr: `flutter_rust_bridge_codegen` and cargo wrapped by
        // sccache both put real diagnostics on stdout, so stderr alone can be empty on the run
        // that most needs explaining. ~keep
        anyhow::bail!(
            "'{cmd}' exited with status {code}\n--- stderr (tail) ---\n{}\n--- stdout (tail) ---\n{}",
            tail(&drained.stderr, ERROR_TAIL_BYTES),
            tail(&drained.stdout, ERROR_TAIL_BYTES)
        );
    }

    Ok(true)
}

/// How much of a failed command's captured stdout/stderr to quote in its error, from the end of
/// each stream.
///
/// Enough to carry a real compiler diagnostic (the whole point) without letting a linker
/// invocation's thousands of lines of "ignoring duplicate libraries" and "built for newer macOS
/// version" warnings — a real case that motivated this — balloon the propagated error. ~keep
const ERROR_TAIL_BYTES: usize = 4096;

/// Keeps roughly the last `max_bytes` of `text`, snapped forward to the next line boundary so a
/// truncated diagnostic reads as whole lines instead of a byte fragment.
fn tail(text: &str, max_bytes: usize) -> &str {
    let Some(mut start) = text.len().checked_sub(max_bytes) else {
        return text;
    };
    // `text.len() - max_bytes` can land inside a multi-byte character; a `String` decoded
    // lossily from a compiler's output is not guaranteed ASCII. ~keep
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    match text[start..].find('\n') {
        Some(offset) => &text[start + offset + 1..],
        None => &text[start..],
    }
}
