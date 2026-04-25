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

    let mut config = format!("[alef]\nversion = \"{}\"\n\n", env!("CARGO_PKG_VERSION"));

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
    fn generate_init_config_includes_alef_version() {
        let config = generate_init_config("my-lib", "1.0.0", &["python".to_string()]);
        assert!(
            config.starts_with("[alef]\n"),
            "config should start with [alef] section"
        );
        let expected_version = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
        assert!(
            config.contains(&expected_version),
            "config should contain alef version from CARGO_PKG_VERSION"
        );
    }

    #[test]
    fn generate_init_config_parses_as_valid_alef_config() {
        let config_str = generate_init_config("my-lib", "1.0.0", &["python".to_string()]);
        let config: alef_core::config::AlefConfig =
            toml::from_str(&config_str).expect("generated config should parse as valid AlefConfig");
        assert_eq!(config.alef.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(config.crate_config.name, "my-lib");
    }
}
