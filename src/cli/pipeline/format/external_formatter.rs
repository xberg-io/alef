//! Invoking an external formatter and reporting what it said.
//!
//! Split out of `format.rs`, which sits at the 1000-line cap. This is the whole "shell out to a
//! formatter binary, and turn a non-zero exit into a readable error" concern — it knows nothing
//! about which languages alef formats or when, which is why it separates cleanly. ~keep

use std::path::{Path, PathBuf};
use std::process::Output;

/// Run a formatter command with arguments in a specific directory.
pub(super) fn run_formatter(command: &str, args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    // ~keep Resolved, not bare: `is_tool_available` finds a tool through `which`, which
    // honours all of PATHEXT, while `Command::new` on Windows appends only `.exe`. A
    // formatter shipped as `mix.bat`/`npm.cmd` therefore passed the availability check and
    // then failed to spawn at all.
    let output = crate::core::tool_command(command)
        .args(args)
        .current_dir(work_dir)
        .output()?;

    if !output.status.success() {
        return Err(formatter_failure(&output));
    }

    Ok(())
}

/// What a `poly fmt` dry run reported through its exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PolyDryRun {
    /// Exit 0: nothing would change.
    Clean,
    /// Exit 1: at least one file would be rewritten.
    WouldChange,
    /// Any other exit (2 is a usage error or a file poly could not process), or no exit code.
    Failed,
}

pub(super) fn classify_poly_dry_run(exit_code: Option<i32>) -> PolyDryRun {
    match exit_code {
        Some(0) => PolyDryRun::Clean,
        Some(1) => PolyDryRun::WouldChange,
        _ => PolyDryRun::Failed,
    }
}

/// `poly fmt <flags...> <target>`: a dry run, because that is poly's default without `--fix`.
///
/// Never `--check`. poly up to 0.32 accepted it as a redundant spelling of the default; 0.33 removed
/// it and exits 2 ("`poly fmt --check` was removed"), which every caller that reads only the exit
/// status would mistake for "needs formatting" (alef #898). The bare dry run means the same on
/// both. ~keep
pub(super) fn poly_dry_run_args(flags: &[&str], target: &Path) -> Vec<String> {
    let mut args = vec!["fmt".to_owned()];
    args.extend(flags.iter().map(|flag| (*flag).to_owned()));
    args.push(target.to_string_lossy().into_owned());
    args
}

/// Run a `poly fmt` dry run and classify its exit status. A failure is logged with poly's own
/// output, so a usage error is visible instead of reading as a tree that needs formatting.
pub(super) fn run_poly_dry_run(args: &[&str], work_dir: &Path) -> PolyDryRun {
    let output = match crate::core::tool_command("poly")
        .args(args)
        .current_dir(work_dir)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!("could not run poly fmt dry run: {error}");
            return PolyDryRun::Failed;
        }
    };
    let outcome = classify_poly_dry_run(output.status.code());
    if outcome == PolyDryRun::Failed {
        tracing::warn!("poly fmt dry run failed: {}", formatter_failure(&output));
    }
    outcome
}

/// Run `poly` and return its stdout, for the one caller that reads a machine-readable payload
/// rather than only an exit status (`owner::PolyCoverage::probe`, which parses
/// `poly fmt --format json`).
///
/// Accepts exit code **1** as success, and that is not a leniency: a `poly fmt` dry run exits 1
/// whenever any file WOULD change, which on a real tree is the ordinary case. Treating 1 as
/// failure would discard a perfectly good JSON payload every time the tree was not already
/// pristine, and the probe would silently degrade to "coverage unknown" exactly when it had the
/// most to say. A crashed or missing poly still errors, because the payload is what is read --
/// `serde_json` rejects an empty or truncated body rather than yielding an empty coverage set.
/// Mirrors [`super::poly_format_exit_code_is_success`]'s contract for the same binary. ~keep
pub(super) fn run_poly_capturing(args: &[&str], work_dir: &Path) -> anyhow::Result<String> {
    let output = crate::core::tool_command("poly")
        .args(args)
        .current_dir(work_dir)
        .output()?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(formatter_failure(&output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(super) fn formatter_failure(output: &Output) -> anyhow::Error {
    anyhow::anyhow!(
        "formatter exited with code {:?}: {}",
        output.status.code(),
        format_command_output(output)
    )
}

pub(super) fn format_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
        (false, true) => format!("stdout:\n{stdout}"),
        (true, false) => format!("stderr:\n{stderr}"),
        (true, true) => "<no output>".to_string(),
    }
}

pub(super) fn resolve_crate_dir(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output_path.to_path_buf())
}
