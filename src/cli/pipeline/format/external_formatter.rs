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
