use crate::cli::pipeline::helpers::{
    check_precondition, run_before, run_command_streamed_with_env, run_command_streamed_with_envs,
};
use crate::cli::registry;
use crate::core::config::output::{StringOrVec, TestConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::publish::ffi_stage;
use anyhow::Context as _;
use rayon::prelude::*;
use tracing::{error, info, warn};

/// Whether the `command`/`coverage` phase would run for `lang_test` under `coverage`.
fn resolve_command_list(lang_test: &TestConfig, coverage: bool) -> Option<&StringOrVec> {
    if coverage {
        lang_test.coverage.as_ref().or(lang_test.command.as_ref())
    } else {
        lang_test.command.as_ref()
    }
}

/// Check the precondition gate for the `e2e` phase.
///
/// `precondition` is written for what `command`/`before` need, not for `e2e` -- a block that
/// tests Python via `uv` but lints with `ruff` has no reason for its `e2e` run to require `ruff`.
/// Gating `e2e` on the block's main `precondition` inherits a check that was authored for a
/// different command, which silently skips a suite that could otherwise run.
///
/// `e2e_precondition` scopes the tooling check to what `e2e` itself needs. When it is unset, this
/// deliberately does NOT fall back to `precondition`: falling back would reintroduce the exact bug
/// this field exists to fix. Instead `e2e` runs ungated. The failure mode this trades away is a
/// hard command failure (e.g. "napi: command not found") instead of a graceful skip -- worse
/// diagnostics, but not a false "everything passed" and not a silent no-op either; the missing
/// tool surfaces as a real, reported test failure. Blocks that want a graceful skip declare
/// `e2e_precondition` explicitly; `validate_test_e2e_precondition` requires one of the two
/// precondition fields whenever `e2e` is set, so this ungated path only applies when a
/// `precondition` covers the `command`/`before` phase but was never meant to gate `e2e` too.
fn check_e2e_precondition(lang: Language, lang_test: &TestConfig) -> bool {
    check_precondition(lang, lang_test.e2e_precondition.as_deref())
}

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

    // `[crates.e2e.env]` entries (e.g. an SSRF-policy allowlist var a fixture's loopback URL
    // needs) must reach the e2e process at spawn time, not just from generated in-tree setup
    // code -- some runtimes (Elixir's `mix test`, whose Rustler NIF loads during Mix's compile
    // phase, before `test_helper.exs` ever runs) load native code before any generated fixture
    // file gets a chance to set it via the language's own env APIs. `test_apps_run` (the
    // registry-mode runner) already sets these at spawn time; this mirrors that mechanism here
    // for the local `e2e/<lang>` runner so both paths agree.
    //
    // Deliberately NOT folded into `env_vars` above and passed through
    // `run_command_streamed_with_env`: that path's `inline_env_in_shell_cmd` uses a
    // PATH-style "prepend to the existing value" guard (`export K='V'"${K:+:$K}"`), which is
    // correct for search-path variables (a duplicated directory is harmless) but corrupts an
    // exact-value var -- `command.env()` sets the child's `K` to `V` before the script runs, so
    // the guard then reads that same `V` back and appends it to itself (`V:V`), never matching
    // a strict comparison an SSRF-policy check performs. Passing exact values only through
    // `Command::env` also keeps their contents out of the `sh -c` argument. ~keep
    // `e2e.env` is a `BTreeMap`, so `iter()` already yields key order -- no separate sort
    // needed to keep this deterministic. ~keep
    let e2e_env: Vec<(&str, String)> = config
        .e2e
        .as_ref()
        .map(|e2e| {
            e2e.env
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect()
        })
        .unwrap_or_default();

    let base_dir = std::env::current_dir()?;

    let langs_to_test: Vec<Language> = languages
        .iter()
        .copied()
        .filter(|lang| {
            let lang_test = config.test_config_for_language(*lang);
            let command_will_run = resolve_command_list(&lang_test, coverage).is_some();
            let e2e_will_run = e2e && lang_test.e2e.is_some();

            let command_gate = command_will_run && check_precondition(*lang, lang_test.precondition.as_deref());
            let e2e_gate = e2e_will_run && check_e2e_precondition(*lang, &lang_test);

            command_gate || e2e_gate
        })
        .collect();
    ensure_requested_suites_will_run(languages, &langs_to_test)?;
    for &lang in &langs_to_test {
        let lang_test = config.test_config_for_language(lang);
        if let Err(e) = run_before(lang, lang_test.before.as_ref()) {
            return Err(e).with_context(|| format!("before hook failed for {lang}"));
        }
    }

    if e2e {
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() || !check_e2e_precondition(lang, &lang_test) {
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
            // `alef test` never runs `cargo build` itself, so it cannot name a single profile the
            // way `alef build`'s own post-build dispatch can -- ask for whichever is on disk. ~keep
            if let Err(e) = super::run_post_build(lang, &bc, config, &base_dir, super::StagingProfile::PreferOnDisk) {
                warn!("[{lang}] post-build processing failed before e2e tests: {e}");
                return Err(e).with_context(|| format!("post-build failed for {lang}"));
            }
        }

        let host_target =
            crate::publish::platform::host_target().context("failed to detect host target for FFI staging")?;
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() || !check_e2e_precondition(lang, &lang_test) {
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
                // No `cargo build` runs here either -- same reasoning as the `run_post_build`
                // call above. ~keep
                match ffi_stage::stage_ffi_preferring_release(config, lang, &host_target, &workspace_root) {
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

            let test_cmds = resolve_command_list(&lang_test, coverage);

            if let Some(cmd_list) = test_cmds
                && check_precondition(*lang, lang_test.precondition.as_deref())
            {
                for cmd in cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_env(cmd, Some(&label), &env_vars) {
                        return (*lang, Err(e));
                    }
                }
            }
            if e2e
                && let Some(e2e_cmd_list) = &lang_test.e2e
                && check_e2e_precondition(*lang, &lang_test)
            {
                for cmd in e2e_cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_envs(cmd, Some(&label), &env_vars, &e2e_env) {
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
            error!("test failed: {lang} — {e}");
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

fn ensure_requested_suites_will_run(requested: &[Language], selected: &[Language]) -> anyhow::Result<()> {
    if !requested.is_empty() && selected.is_empty() {
        anyhow::bail!("every requested test suite was skipped by its precondition");
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
        if target_release.exists()
            && let Ok(entries) = std::fs::read_dir(&target_release)
        {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else { continue };
                if name_str.starts_with(lib_prefix)
                    && name_str.ends_with(lib_ext)
                    && let Some(path_str) = target_release.to_str()
                {
                    info!("Native library directory: {}", path_str);
                    return Some(path_str.to_string());
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    None
}

#[cfg(test)]
mod tests {
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

    /// Build a ResolvedCrateConfig with `[crates.test.python]` set to `extra_toml` plus an `e2e`
    /// command built from `e2e_cmd`. No `command` is set, so the block exercises the `e2e` phase
    /// in isolation -- matching the common consumer shape (`before` + `e2e`, no unit-test phase).
    #[cfg(unix)]
    fn make_config_with_e2e(extra_toml: &str, e2e_cmd: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
{extra_toml}
e2e = "{e2e_cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// A shell command that appends `text` to `path`, escaped for single-quoting.
    #[cfg(unix)]
    fn shell_append_cmd(path: &std::path::Path, text: &str) -> String {
        let escaped = path.display().to_string().replace('\'', "'\\''");
        format!("printf '{text}\\n' >> '{escaped}'")
    }

    /// A fresh, empty marker file path scoped by `name` and the test process id, so concurrent
    /// tests in this file never collide.
    #[cfg(unix)]
    fn marker_path(name: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("alef_e2e_precondition_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        tmp.join("marker.txt")
    }

    /// `e2e_precondition` present and passing: the e2e command runs.
    #[cfg(unix)]
    #[test]
    fn e2e_precondition_present_and_passing_allows_e2e_to_run() {
        let marker = marker_path("passing");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("e2e_precondition = \"true\"", &e2e_cmd);

        test(&config, &[Language::Python], true, false).expect("e2e should run when e2e_precondition passes");

        assert!(marker.exists(), "e2e command should have executed");
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// `e2e_precondition` present and failing: the e2e command is skipped, and since it is the
    /// only requested suite, the exit-1 backstop fires.
    #[cfg(unix)]
    #[test]
    fn e2e_precondition_present_and_failing_skips_e2e_and_trips_backstop() {
        let marker = marker_path("failing");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("e2e_precondition = \"false\"", &e2e_cmd);

        let error = test(&config, &[Language::Python], true, false)
            .expect_err("a failing e2e_precondition for the only requested language must trip the backstop");
        assert!(
            error.to_string().contains("every requested test suite was skipped"),
            "got: {error}"
        );
        assert!(!marker.exists(), "e2e command must not run when e2e_precondition fails");
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// `e2e_precondition` absent: the e2e phase runs ungated rather than inheriting a failing main
    /// `precondition` written for a different command. This is the fix for the reported defect --
    /// before it, a block with no `command` (only `before` + `e2e`) had no way to give `e2e` its
    /// own tooling gate, so consumers set `precondition` to whatever `command` needed and `e2e`
    /// inherited it.
    #[cfg(unix)]
    #[test]
    fn absent_e2e_precondition_does_not_inherit_a_failing_main_precondition() {
        let marker = marker_path("absent");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("precondition = \"false\"", &e2e_cmd);

        test(&config, &[Language::Python], true, false)
            .expect("e2e must run ungated when e2e_precondition is absent, even if the main precondition fails");

        assert!(
            marker.exists(),
            "e2e command should have executed despite the failing main precondition"
        );
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// The main `precondition` still gates the `command` phase exactly as before this change.
    #[cfg(unix)]
    #[test]
    fn main_precondition_still_gates_the_command_phase() {
        let marker = marker_path("command_gate");
        let cmd = shell_append_cmd(&marker, "ran");
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
precondition = "false"
command = "{cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        let config = cfg.resolve().unwrap().remove(0);

        let error = test(&config, &[Language::Python], false, false)
            .expect_err("a failing main precondition must still skip the command phase and trip the backstop");
        assert!(
            error.to_string().contains("every requested test suite was skipped"),
            "got: {error}"
        );
        assert!(
            !marker.exists(),
            "command must not run when the main precondition fails"
        );
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// `[crates.e2e.env]` entries must reach the local `e2e` run command's spawned process --
    /// not just generated in-tree fixture setup code, which for some runtimes (e.g. Elixir's
    /// `mix test`, whose NIF loads before `test_helper.exs` runs) executes too late to matter.
    /// Mirrors `test_apps::test_apps_run_tests::e2e_env_vars_are_exported_to_run_command`,
    /// which already covers the registry-mode runner; this covers the local one.
    #[cfg(unix)]
    #[test]
    fn e2e_config_env_vars_are_exported_to_e2e_run_command() {
        let toml = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.env]
ALEF_TEST_ALLOW_PRIVATE_NETWORK = "true"
[crates.e2e.call]
function = "process"
module = "test-lib"
result_var = "result"

[crates.test.python]
e2e = "test \"$ALEF_TEST_ALLOW_PRIVATE_NETWORK\" = true"
"#;
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        let config = cfg.resolve().unwrap().remove(0);

        let result = test(&config, &[Language::Python], true, false);
        assert!(
            result.is_ok(),
            "a declared [crates.e2e.env] var must reach the local e2e run command: {result:?}"
        );
    }

    #[test]
    fn rejects_all_requested_suites_being_skipped() {
        let error = ensure_requested_suites_will_run(&[Language::Python], &[]).expect_err("zero suites must fail");
        assert!(error.to_string().contains("every requested test suite was skipped"));
    }

    #[test]
    fn permits_an_explicitly_empty_test_selection() {
        ensure_requested_suites_will_run(&[], &[]).expect("no requested suites is valid");
    }
}
