use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed_with_env};
use crate::cli::registry;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::publish::ffi_stage;
use crate::publish::platform::RustTarget;
use anyhow::Context as _;
use rayon::prelude::*;
use tracing::{info, warn};

pub fn test(config: &ResolvedCrateConfig, languages: &[Language], e2e: bool, coverage: bool) -> anyhow::Result<()> {
    let pdfium_dir = compute_pdfium_dir();
    let mut env_vars: Vec<(&str, String)> = Vec::new();

    if let Some(lib_dir) = pdfium_dir {
        #[cfg(target_os = "macos")]
        {
            env_vars.push(("DYLD_FALLBACK_LIBRARY_PATH", lib_dir.clone()));
            env_vars.push(("DYLD_LIBRARY_PATH", lib_dir));
        }
        #[cfg(target_os = "linux")]
        {
            env_vars.push(("LD_LIBRARY_PATH", lib_dir));
        }
        #[cfg(target_os = "windows")]
        {
            env_vars.push(("PATH", lib_dir));
        }
    }

    let base_dir = std::env::current_dir()?;

    let langs_to_test: Vec<Language> = languages
        .iter()
        .copied()
        .filter(|lang| {
            let lang_test = config.test_config_for_language(*lang);
            check_precondition(*lang, lang_test.precondition.as_deref())
        })
        .collect();
    for &lang in &langs_to_test {
        let lang_test = config.test_config_for_language(lang);
        if let Err(e) = run_before(lang, lang_test.before.as_ref()) {
            return Err(e).with_context(|| format!("before hook failed for {lang}"));
        }
    }

    if e2e {
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() {
                continue;
            }
            let Some(backend) = registry::try_get_backend(lang) else {
                continue;
            };
            let Some(bc) = backend.build_config_with_config(config) else {
                continue;
            };
            if bc.post_build.is_empty() {
                continue;
            }
            if let Err(e) = super::run_post_build(lang, &bc, config, &base_dir) {
                eprintln!("  [{lang}] post-build processing failed before e2e tests: {e}");
                return Err(e).with_context(|| format!("post-build failed for {lang}"));
            }
        }

        let host_target = get_host_target().context("failed to detect host target for FFI staging")?;
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() {
                continue;
            }
            if !matches!(lang, Language::Go | Language::Java | Language::Csharp) {
                continue;
            }
            let workspace_root = std::env::current_dir().ok().and_then(|cwd| {
                let mut current = cwd;
                loop {
                    if current.join("target").exists() && current.join("alef.toml").exists() {
                        return Some(current);
                    }
                    if !current.pop() {
                        return None;
                    }
                }
            });
            if let Some(workspace_root) = workspace_root {
                match ffi_stage::stage_ffi(config, lang, &host_target, &workspace_root) {
                    Ok(dest) => {
                        info!("[{lang}] staged FFI artifacts to {}", dest.display());
                    }
                    Err(e) => {
                        warn!("[{lang}] failed to stage FFI artifacts: {e}");
                    }
                }
                if let Ok(Some(header)) = ffi_stage::stage_header(config, lang, &host_target, &workspace_root) {
                    info!("[{lang}] staged FFI header to {}", header.display());
                }
            }
        }
    }

    let results: Vec<(Language, anyhow::Result<()>)> = langs_to_test
        .par_iter()
        .map(|lang| {
            let label = lang.to_string();
            let lang_test = config.test_config_for_language(*lang);

            let test_cmds = if coverage {
                lang_test.coverage.as_ref().or(lang_test.command.as_ref())
            } else {
                lang_test.command.as_ref()
            };

            if let Some(cmd_list) = test_cmds {
                for cmd in cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_env(cmd, Some(&label), &env_vars) {
                        return (*lang, Err(e));
                    }
                }
            }
            if e2e && let Some(e2e_cmd_list) = &lang_test.e2e {
                for cmd in e2e_cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_env(cmd, Some(&label), &env_vars) {
                        return (*lang, Err(e));
                    }
                }
            }
            (*lang, Ok(()))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        if let Err(e) = result {
            eprintln!("✗ test failed: {lang} — {e}");
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

/// Compute the target/release directory path from the workspace root.
///
/// Walks up from the current working directory to find any `target/release/`
/// containing a dynamic library (regardless of which one — pdfium, an FFI crate,
/// etc.). This directory is added to the dynamic library search path so that
/// e2e test runners (dotnet test, JVM, Node, …) can dlopen the workspace's
/// native libraries.
fn compute_pdfium_dir() -> Option<String> {
    use std::env;

    let (lib_prefix, lib_ext): (&str, &str) = if cfg!(target_os = "macos") {
        ("lib", ".dylib")
    } else if cfg!(target_os = "windows") {
        ("", ".dll")
    } else {
        ("lib", ".so")
    };

    let mut current = env::current_dir().ok()?;

    loop {
        let target_release = current.join("target").join("release");
        if target_release.exists() {
            if let Ok(entries) = std::fs::read_dir(&target_release) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let Some(name_str) = name.to_str() else { continue };
                    if name_str.starts_with(lib_prefix) && name_str.ends_with(lib_ext) {
                        if let Some(path_str) = target_release.to_str() {
                            info!("Native library directory: {}", path_str);
                            return Some(path_str.to_string());
                        }
                    }
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    None
}

/// Get the current host Rust target triple by parsing rustc output.
///
/// Parses `rustc --version --verbose` to extract the `host:` line,
/// returning the target triple (e.g. `aarch64-apple-darwin`).
fn get_host_target() -> anyhow::Result<RustTarget> {
    use std::process::Command;

    let output = Command::new("rustc")
        .arg("--version")
        .arg("--verbose")
        .output()
        .context("failed to run rustc --version --verbose")?;

    if !output.status.success() {
        anyhow::bail!("rustc --version --verbose exited with non-zero status");
    }

    let stdout = String::from_utf8(output.stdout).context("rustc output is not valid UTF-8")?;

    for line in stdout.lines() {
        if let Some(triple) = line.strip_prefix("host:") {
            let triple = triple.trim();
            return RustTarget::parse(triple).with_context(|| format!("failed to parse host target triple: {triple}"));
        }
    }

    anyhow::bail!("rustc --version --verbose did not output a 'host:' line")
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;
    #[cfg(unix)]
    use crate::core::config::NewAlefConfig;

    /// Build a ResolvedCrateConfig that has `before` and `e2e` wired for `python`
    /// using the given shell commands.  `command` is intentionally absent so the
    /// test exercises the `before` -> `e2e` path in isolation (no unit-test phase).
    #[cfg(unix)]
    fn make_config_with_before_and_e2e(before_cmd: &str, e2e_cmd: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
before = ["{before_cmd}"]
e2e = "{e2e_cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// Verify that `before` hooks run before `e2e` commands.
    ///
    /// Before this fix, `before` entries in `[crates.test.<lang>]` were only
    /// documented as running prior to `command`; e2e invocations could start
    /// without the setup steps completing first (e.g. a Kotlin Android binding
    /// can need its FFI library built and symlinked before Gradle loads JNI).
    /// Phase 1 runs `before`
    /// sequentially for every language before Phase 2 executes either
    /// `command` or `e2e`, so both invocation paths receive the same setup.
    #[cfg(unix)]
    #[test]
    fn before_hook_runs_before_e2e_command() {
        let tmp = std::env::temp_dir().join(format!("alef_before_e2e_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        let order_file = tmp.join("order.txt");
        std::fs::write(&order_file, "").expect("create order file");

        let path_str = order_file.display().to_string().replace('\'', "'\\''");

        let before_cmd = format!("printf 'A\\n' >> '{path_str}'");
        let e2e_cmd = format!("printf 'B\\n' >> '{path_str}'");

        let config = make_config_with_before_and_e2e(&before_cmd, &e2e_cmd);

        test(&config, &[Language::Python], true, false)
            .expect("test() should succeed when before and e2e commands exit 0");

        let content = std::fs::read_to_string(&order_file).expect("read order file");
        let lines: Vec<&str> = content.lines().collect();

        std::fs::remove_dir_all(&tmp).ok();

        assert_eq!(
            lines,
            vec!["A", "B"],
            "before hook must run before e2e command; got order: {lines:?}"
        );
    }
}
