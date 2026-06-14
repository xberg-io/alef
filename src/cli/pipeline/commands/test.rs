use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed_with_env};
use crate::cli::registry;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::publish::ffi_stage;
use crate::publish::platform::RustTarget;
use anyhow::Context as _;
use rayon::prelude::*;
use tracing::{info, warn};

pub fn test(config: &ResolvedCrateConfig, languages: &[Language], e2e: bool, coverage: bool) -> anyhow::Result<()> {
    // Compute pdfium dylib path from the workspace target directory.
    // Set DYLD_LIBRARY_PATH/LD_LIBRARY_PATH so pdfium-render can dlopen libpdfium.dylib at runtime.
    let pdfium_dir = compute_pdfium_dir();
    let mut env_vars: Vec<(&str, String)> = Vec::new();

    if let Some(lib_dir) = pdfium_dir {
        // Set platform-appropriate library search path
        #[cfg(target_os = "macos")]
        {
            // DYLD_LIBRARY_PATH is stripped by macOS SIP across some exec chains
            // (notably when child processes are signed/notarized).
            // DYLD_FALLBACK_LIBRARY_PATH is preserved and serves the same role for dlopen.
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

    // Phase 1 (sequential): run all per-language `before` hooks.
    //
    // `before` hooks run before BOTH `command` and `e2e` — any setup step
    // (building native libraries, creating symlinks, etc.) declared in
    // `[crates.test.<lang>] before = [...]` is guaranteed to complete before
    // either the unit-test commands (Phase 2) or the e2e test commands
    // (also Phase 2) execute.
    //
    // The test phase used to run before hooks inside the same `par_iter` as the
    // test commands. Most consumer `[crates.test.<lang>] before = ...` entries
    // invoke `cargo build` against the shared `target/` directory, so running
    // them in parallel produces a textbook cargo race: one process deletes a
    // dep info file while another is writing it, surfacing as
    // "could not parse/generate dep info" / "No such file or directory" /
    // "failed to write bytecode" and assorted half-baked artifacts (e.g. the
    // flutter_rust_bridge `lib.dart` containing mismatched `value`/`field0`
    // factory params, because a sibling cargo build trampled the FRB emit).
    // The build phase already runs its before hooks sequentially (see `build`
    // above); mirror that here so the test phase is equally safe.
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

    // Phase 1b (sequential, only when `--e2e`): run each language's post-build
    // step before the parallel test phase. Many post-build steps shell out to
    // `cargo build` against the shared `target/` directory (swift, kotlin
    // android …); running them inside Phase 2's `par_iter` produces the same
    // textbook cargo race the before-hook serialization (Phase 1) was added
    // to avoid: rustc temp files (`*.rcgu.o.*`) fail with "No such file or
    // directory" when a sibling cargo invocation cleans the deps dir under
    // them. Doing the post-build pass serially before parallel tests start
    // mirrors the build pipeline's own serialization of post-build steps.
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

        // Stage FFI artifacts for FFI-dependent languages before e2e tests
        let host_target = get_host_target().context("failed to detect host target for FFI staging")?;
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() {
                continue;
            }
            // Only stage for Go, Java, C# (FFI-dependent languages)
            if !matches!(lang, Language::Go | Language::Java | Language::Csharp) {
                continue;
            }
            let workspace_root = std::env::current_dir().ok().and_then(|cwd| {
                // Walk up to workspace root (directory containing target/ and alef.toml)
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
                        // Log as warning but don't fail — FFI may not be built yet
                        warn!("[{lang}] failed to stage FFI artifacts: {e}");
                    }
                }
                // Optionally stage the header
                if let Ok(Some(header)) = ffi_stage::stage_header(config, lang, &host_target, &workspace_root) {
                    info!("[{lang}] staged FFI header to {}", header.display());
                }
            }
        }
    }

    // Phase 2 (parallel): run per-language test commands.
    let results: Vec<(Language, anyhow::Result<()>)> = langs_to_test
        .par_iter()
        .map(|lang| {
            let label = lang.to_string();
            let lang_test = config.test_config_for_language(*lang);

            // Use coverage commands when --coverage flag is set, fall back to regular test
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
            // Accept the directory if it contains *any* native library matching
            // the platform's prefix/extension. We don't care which library — the
            // DYLD path is shared by every test process that dlopens FFI deps.
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
    use super::*;
    use crate::core::config::NewAlefConfig;

    /// Build a ResolvedCrateConfig that has `before` and `e2e` wired for `python`
    /// using the given shell commands.  `command` is intentionally absent so the
    /// test exercises the `before` → `e2e` path in isolation (no unit-test phase).
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
    /// without the setup steps completing first (e.g. the kreuzberg
    /// kotlin-android binding needs `cargo build -p kreuzberg-ffi` + a symlink
    /// before Gradle can load `kreuzberg_jni`).  Phase 1 runs `before`
    /// sequentially for every language before Phase 2 executes either
    /// `command` or `e2e`, so both invocation paths receive the same setup.
    #[cfg(unix)]
    #[test]
    fn before_hook_runs_before_e2e_command() {
        // Write a sentinel file to a temp path and record the append order.
        let tmp = std::env::temp_dir().join(format!("alef_before_e2e_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        let order_file = tmp.join("order.txt");
        std::fs::write(&order_file, "").expect("create order file");

        // Escape the path for shell: replace single-quotes with escaped form.
        let path_str = order_file.display().to_string().replace('\'', "'\\''");

        let before_cmd = format!("printf 'A\\n' >> '{path_str}'");
        let e2e_cmd = format!("printf 'B\\n' >> '{path_str}'");

        let config = make_config_with_before_and_e2e(&before_cmd, &e2e_cmd);

        test(&config, &[Language::Python], /* e2e= */ true, /* coverage= */ false)
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
