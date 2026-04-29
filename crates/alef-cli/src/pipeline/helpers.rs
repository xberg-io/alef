use alef_core::config::Language;
use alef_core::config::output::StringOrVec;
use anyhow::Context as _;
use tracing::{info, warn};

/// Run a shell command, logging and failing on non-zero exit.
pub(crate) fn run_command(cmd: &str) -> anyhow::Result<()> {
    info!("Running: {cmd}");
    let status = std::process::Command::new("sh").args(["-c", cmd]).status()?;
    if !status.success() {
        anyhow::bail!("Command failed: {cmd}");
    }
    Ok(())
}

/// Run a shell command with stdout/stderr streamed to the parent's stderr in
/// real time, optionally line-prefixed with `[label] `.
///
/// Use this for long-running, user-facing commands (`pnpm install`, `bundle
/// install`, `cargo update`, formatters, linters) where blocking until exit
/// to print output makes the CLI feel hung. When `label` is `None` the child's
/// streams are inherited directly (zero overhead). When `label` is `Some`,
/// stdout/stderr are piped and pumped to the parent's stderr by two reader
/// threads so concurrent runs from different languages don't interleave
/// per-line.
pub(crate) fn run_command_streamed(cmd: &str, label: Option<&str>) -> anyhow::Result<()> {
    info!("Running: {cmd}");
    let mut command = std::process::Command::new("sh");
    command.args(["-c", cmd]);

    let Some(prefix) = label else {
        let status = command
            .status()
            .with_context(|| format!("failed to spawn: {cmd}"))?;
        if !status.success() {
            anyhow::bail!("Command failed: {cmd}");
        }
        return Ok(());
    };

    let prefix = format!("[{prefix}] ");
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn: {cmd}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let p1 = prefix.clone();
    let h_out = stdout.map(|s| std::thread::spawn(move || pump_lines(s, &p1)));
    let p2 = prefix.clone();
    let h_err = stderr.map(|s| std::thread::spawn(move || pump_lines(s, &p2)));

    let status = child.wait().with_context(|| format!("failed to wait on: {cmd}"))?;
    if let Some(h) = h_out {
        let _ = h.join();
    }
    if let Some(h) = h_err {
        let _ = h.join();
    }
    if !status.success() {
        anyhow::bail!("Command failed: {cmd}");
    }
    Ok(())
}

fn pump_lines<R: std::io::Read>(reader: R, prefix: &str) {
    use std::io::{BufRead, BufReader, Write};
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    let stderr = std::io::stderr();
    loop {
        line.clear();
        match buf.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let mut lock = stderr.lock();
                let _ = lock.write_all(prefix.as_bytes());
                let _ = lock.write_all(line.as_bytes());
                if !line.ends_with('\n') {
                    let _ = lock.write_all(b"\n");
                }
            }
        }
    }
}

/// Streamed variant of `run_command_captured_with_timeout`. Output is piped to
/// the parent's stderr live (line-prefixed when `label` is set), and the child
/// is killed if the deadline elapses.
pub(crate) fn run_command_streamed_with_timeout(
    cmd: &str,
    label: Option<&str>,
    timeout_secs: Option<u64>,
) -> anyhow::Result<()> {
    let Some(secs) = timeout_secs else {
        return run_command_streamed(cmd, label);
    };
    info!("Running (timeout {secs}s): {cmd}");
    let prefix = label.map(|l| format!("[{l}] "));

    let mut command = std::process::Command::new("sh");
    command.args(["-c", cmd]);

    let mut child = if prefix.is_some() {
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn: {cmd}"))?
    } else {
        command.spawn().with_context(|| format!("failed to spawn: {cmd}"))?
    };

    let h_out = if let (Some(p), Some(s)) = (prefix.clone(), child.stdout.take()) {
        Some(std::thread::spawn(move || pump_lines(s, &p)))
    } else {
        None
    };
    let h_err = if let (Some(p), Some(s)) = (prefix.clone(), child.stderr.take()) {
        Some(std::thread::spawn(move || pump_lines(s, &p)))
    } else {
        None
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait()? {
            Some(status) => {
                if let Some(h) = h_out {
                    let _ = h.join();
                }
                if let Some(h) = h_err {
                    let _ = h.join();
                }
                if !status.success() {
                    anyhow::bail!("Command failed: {cmd}");
                }
                return Ok(());
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("Command timed out after {secs}s: {cmd}");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// Run a shell command with an optional timeout.
///
/// If `timeout_secs` is `Some(n)`, kills the child process after `n` seconds and
/// returns a "timed out" error. Otherwise behaves identically to
/// [`run_command_captured`].
pub(crate) fn run_command_captured_with_timeout(
    cmd: &str,
    timeout_secs: Option<u64>,
) -> anyhow::Result<(String, String)> {
    let Some(secs) = timeout_secs else {
        return run_command_captured(cmd);
    };
    info!("Running (timeout {secs}s): {cmd}");
    let mut child = std::process::Command::new("sh")
        .args(["-c", cmd])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn: {cmd}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait()? {
            Some(status) => {
                let output = child.wait_with_output()?;
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                if !status.success() {
                    anyhow::bail!("Command failed: {cmd}\n{stderr}");
                }
                return Ok((stdout, stderr));
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("Command timed out after {secs}s: {cmd}");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// Run a shell command, capturing stdout and stderr.
///
/// Returns the captured output on success.  On failure the error includes
/// the command string and captured stderr for diagnostics.
pub(crate) fn run_command_captured(cmd: &str) -> anyhow::Result<(String, String)> {
    info!("Running: {cmd}");
    let output = std::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .with_context(|| format!("failed to spawn: {cmd}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        anyhow::bail!("Command failed: {cmd}\n{stderr}");
    }
    Ok((stdout, stderr))
}

/// Check a precondition command. Returns `true` if the command succeeds (or
/// is absent), `false` if it fails (language should be skipped).
pub(crate) fn check_precondition(lang: Language, precondition: Option<&str>) -> bool {
    let Some(cmd) = precondition else {
        return true;
    };
    info!("Checking precondition for {lang}: {cmd}");
    let status = std::process::Command::new("sh")
        .args(["-c", cmd])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => true,
        _ => {
            warn!("Skipping {lang}: precondition failed ({cmd})");
            false
        }
    }
}

/// Run before-hook commands. Returns `Ok(())` on success, or an error if any
/// command fails (which should abort the operation for this language).
pub(crate) fn run_before(lang: Language, before: Option<&StringOrVec>) -> anyhow::Result<()> {
    let Some(cmds) = before else {
        return Ok(());
    };
    for cmd in cmds.commands() {
        info!("Running before hook for {lang}: {cmd}");
        let (stdout, stderr) =
            run_command_captured(cmd).with_context(|| format!("before hook failed for {lang}: {cmd}"))?;
        if !stdout.is_empty() {
            info!("[{lang} before] {stdout}");
        }
        if !stderr.is_empty() {
            info!("[{lang} before] {stderr}");
        }
    }
    Ok(())
}

/// Initialize a new alef.toml config file.
pub fn init(config_path: &std::path::Path, languages: Option<Vec<String>>) -> anyhow::Result<()> {
    // Read crate name and version from Cargo.toml
    let (crate_name, crate_version) = read_crate_metadata()?;

    // Use provided languages or default to ["python", "node", "ffi"]
    let langs = languages.unwrap_or_else(|| vec!["python".to_string(), "node".to_string(), "ffi".to_string()]);

    // Generate config content
    let config_content = generate_init_config(&crate_name, &crate_version, &langs);

    // Write to alef.toml
    std::fs::write(config_path, config_content)
        .with_context(|| format!("failed to write config to {}", config_path.display()))?;
    info!("Created {}", config_path.display());

    Ok(())
}

fn read_crate_metadata() -> anyhow::Result<(String, String)> {
    let content = std::fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let value: toml::Value = toml::from_str(&content).context("failed to parse Cargo.toml")?;

    // Try workspace.package first
    if let Some(name) = value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
    {
        if let Some(version) = value
            .get("workspace")
            .and_then(|w| w.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
        {
            return Ok((name.to_string(), version.to_string()));
        }
    }

    // Try package directly
    if let Some(name) = value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
    {
        if let Some(version) = value
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
        {
            return Ok((name.to_string(), version.to_string()));
        }
    }

    anyhow::bail!("Could not find package name and version in Cargo.toml")
}

fn generate_init_config(crate_name: &str, _crate_version: &str, languages: &[String]) -> String {
    let source_path = format!("crates/{}/src/lib.rs", crate_name);

    let mut config = format!("version = \"{}\"\n", env!("CARGO_PKG_VERSION"));

    config.push_str("languages = [");

    for (i, lang) in languages.iter().enumerate() {
        if i > 0 {
            config.push_str(", ");
        }
        config.push('"');
        config.push_str(lang);
        config.push('"');
    }
    config.push_str("]\n\n");

    config.push_str(&format!(
        "[crate]\nname = \"{}\"\nsources = [\"{}\"]\nversion_from = \"Cargo.toml\"\n",
        crate_name, source_path
    ));

    // Add language-specific configs
    if languages.contains(&"python".to_string()) {
        config.push_str(&format!(
            "\n[python]\nmodule_name = \"_{}\"\n",
            crate_name.replace('-', "_")
        ));
    }

    if languages.contains(&"node".to_string()) {
        config.push_str(&format!("\n[node]\npackage_name = \"{crate_name}\"\n"));
    }

    if languages.contains(&"ffi".to_string()) {
        config.push_str(&format!("\n[ffi]\nprefix = \"{}\"\n", crate_name.replace('-', "_")));
    }

    if languages.contains(&"go".to_string()) {
        config.push_str(&format!(
            "\n[go]\nmodule = \"github.com/kreuzberg-dev/{}\"\n",
            crate_name
        ));
    }

    if languages.contains(&"ruby".to_string()) {
        config.push_str(&format!("\n[ruby]\ngem_name = \"{}\"\n", crate_name.replace('-', "_")));
    }

    if languages.contains(&"java".to_string()) {
        config.push_str("\n[java]\npackage = \"dev.kreuzberg\"\n");
    }

    if languages.contains(&"csharp".to_string()) {
        config.push_str(&format!("\n[csharp]\nnamespace = \"{}\"\n", to_pascal_case(crate_name)));
    }

    // Global tooling preferences. All fields are optional; the defaults shown
    // match alef's built-in behavior — uncomment to override.
    config.push_str(
        "\n[tools]\n\
         # python_package_manager = \"uv\"   # uv | pip | poetry\n\
         # node_package_manager = \"pnpm\"   # pnpm | npm | yarn\n\
         # rust_dev_tools = [\"cargo-edit\", \"cargo-sort\", \"cargo-machete\", \"cargo-deny\", \"cargo-llvm-cov\"]\n",
    );

    config
}

fn to_pascal_case(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_precondition_with_none_returns_true() {
        assert!(
            check_precondition(Language::Python, None),
            "None precondition should always pass"
        );
    }

    #[test]
    fn check_precondition_with_true_command_returns_true() {
        assert!(
            check_precondition(Language::Python, Some("true")),
            "Precondition 'true' should succeed"
        );
    }

    #[test]
    fn check_precondition_with_false_command_returns_false() {
        assert!(
            !check_precondition(Language::Python, Some("false")),
            "Precondition 'false' should fail"
        );
    }

    #[test]
    fn run_before_with_none_returns_ok() {
        run_before(Language::Python, None).expect("run_before with None should return Ok");
    }

    #[test]
    fn run_before_with_successful_single_command_returns_ok() {
        let cmd = StringOrVec::Single("true".to_string());
        run_before(Language::Python, Some(&cmd)).expect("run_before with 'true' should return Ok");
    }

    #[test]
    fn run_before_with_failing_single_command_returns_err() {
        let cmd = StringOrVec::Single("false".to_string());
        let result = run_before(Language::Python, Some(&cmd));
        assert!(result.is_err(), "run_before with 'false' should return Err");
    }

    #[test]
    fn run_before_with_multiple_commands_all_succeed_returns_ok() {
        let cmd = StringOrVec::Multiple(vec!["true".to_string(), "true".to_string()]);
        run_before(Language::Python, Some(&cmd)).expect("run_before with all-successful commands should return Ok");
    }

    #[test]
    fn run_before_aborts_on_first_failing_command() {
        // Second command would succeed but first fails, so Err is returned.
        let cmd = StringOrVec::Multiple(vec!["false".to_string(), "true".to_string()]);
        let result = run_before(Language::Python, Some(&cmd));
        assert!(
            result.is_err(),
            "run_before should abort and return Err when a command fails"
        );
    }

    #[test]
    fn check_precondition_works_for_non_python_language() {
        assert!(
            check_precondition(Language::Go, None),
            "None precondition should pass for Go"
        );
        assert!(
            check_precondition(Language::Go, Some("true")),
            "Precondition 'true' should pass for Go"
        );
        assert!(
            !check_precondition(Language::Go, Some("false")),
            "Precondition 'false' should fail for Go"
        );
    }

    #[test]
    fn generate_init_config_includes_version() {
        let config = generate_init_config("my-lib", "1.0.0", &["python".to_string()]);
        let expected = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
        assert!(config.starts_with(&expected), "config should start with version key");
    }

    #[test]
    fn generate_init_config_parses_as_valid_alef_config() {
        let config_str = generate_init_config("my-lib", "1.0.0", &["python".to_string()]);
        let config: alef_core::config::AlefConfig =
            toml::from_str(&config_str).expect("generated config should parse as valid AlefConfig");
        assert_eq!(config.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(config.crate_config.name, "my-lib");
    }

    #[test]
    fn run_command_captured_with_timeout_succeeds_within_limit() {
        // A command that completes quickly should succeed even with a timeout
        let result = run_command_captured_with_timeout("echo hello", Some(5));
        assert!(result.is_ok(), "Quick command should succeed with timeout");
        let (stdout, _) = result.unwrap();
        assert!(stdout.contains("hello"), "Command output should be captured");
    }

    #[test]
    fn run_command_captured_with_timeout_kills_on_timeout() {
        // A command that takes longer than the timeout should fail
        let result = run_command_captured_with_timeout("sleep 5", Some(1));
        assert!(result.is_err(), "Command that exceeds timeout should return error");
        let err_msg = format!("{:?}", result);
        assert!(err_msg.contains("timed out"), "Error should mention timeout");
    }

    #[test]
    fn run_command_captured_without_timeout() {
        // Commands without a timeout should work as before
        let result = run_command_captured_with_timeout("echo test", None);
        assert!(result.is_ok(), "Command without timeout should succeed");
        let (stdout, _) = result.unwrap();
        assert!(stdout.contains("test"), "Command output should be captured");
    }
}
