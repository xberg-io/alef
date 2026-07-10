use crate::cli::pipeline::helpers::{self, check_precondition};
use crate::core::config::output::StringOrVec;
use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use rayon::prelude::*;
use tracing::{info, warn};

/// Install dependencies for each language.
///
/// If `timeout_override` is Some, all languages use that timeout; otherwise each
/// language uses its configured `timeout_seconds` (defaulting to 600 seconds).
pub fn setup(
    config: &ResolvedCrateConfig,
    languages: &[Language],
    timeout_override: Option<u64>,
) -> anyhow::Result<()> {
    let base_dir = std::env::current_dir()?;
    let results: Vec<(Language, anyhow::Result<()>)> = languages
        .par_iter()
        .map(|lang| {
            let label = lang.to_string();
            let setup_cfg = config.setup_config_for_language(*lang);
            let timeout_secs = timeout_override.unwrap_or(setup_cfg.timeout_seconds);

            if !check_precondition(*lang, setup_cfg.precondition.as_deref()) {
                return (*lang, Ok(()));
            }
            let cwd: Option<std::path::PathBuf> = setup_cfg.workdir.as_ref().and_then(|w| {
                let joined = base_dir.join(w);
                if joined.exists() {
                    Some(joined)
                } else {
                    warn!(
                        "setup workdir {} for {lang} does not exist; running from repo root",
                        joined.display()
                    );
                    None
                }
            });
            let result: anyhow::Result<()> = (|| {
                run_before_with_timeout(*lang, setup_cfg.before.as_ref(), timeout_secs)?;
                if let Some(cmd_list) = &setup_cfg.install {
                    for cmd in cmd_list.commands() {
                        helpers::run_command_streamed_with_cwd_and_timeout(
                            cmd,
                            Some(&label),
                            Some(timeout_secs),
                            cwd.as_deref(),
                        )
                        .with_context(|| format!("setup for {lang} timed out after {timeout_secs}s"))?;
                    }
                }
                Ok(())
            })();
            (*lang, result)
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        if let Err(e) = result {
            eprintln!("✗ setup failed: {lang} — {e}");
            warn!("Setup failed for {lang}: {e}");
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Run before-hook commands with timeout consideration. Returns `Ok(())` on success.
fn run_before_with_timeout(lang: Language, before: Option<&StringOrVec>, timeout_secs: u64) -> anyhow::Result<()> {
    let Some(cmds) = before else {
        return Ok(());
    };
    for cmd in cmds.commands() {
        info!("Running before hook for {lang}: {cmd}");
        let (stdout, stderr) = helpers::run_command_captured_with_timeout(cmd, Some(timeout_secs))
            .with_context(|| format!("before hook timed out for {lang} after {timeout_secs}s: {cmd}"))?;
        if !stdout.is_empty() {
            info!("[{lang} before] {stdout}");
        }
        if !stderr.is_empty() {
            info!("[{lang} before] {stderr}");
        }
    }
    Ok(())
}
