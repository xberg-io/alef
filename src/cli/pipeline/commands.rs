use crate::core::config::output::StringOrVec;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::template_versions as tv;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::cli::registry;

use super::helpers::{
    check_precondition, check_precondition_named, run_before, run_command, run_command_captured, run_command_streamed,
    run_command_streamed_with_env,
};

/// Which lint phases to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LintPhase {
    Format,
    Check,
    Typecheck,
}

/// Filter languages through precondition and before hooks for lint/fmt.
///
/// Returns the subset of languages whose precondition passed (or was absent).
/// Runs before hooks for each passing language. Fails if any before hook fails.
fn prepare_lint_languages(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<Vec<Language>> {
    let mut ready = Vec::with_capacity(languages.len());
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            continue;
        }
        run_before(lang, lang_lint.before.as_ref())?;
        ready.push(lang);
    }
    Ok(ready)
}

/// Run a single lint phase across all languages in parallel.
fn run_phase(config: &ResolvedCrateConfig, languages: &[Language], phase: LintPhase) -> anyhow::Result<()> {
    let tasks: Vec<(&Language, String)> = languages
        .iter()
        .filter_map(|lang| {
            let lang_lint = config.lint_config_for_language(*lang);
            let cmds = match phase {
                LintPhase::Format => lang_lint.format,
                LintPhase::Check => lang_lint.check,
                LintPhase::Typecheck => lang_lint.typecheck,
            };
            cmds.map(|cmd_list| {
                cmd_list
                    .commands()
                    .into_iter()
                    .map(|c| (lang, c.to_string()))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let results: Vec<anyhow::Result<()>> = tasks
        .par_iter()
        .map(|(lang, cmd)| run_command_streamed(cmd, Some(&lang.to_string())))
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for result in results {
        if let Err(e) = result {
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

/// Run all configured lint/format commands on generated output.
///
/// Executes in two waves for correctness:
/// 1. Format (all languages in parallel) — normalizes code first
/// 2. Check + Typecheck (all languages in parallel) — validates normalized code
pub fn lint(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    // Wave 1: format all languages in parallel
    run_phase(config, &ready, LintPhase::Format)?;
    // Wave 2: check + typecheck all languages in parallel
    run_phase(config, &ready, LintPhase::Check)?;
    run_phase(config, &ready, LintPhase::Typecheck)?;
    Ok(())
}

/// Run only format commands on generated output.
pub fn fmt(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    run_phase(config, &ready, LintPhase::Format)
}

/// Run format commands as part of post-generation, never propagating failure.
///
/// Formatting after `alef generate` is best-effort: formatters are *expected*
/// to modify generated files (that's their purpose), and a missing formatter,
/// a config issue, or a non-zero exit must not abort the generate run. Each
/// per-language failure is logged as a warning and processing continues with
/// the next language.
pub fn fmt_post_generate(config: &ResolvedCrateConfig, languages: &[Language]) {
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            // `check_precondition` already warns and returns false on miss.
            continue;
        }
        if let Err(e) = run_before(lang, lang_lint.before.as_ref()) {
            warn!("[{lang}] post-generation `before` hook failed: {e:#}");
            continue;
        }
        let Some(cmd_list) = lang_lint.format else {
            continue;
        };
        for cmd in cmd_list.commands() {
            if let Err(e) = run_command_streamed(cmd, Some(&lang.to_string())) {
                warn!("[{lang}] post-generation format command failed (continuing): {e:#}");
            }
        }
    }
}

/// Deduplicate command strings across language plans, preserving within-language order.
///
/// The first language claiming a command owns it; subsequent languages with the same
/// command string get it removed from their list. This prevents races when multiple
/// languages (e.g. Node and Wasm) share workspace-wide commands like `pnpm up --latest -r -w`.
fn dedupe_plans(plans: Vec<(Language, Vec<String>)>) -> Vec<(Language, Vec<String>)> {
    let mut seen = std::collections::HashSet::<String>::new();
    plans
        .into_iter()
        .map(|(lang, cmds)| {
            let unique: Vec<String> = cmds.into_iter().filter(|c| seen.insert(c.clone())).collect();
            (lang, unique)
        })
        .collect()
}

/// Update dependencies for each language.
///
/// When `latest` is true, runs the aggressive `upgrade` commands (including
/// incompatible/major version bumps). Otherwise runs the safe `update` commands.
///
/// Executes in two phases:
/// 1. Sequential: check preconditions and collect command lists; deduplicate across languages.
/// 2. Parallel: run each language's deduped command list (within-language order preserved).
pub fn update(config: &ResolvedCrateConfig, languages: &[Language], latest: bool) -> anyhow::Result<()> {
    // Phase 1 (sequential): check preconditions, run before hooks, collect command lists.
    let mut plans: Vec<(Language, Vec<String>)> = Vec::new();
    for &lang in languages {
        let update_cfg = config.update_config_for_language(lang);
        if !check_precondition(lang, update_cfg.precondition.as_deref()) {
            continue;
        }
        run_before(lang, update_cfg.before.as_ref())?;
        let cmds = if latest {
            update_cfg.upgrade.as_ref()
        } else {
            update_cfg.update.as_ref()
        };
        let cmd_strings: Vec<String> = cmds
            .map(|cmd_list| cmd_list.commands().into_iter().map(|c| c.to_string()).collect())
            .unwrap_or_default();
        plans.push((lang, cmd_strings));
    }

    // Deduplicate shared commands (e.g. pnpm workspace commands shared by Node and Wasm).
    let plans = dedupe_plans(plans);

    // Phase 2 (parallel): run each language's deduped command list with live output.
    let results: Vec<(Language, anyhow::Result<()>)> = plans
        .par_iter()
        .map(|(lang, cmds)| {
            let label = lang.to_string();
            for cmd in cmds {
                if let Err(e) = run_command_streamed(cmd, Some(&label)) {
                    return (*lang, Err(e));
                }
            }
            (*lang, Ok(()))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        if let Err(e) = result {
            eprintln!("✗ update failed: {lang} — {e}");
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

/// Run configured test commands for each language.
///
/// When `coverage` is true, runs coverage commands instead of regular test commands.
/// When `e2e` is true, also runs e2e test commands.
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
            // Resolve the per-language working directory. Languages whose
            // manifest does not live at the repo root (swift / kotlin-android /
            // dart / zig) declare a `workdir` so install commands like
            // `swift package resolve` and `gradle build` find their manifest.
            // Skip the cwd if the directory does not exist yet — the binding
            // may not have been initialized, in which case the install command
            // would have failed anyway from the repo root.
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
                        super::helpers::run_command_streamed_with_cwd_and_timeout(
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
        let (stdout, stderr) = super::helpers::run_command_captured_with_timeout(cmd, Some(timeout_secs))
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

/// Clean build artifacts for each language.
pub fn clean(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<()> {
    let results: Vec<(Language, anyhow::Result<()>)> = languages
        .par_iter()
        .map(|lang| {
            let label = lang.to_string();
            let clean_cfg = config.clean_config_for_language(*lang);
            if !check_precondition(*lang, clean_cfg.precondition.as_deref()) {
                return (*lang, Ok(()));
            }
            if let Err(e) = run_before(*lang, clean_cfg.before.as_ref()) {
                return (*lang, Err(e));
            }
            if let Some(cmd_list) = &clean_cfg.clean {
                for cmd in cmd_list.commands() {
                    if let Err(e) = run_command_streamed(cmd, Some(&label)) {
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
            eprintln!("✗ clean failed: {lang} — {e}");
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

/// Outcome of running a single language's registry-mode test app.
///
/// Distinguishes a precondition skip from a genuine pass so the summary can
/// report them separately (a skip is not a failure, but it is also not a pass).
#[derive(Debug)]
enum TestAppOutcome {
    /// The run commands executed successfully.
    Passed,
    /// The precondition command failed, so the run was skipped.
    Skipped,
    /// A before hook or a run command failed.
    Failed(anyhow::Error),
}

/// A running e2e mock-server process plus the env vars its startup line exported.
///
/// The child is kept alive for the lifetime of this guard; dropping it closes the
/// child's stdin (the mock-server blocks reading stdin and exits on EOF) and then
/// kills + reaps the process so no orphan listener survives the run.
struct MockServerHandle {
    child: std::process::Child,
    /// Env vars to inject into every test-app `run` command: always
    /// `MOCK_SERVER_URL`, plus `MOCK_SERVERS` when the server printed it.
    env_vars: Vec<(&'static str, String)>,
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        // Closing stdin triggers the server's graceful shutdown (it blocks on a
        // stdin read loop). Then kill + wait to guarantee no orphan listener.
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Check if the given Cargo.toml defines a [[bin]] target named "mock-server".
fn has_mock_server_bin(manifest_path: &std::path::Path) -> anyhow::Result<bool> {
    let content = std::fs::read_to_string(manifest_path)
        .context("failed to read Cargo.toml to check for mock-server bin target")?;
    // Simple heuristic: look for the line `name = "mock-server"` within a [[bin]] section.
    // TOML parsing is more complex, but for generated files we know the format is
    // `[[bin]]\nname = "mock-server"\npath = "src/main.rs"` when the bin is present.
    Ok(content.contains("[[bin]]") && content.contains("name = \"mock-server\""))
}

/// Build and start the shared e2e mock-server, returning a handle whose env vars
/// (`MOCK_SERVER_URL`, optional `MOCK_SERVERS`) must be injected into every
/// test-app `run` command.
///
/// The mock-server crate is the alef-generated `<e2e.output>/rust` project, built
/// in release (mirroring sample_project's Taskfile `e2e:build`), producing the
/// `mock-server` binary at `<e2e.output>/rust/target/release/mock-server`. On
/// startup the binary prints `MOCK_SERVER_URL=http://127.0.0.1:<port>` (and, when
/// host-root fixtures exist, `MOCK_SERVERS={...}`) to stdout, then blocks reading
/// stdin until the parent closes the pipe.
///
/// Returns `Ok(None)` when the e2e config has no fixtures directory / rust crate
/// to build (no HTTP fixtures → no mock-server needed); the test apps then run
/// without the env vars exactly as before. Any build/spawn/parse failure is a hard
/// error so a missing server never silently degrades to "connection refused".
fn start_mock_server(config: &ResolvedCrateConfig) -> anyhow::Result<Option<MockServerHandle>> {
    let Some(e2e) = config.e2e.as_ref() else {
        return Ok(None);
    };
    let base_dir = std::env::current_dir().context("failed to resolve current directory")?;
    let rust_crate_dir = base_dir.join(&e2e.output).join("rust");
    let manifest_path = rust_crate_dir.join("Cargo.toml");
    // No generated rust mock-server crate → nothing to start. This happens for
    // crates whose fixtures never need an HTTP mock server.
    if !manifest_path.exists() {
        info!(
            "No e2e mock-server crate at {} — running test apps without MOCK_SERVER_URL",
            manifest_path.display()
        );
        return Ok(None);
    }

    // Check if the Cargo.toml actually defines a mock-server bin target.
    // It won't exist if no fixtures required a mock server (needs_mock_server was false).
    if !has_mock_server_bin(&manifest_path)? {
        info!(
            "No [[bin]] mock-server target in {} — running test apps without MOCK_SERVER_URL",
            manifest_path.display()
        );
        return Ok(None);
    }

    // Build the mock-server binary in release (matches Taskfile `e2e:build`).
    info!("Building e2e mock-server: {}", manifest_path.display());
    run_command_streamed(
        &format!(
            "cargo build --release --manifest-path {} --bin mock-server",
            manifest_path.display()
        ),
        Some("mock-server"),
    )
    .context("failed to build the e2e mock-server")?;

    let bin_path = rust_crate_dir.join("target").join("release").join("mock-server");
    if !bin_path.exists() {
        anyhow::bail!("e2e mock-server binary not found after build: {}", bin_path.display());
    }

    // The mock-server resolves fixtures relative to its first argument; pass an
    // absolute path so it does not depend on the child's working directory.
    let fixtures_dir = base_dir.join(&e2e.fixtures);

    info!(
        "Starting e2e mock-server ({}) with fixtures {}",
        bin_path.display(),
        fixtures_dir.display()
    );
    let mut child = std::process::Command::new(&bin_path)
        .arg(&fixtures_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to spawn e2e mock-server: {}", bin_path.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("e2e mock-server stdout pipe was not captured")?;

    // Read the startup lines. The server prints `MOCK_SERVER_URL=...` first, then
    // always prints `MOCK_SERVERS={...}` (possibly empty `{}`), then blocks on
    // stdin. We stop once we have the URL and have either seen MOCK_SERVERS or hit
    // a non-`MOCK_SERVER` line. Bound the loop so a misbehaving server can't hang.
    let mut reader = std::io::BufReader::new(stdout);
    let mut url: Option<String> = None;
    let mut servers: Option<String> = None;
    {
        use std::io::BufRead as _;
        let mut line = String::new();
        for _ in 0..8 {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("MOCK_SERVER_URL=") {
                        url = Some(rest.to_string());
                    } else if let Some(rest) = trimmed.strip_prefix("MOCK_SERVERS=") {
                        servers = Some(rest.to_string());
                        break;
                    } else if url.is_some() {
                        break;
                    }
                }
            }
        }
    }

    let url = url
        .context("e2e mock-server did not print a MOCK_SERVER_URL= line on startup; cannot run test apps without it")?;
    info!("e2e mock-server ready at {url}");

    // Drain the rest of stdout in the background so the server's writes never
    // block on a full pipe over the lifetime of the run.
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        let mut sink = String::new();
        while reader.read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
            sink.clear();
        }
    });

    let mut env_vars: Vec<(&'static str, String)> = vec![("MOCK_SERVER_URL", url)];
    if let Some(servers) = servers {
        env_vars.push(("MOCK_SERVERS", servers));
    }

    Ok(Some(MockServerHandle { child, env_vars }))
}

/// Run the registry-mode test app for each language.
///
/// Each test app exercises the *published* package against the same fixtures the
/// e2e suite uses. `names` are `[e2e].languages` entries — usually language slugs,
/// but also string-only registry targets like `brew`. For each: check the
/// precondition (skip with a warning on failure), run the `before` hook (abort
/// that target on failure), then execute each configured `run` command with live
/// output. The `run` commands `cd` into their own `test_apps/<name>/` directory,
/// so no cwd is supplied here.
///
/// Before running any apps, a single shared e2e mock-server is built and started;
/// its `MOCK_SERVER_URL` (and `MOCK_SERVERS` when present) is injected into every
/// `run`/`before` command's environment. The harnesses use this value instead of
/// spawning their own server (the local `e2e/<lang>/` binary path does not exist
/// under `test_apps/<lang>/`). The server is stopped when this function returns,
/// on success or failure, via the `MockServerHandle` guard.
///
/// Targets run in parallel (mirroring `setup`/`clean`). A precondition skip — and
/// a target with no `run` command (e.g. `ffi`) — is reported distinctly from a
/// pass; the first failing target's error is returned so the process exits non-zero.
pub fn test_apps_run(config: &ResolvedCrateConfig, names: &[String]) -> anyhow::Result<()> {
    // Build + start one shared mock-server for all targets. The guard stops it on
    // drop (end of this function), whether we return Ok or Err. A build/start
    // failure aborts the whole run with a clear error rather than letting every
    // harness fail with "connection refused".
    let server = start_mock_server(config).context("failed to start e2e mock-server for test apps")?;
    let server_env: Vec<(&str, String)> = server
        .as_ref()
        .map(|h| h.env_vars.iter().map(|(k, v)| (*k, v.clone())).collect())
        .unwrap_or_default();
    // Plain, overwrite-style export prefix for the server env vars. Do NOT use
    // `run_command_streamed_with_env` here: it appends PATH-style
    // (`export VAR='val'"${VAR:+:$VAR}"`), which corrupts a plain value like
    // MOCK_SERVER_URL into `http://host:port:http://host:port` (invalid → getaddrinfo
    // failure). These are scalar values, so set them outright. Values are URLs / JSON
    // (double-quoted) with no single quotes, so single-quote wrapping is safe.
    let env_prefix: String = server_env.iter().map(|(k, v)| format!("export {k}='{v}'; ")).collect();

    let results: Vec<(String, TestAppOutcome)> = names
        .par_iter()
        .map(|name| {
            let cfg = config.test_apps_run_config_for_name(name);
            if !check_precondition_named(name, cfg.precondition.as_deref()) {
                // `check_precondition_named` already warns on miss.
                return (name.clone(), TestAppOutcome::Skipped);
            }
            if let Some(before) = &cfg.before {
                for cmd in before.commands() {
                    if let Err(e) = run_command_streamed(&format!("{env_prefix}{cmd}"), Some(name)) {
                        return (name.clone(), TestAppOutcome::Failed(e));
                    }
                }
            }
            match &cfg.run {
                Some(cmd_list) => {
                    for cmd in cmd_list.commands() {
                        if let Err(e) = run_command_streamed(&format!("{env_prefix}{cmd}"), Some(name)) {
                            return (name.clone(), TestAppOutcome::Failed(e));
                        }
                    }
                    (name.clone(), TestAppOutcome::Passed)
                }
                // No run command configured (e.g. ffi/jni) — nothing to verify.
                None => (name.clone(), TestAppOutcome::Skipped),
            }
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (name, outcome) in results {
        match outcome {
            TestAppOutcome::Passed => eprintln!("✓ test-app passed: {name}"),
            TestAppOutcome::Skipped => eprintln!("⊘ test-app skipped: {name}"),
            TestAppOutcome::Failed(e) => {
                eprintln!("✗ test-app failed: {name} — {e}");
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Build language bindings using configured build commands.
///
/// Uses configurable per-language build commands from `[build_commands.<lang>]`
/// in alef.toml, falling back to sensible defaults. Resolves build order
/// (FFI-dependent languages build after FFI).
pub fn build(config: &ResolvedCrateConfig, languages: &[Language], release: bool) -> anyhow::Result<()> {
    let crate_name = &config.name;
    let base_dir = std::env::current_dir()?;

    // Split into FFI-independent and FFI-dependent languages
    let mut independent = Vec::new();
    let mut ffi_dependent = Vec::new();
    let mut need_ffi = false;

    // Rust is handled via configurable build commands, not the registry
    let mut rust_langs: Vec<Language> = Vec::new();

    for &lang in languages {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        if !check_precondition(lang, build_cmd_cfg.precondition.as_deref()) {
            continue;
        }
        if lang == Language::Rust {
            rust_langs.push(lang);
            continue;
        }
        let backend = registry::get_backend(lang);
        if let Some(bc) = backend.build_config_with_config(config) {
            if bc.depends_on_ffi() {
                ffi_dependent.push((lang, bc));
                need_ffi = true;
            } else {
                independent.push((lang, bc));
            }
        } else {
            info!("No build config for {lang}, skipping");
        }
    }

    // Build Rust first (other bindings may depend on it)
    for &lang in &rust_langs {
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
                run_command(cmd).with_context(|| format!("failed to build {lang}"))?;
            }
        }
    }

    // Build FFI first if needed by dependent languages
    if need_ffi
        && !independent
            .iter()
            .any(|(_, bc)| bc.tool == "cargo" && bc.crate_suffix == "-ffi")
    {
        // Resolve FFI crate name from output path
        let ffi_crate = output_path_for(Language::Ffi, config)
            .map(resolve_crate_dir)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| {
                // Fallback: construct from crate name
                Box::leak(format!("{crate_name}-ffi").into_boxed_str())
            });
        info!("Building FFI crate: {ffi_crate}");
        let mut cmd = format!("cargo build -p {ffi_crate}");
        if release {
            cmd.push_str(" --release");
        }
        run_command(&cmd).context("failed to build FFI crate")?;
    }

    // Run before hooks for independent languages (sequentially — they may have side effects)
    for (lang, _) in &independent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    // Build independent languages in parallel
    let build_results: Vec<anyhow::Result<(String, String)>> = independent
        .par_iter()
        .map(|(lang, bc)| {
            // Check for explicit build_commands override before using backend default
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                // Use the user-provided build_commands override if the override differs from defaults
                if config.build_commands.contains_key(&lang.to_string()) {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
            }
            info!("Building {lang} ({})...", bc.tool);
            let build_cmd = build_command_for(*lang, bc, config, release);
            run_command_captured(&build_cmd).with_context(|| format!("failed to build language bindings for {lang}"))
        })
        .collect();

    for ((lang, bc), result) in independent.iter().zip(build_results) {
        let (stdout, stderr) = result?;
        if !stdout.is_empty() {
            info!("[{lang} build] {stdout}");
        }
        if !stderr.is_empty() {
            debug!("[{lang} build] {stderr}");
        }
        run_post_build(*lang, bc, config, &base_dir)
            .with_context(|| format!("failed to run post-build steps for {lang}"))?;
    }

    // Run before hooks for FFI-dependent languages
    for (lang, _) in &ffi_dependent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    // Build FFI-dependent languages in parallel
    let build_results: Vec<anyhow::Result<(String, String)>> = ffi_dependent
        .par_iter()
        .map(|(lang, bc)| {
            // Check for explicit build_commands override before using backend default
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                if config.build_commands.contains_key(&lang.to_string()) {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
            }
            info!("Building {lang} ({})...", bc.tool);
            let build_cmd = build_command_for(*lang, bc, config, release);
            run_command_captured(&build_cmd).with_context(|| format!("failed to build language bindings for {lang}"))
        })
        .collect();

    for ((lang, bc), result) in ffi_dependent.iter().zip(build_results) {
        let (stdout, stderr) = result?;
        if !stdout.is_empty() {
            info!("[{lang} build] {stdout}");
        }
        if !stderr.is_empty() {
            debug!("[{lang} build] {stderr}");
        }
        run_post_build(*lang, bc, config, &base_dir)
            .with_context(|| format!("failed to run post-build steps for {lang}"))?;
    }

    Ok(())
}

/// Resolve the crate directory from the output config path.
/// Output paths like `crates/sample-markdown-node/src/` → `crates/sample-markdown-node`.
fn resolve_crate_dir(output_path: &Path) -> &Path {
    // If path ends in src/ or src, go up one level
    if output_path.file_name().is_some_and(|n| n == "src") {
        output_path.parent().unwrap_or(output_path)
    } else {
        output_path
    }
}

/// Get the output path for a language from config.
fn output_path_for(lang: Language, config: &ResolvedCrateConfig) -> Option<&Path> {
    match lang {
        Language::Python => config.explicit_output.python.as_deref(),
        Language::Node => config.explicit_output.node.as_deref(),
        Language::Ruby => config.explicit_output.ruby.as_deref(),
        Language::Php => config.explicit_output.php.as_deref(),
        Language::Ffi => config.explicit_output.ffi.as_deref(),
        Language::Go => config.explicit_output.go.as_deref(),
        Language::Java => config.explicit_output.java.as_deref(),
        Language::Csharp => config.explicit_output.csharp.as_deref(),
        Language::Wasm => config.explicit_output.wasm.as_deref(),
        Language::Elixir => config.explicit_output.elixir.as_deref(),
        Language::R => config.explicit_output.r.as_deref(),
        // Rust is the core language — no separate output path.
        // C is an e2e test consumer of the FFI layer — no generated binding output path.
        // Jni output is emitted into the consumer's Rust workspace, not a separate binding crate.
        Language::Rust | Language::C | Language::Jni => None,
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => None,
    }
}

/// Generate the shell command to build a specific language.
fn build_command_for(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    release: bool,
) -> String {
    let release_flag = if release { " --release" } else { "" };

    // Resolve the crate directory from the output path
    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .and_then(|p| p.to_str())
        .unwrap_or("");

    match bc.tool {
        "maturin" => {
            format!("maturin develop --manifest-path {crate_dir}/Cargo.toml{release_flag}")
        }
        "napi" => {
            // NAPI outputs .node + .d.ts to the crate directory
            // Use npx to provision @napi-rs/cli on demand
            format!(
                "npx --yes -p @napi-rs/cli@{} napi build --platform --manifest-path {}/Cargo.toml -o {}{}",
                tv::npm::NAPI_RS_CLI_CRATE,
                crate_dir,
                crate_dir,
                release_flag
            )
        }
        "wasm-pack" => {
            let profile = if release { "--release" } else { "--dev" };
            // `web` target exposes a default `init(wasm_bytes_or_url)` function which
            // both the e2e test runner and a typical web app use; `bundler` produces a
            // package that auto-initializes on import and has no `init` default export.
            // The e2e test codegen calls `init(wasmBytes)` explicitly, so `web` is the
            // matching target.
            format!("wasm-pack build {crate_dir} {profile} --target web")
        }
        "cargo" => {
            // When the language has no explicit `output` path (e.g. Dart in FRB style,
            // whose generated Dart sources live at packages/dart/lib/src/ but whose
            // Rust crate lives at packages/<lang>/rust/), `output_path_for` returns
            // None and `crate_dir` is empty. In that case rely on the registered
            // `crate_suffix` to invoke the workspace member directly.
            if crate_dir.is_empty() && !bc.crate_suffix.is_empty() {
                return format!("cargo build -p {}{}{}", config.name, bc.crate_suffix, release_flag);
            }
            // Check for a standalone crate directory (e.g., Ruby's native/ subdir,
            // or R's extendr crate at packages/r/src/rust/) that is excluded from
            // the workspace and must be built via cd + cargo build.
            let native_dir = Path::new(crate_dir).join("native");
            let native_manifest = native_dir.join("Cargo.toml");
            if native_manifest.exists() {
                let dir = native_dir.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else if let Some(standalone) = {
                // Look at most 2 levels up for the crate's own Cargo.toml;
                // if it declares its own `[workspace]`, treat as standalone
                // (cd in and `cargo build`). Don't walk past that to the
                // repo-root workspace.
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut found: Option<std::path::PathBuf> = None;
                for _ in 0..3 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest) {
                            if contents.contains("[workspace]") {
                                found = Some(p.clone());
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                found
            } {
                let dir = standalone.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else {
                // Walk up to find the nearest [package] Cargo.toml and remember its dir.
                // Then walk further up to find a parent [workspace] Cargo.toml to determine
                // whether the package is a workspace member (use `-p name`) or excluded
                // (fall back to `cd <dir> && cargo build`).
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut package_name: Option<String> = None;
                let mut package_dir: Option<std::path::PathBuf> = None;
                for _ in 0..4 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest) {
                            if contents.contains("[package]") {
                                for line in contents.lines() {
                                    let trimmed = line.trim();
                                    if let Some(rest) = trimmed.strip_prefix("name") {
                                        let rest = rest.trim_start_matches([' ', '=']).trim();
                                        let rest = rest.trim_matches(['"', '\'']);
                                        if !rest.is_empty() {
                                            package_name = Some(rest.to_string());
                                            package_dir = Some(p.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                // Search upward from the package dir for a workspace Cargo.toml.
                // If found and the package is in `exclude = [...]`, treat as standalone.
                let is_excluded_from_workspace = if let Some(pdir) = &package_dir {
                    let mut q = pdir.clone();
                    let mut excluded = false;
                    while q.pop() {
                        let manifest = q.join("Cargo.toml");
                        if manifest.exists() {
                            if let Ok(contents) = std::fs::read_to_string(&manifest) {
                                if contents.contains("[workspace]") {
                                    let rel = pdir.strip_prefix(&q).unwrap_or(pdir).to_string_lossy().into_owned();
                                    let rel_norm = rel.replace('\\', "/");
                                    excluded = contents.lines().map(|l| l.trim()).any(|l| {
                                        l.contains(&format!("\"{rel_norm}\"")) && {
                                            // Only count occurrences inside an `exclude = [...]` context;
                                            // approximate by also looking for "exclude" in nearby lines.
                                            // A simple heuristic: the path appears after the literal
                                            // `exclude = [`. Use a substring match on the whole file.
                                            let needle = format!("\"{rel_norm}\"");
                                            let exclude_section = contents.split("exclude").nth(1).unwrap_or("");
                                            let members_section = contents.split("members").nth(1).unwrap_or("");
                                            let in_exclude = exclude_section.contains(&needle);
                                            let in_members =
                                                members_section.contains(&needle) && !exclude_section.contains(&needle);
                                            in_exclude && !in_members
                                        }
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    excluded
                } else {
                    false
                };
                if is_excluded_from_workspace {
                    if let Some(pdir) = package_dir {
                        let dir = pdir.display();
                        format!("cd {dir} && cargo build{release_flag}")
                    } else {
                        format!("cd {crate_dir} && cargo build{release_flag}")
                    }
                } else {
                    let crate_name = package_name.unwrap_or_else(|| {
                        Path::new(crate_dir)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(crate_dir)
                            .to_string()
                    });
                    format!("cargo build -p {crate_name}{release_flag}")
                }
            }
        }
        "mix" => {
            // The elixir [crates.output] points at native/<nif>/src/, but mix runs from the
            // mix project root containing mix.exs. Walk up from the source dir to find it.
            let dir = config
                .explicit_output
                .elixir
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/elixir");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("mix.exs").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mix compile")
        }
        "mvn" => {
            let dir = config
                .explicit_output
                .java
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/java");
            // Walk up from the source dir until we find a pom.xml. The java
            // [crates.output] points at src/main/java, but maven runs from the
            // project root.
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("pom.xml").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mvn package -DskipTests --batch-mode --no-transfer-progress")
        }
        "dotnet" => {
            let dir = config
                .explicit_output
                .csharp
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/csharp");
            // Find the directory containing the .csproj. The csharp [crates.output] often
            // points at a source path (e.g. `packages/csharp/src/`), so we walk both:
            //   1. directly inside `dir` and one level of children, and
            //   2. upward from `dir`, scanning each parent and one level of children.
            // First match wins.
            let scan_for_csproj = |start: &std::path::Path| -> Option<String> {
                if start
                    .read_dir()
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
                    })
                    .unwrap_or(false)
                {
                    return Some(start.to_string_lossy().to_string());
                }
                start.read_dir().ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .find(|e| {
                            e.path().is_dir()
                                && e.path().read_dir().ok().is_some_and(|sub| {
                                    sub.filter_map(|s| s.ok())
                                        .any(|s| s.path().extension().is_some_and(|ext| ext == "csproj"))
                                })
                        })
                        .map(|e| e.path().to_string_lossy().to_string())
                })
            };
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                let mut found = scan_for_csproj(&p);
                while found.is_none() && p.pop() {
                    found = scan_for_csproj(&p);
                }
                found.unwrap_or_else(|| dir.to_string())
            };
            let dotnet_config = if release { "Release" } else { "Debug" };
            format!("cd {build_dir} && dotnet build --configuration {dotnet_config} --verbosity quiet")
        }
        "go" => {
            let dir = config
                .explicit_output
                .go
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/go");
            format!("cd {dir} && go build ./...")
        }
        other => format!("echo 'Unknown build tool: {other}'"),
    }
}

/// Run post-build processing steps (e.g., patching .d.ts files).
pub fn run_post_build(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> anyhow::Result<()> {
    use crate::core::backend::PostBuildStep;

    // Resolve the crate directory from the output path
    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .unwrap_or(Path::new(""));

    for step in &bc.post_build {
        match step {
            PostBuildStep::PatchFile { path, find, replace } => {
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-build patch target {}", file_path.display()))?;
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
                run_run_command(cmd, args, &work_dir)
                    .with_context(|| format!("post-build RunCommand '{cmd}' failed"))?;
            }
            PostBuildStep::PostProcessFile { path, processor } => {
                use crate::core::backend::PostProcessor;
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-process target {}", file_path.display()))?;
                    let processed = match processor {
                        PostProcessor::FrbDartSealedVariants => {
                            crate::backends::dart::rewrite_frb_sealed_variants(&content)
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
        }
    }

    Ok(())
}

/// Hard upper bound on how long a post-build `RunCommand` may run before alef
/// considers it hung and kills it. Cold-cache `cargo build --release` for the
/// swift binding crate against a polyglot project's full feature set
/// legitimately takes 10-20 minutes; FRB codegen on a warm cache finishes in
/// under a minute. 30 minutes accommodates both without false-positiving
/// slow first-runs on cold CI caches.
const RUN_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

/// Interval between `try_wait()` polls. Short enough to react promptly to a
/// finished child, long enough not to burn CPU in a tight loop.
const RUN_COMMAND_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Execute a `RunCommand` post-build step.
///
/// Spawns `cmd` with `args` in `base_dir`, streaming stdout/stderr through
/// alef's own stdio so interactive subprocess progress is visible. Enforces a
/// `RUN_COMMAND_TIMEOUT` ceiling; on timeout the child is SIGKILL'd and the
/// call returns an error. Returns an error on non-zero exit status.
///
/// Escape hatch: the env var `ALEF_SKIP_COMMANDS` accepts a comma-separated
/// list of `cmd` names to skip without running. Useful in environments where
/// a post-build tool is unavailable, hangs (e.g. `flutter_rust_bridge_codegen`
/// installing Flutter via FVM under CI), or simply isn't desired this run.
/// Each skipped command logs a `warn!` so the omission is visible.
fn run_run_command(cmd: &str, args: &[&str], base_dir: &Path) -> anyhow::Result<()> {
    if let Ok(skip_list) = std::env::var("ALEF_SKIP_COMMANDS") {
        if skip_list.split(',').any(|s| s.trim() == cmd) {
            warn!("[{cmd}] skipped via ALEF_SKIP_COMMANDS env var");
            return Ok(());
        }
    }
    let mut child = match std::process::Command::new(cmd)
        .args(args)
        .current_dir(base_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "[{cmd}] not on PATH — skipping post-build step. Install '{cmd}' to regenerate at build time; falling back to committed generated files."
            );
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err).context(format!("failed to spawn '{cmd}'"))),
    };

    let started_at = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started_at.elapsed() > RUN_COMMAND_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("'{cmd}' exceeded {}s timeout; killed", RUN_COMMAND_TIMEOUT.as_secs());
                }
                std::thread::sleep(RUN_COMMAND_POLL_INTERVAL);
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context(format!("failed to wait for '{cmd}'")));
            }
        }
    };

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        anyhow::bail!("'{cmd}' exited with status {code}");
    }

    Ok(())
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;
    use crate::core::backend::{BuildConfig, BuildDependency};

    #[test]
    fn dedupe_plans_removes_duplicate_commands_across_languages() {
        let plans = vec![
            (
                Language::Node,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
            (
                Language::Wasm,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["corepack use pnpm@latest", "pnpm up --latest -r -w"]);
        assert!(result[1].1.is_empty(), "Wasm should have no commands after dedupe");
    }

    #[test]
    fn dedupe_plans_preserves_within_language_order() {
        let plans = vec![
            (
                Language::Rust,
                vec!["cargo upgrade --incompatible".to_string(), "cargo update".to_string()],
            ),
            (
                Language::Node,
                vec!["cargo update".to_string()], // hypothetical duplicate
            ),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["cargo upgrade --incompatible", "cargo update"]);
        assert!(result[1].1.is_empty());
    }

    #[test]
    fn dedupe_plans_unique_commands_unchanged() {
        let plans = vec![
            (Language::Python, vec!["uv sync --upgrade".to_string()]),
            (Language::Ruby, vec!["bundle update --all".to_string()]),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["uv sync --upgrade"]);
        assert_eq!(result[1].1, vec!["bundle update --all"]);
    }

    #[test]
    fn csharp_build_command_uses_verbosity_flag_not_query_mode() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        let command = build_command_for(Language::Csharp, &build_config, &config, false);

        assert!(
            command.contains("--verbosity quiet"),
            "C# build must use explicit quiet verbosity: {command}"
        );
        assert!(
            !command.contains(" -q"),
            "C# build must not use dotnet query mode shorthand: {command}"
        );
    }
}

#[cfg(all(test, unix))]
mod test_apps_run_tests {
    // POSIX-only: relies on the shell builtins `true`/`false` to drive
    // precondition / run outcomes. On Windows `sh` is absent, so a precondition
    // miss is indistinguishable from a missing-program error and the skip-vs-fail
    // distinction this module asserts could not be exercised meaningfully.
    use super::*;

    fn resolved_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "false"
run = "false"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn failing_precondition_is_skipped_not_failed() {
        // The python run override has `precondition = "false"`, so the language
        // is skipped. The `run = "false"` command must NOT execute — if it did,
        // the run would fail. A skip is reported distinctly and never surfaces
        // as an error, so the overall result is `Ok`.
        let config = resolved_config();
        let result = test_apps_run(&config, &["python".to_string()]);
        assert!(
            result.is_ok(),
            "a precondition skip must be reported as skipped, not failed: {result:?}"
        );
    }

    #[test]
    fn failing_run_command_propagates_error() {
        // With the precondition passing (`true`) and the run command failing
        // (`false`), the language is genuinely failed and the first error
        // propagates so the process exits non-zero.
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "true"
run = "false"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let result = test_apps_run(&config, &["python".to_string()]);
        assert!(result.is_err(), "a failing run command must propagate as an error");
    }

    #[test]
    fn passing_run_command_succeeds() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "true"
run = "true"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let result = test_apps_run(&config, &["python".to_string()]);
        assert!(result.is_ok(), "a passing run command must succeed: {result:?}");
    }
}

#[cfg(all(test, unix))]
mod fmt_post_generate_tests {
    // Tests in this module rely on `sh -c` and the POSIX `true`/`false`
    // builtins. They are skipped on Windows where `sh` is not on PATH and
    // a missing-program error is indistinguishable from a precondition
    // miss (both cause `check_precondition` to return false), which would
    // make every test trivially pass for the wrong reason.
    use super::*;
    use crate::core::config::output::{LintConfig, StringOrVec};

    fn config_with_lint(lang: Language, cfg: LintConfig) -> ResolvedCrateConfig {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let mut c = alef_cfg.resolve().unwrap().remove(0);
        c.lint.insert(lang.to_string(), cfg);
        c
    }

    #[test]
    fn fmt_post_generate_swallows_failing_format_command() {
        // A format command that exits non-zero must not panic or propagate;
        // fmt_post_generate has no return value, so reaching the end is the
        // contract.
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_swallows_failing_before_hook() {
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: Some(StringOrVec::Single("false".to_string())),
            format: Some(StringOrVec::Single("true".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_skips_when_precondition_fails() {
        let lint = LintConfig {
            precondition: Some("false".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        // Precondition is `false` → format step never runs, so the (otherwise
        // failing) `false` command isn't invoked. No panic, no propagation.
        fmt_post_generate(&cfg, &[Language::Python]);
    }
}

#[cfg(all(test, unix))]
mod run_command_tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_skip_env(previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("ALEF_SKIP_COMMANDS", value),
                None => std::env::remove_var("ALEF_SKIP_COMMANDS"),
            }
        }
    }

    #[test]
    fn run_run_command_succeeds_for_echo() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("echo", &["alef-runcommand-ok"], &dir);
        restore_skip_env(previous);
        assert!(result.is_ok(), "echo should succeed: {result:?}");
    }

    #[test]
    fn run_run_command_fails_for_false() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("false", &[], &dir);
        restore_skip_env(previous);
        assert!(result.is_err(), "false should return Err");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("exited with status"),
            "error should mention exit status: {msg}"
        );
    }

    #[test]
    fn run_run_command_honors_skip_env_var() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        // Single test rather than two parallel tests, because cargo runs tests
        // concurrently by default and ALEF_SKIP_COMMANDS is a process-global
        // env var: separate tests would race each other on set/unset.
        let dir = std::env::temp_dir();
        // Phase 1: env var set, cmd in list → must skip (returns Ok despite
        // `false` exiting non-zero).
        // Safety: required by std's set_var contract on recent toolchains.
        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "noop,false , another");
        }
        let skipped = run_run_command("false", &[], &dir);
        assert!(
            skipped.is_ok(),
            "listed command must return Ok without spawning: {skipped:?}"
        );

        // Phase 2: env var set, cmd NOT in list → must spawn and surface
        // failure (so we know the env var isn't a blanket skip).
        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "something-else");
        }
        let honored = run_run_command("false", &[], &dir);
        restore_skip_env(previous);
        assert!(
            honored.is_err(),
            "unlisted command must still spawn and surface failure"
        );
    }
}
