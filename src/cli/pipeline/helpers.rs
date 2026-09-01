use std::path::Path;

use crate::core::config::Language;
use crate::core::config::output::StringOrVec;
use crate::process::capture::{
    OUTPUT_DRAIN_GRACE, StreamDrain, collect_output_within, output_reader, output_reader_tee, spawn_drain,
    wait_for_drains,
};
use crate::process::timed::{Deadline, GroupChild};
use anyhow::Context as _;
use tracing::info;

#[cfg(all(test, unix))]
mod timeout_tests;

mod process_exec;
pub(crate) use process_exec::run_argv_step_streamed;

/// Run a shell command with environment variables scoped to the child process.
pub(crate) fn run_command_with_env(cmd: &str, environment: &[(&str, &str)]) -> anyhow::Result<()> {
    info!("Running: {cmd}");
    let status = std::process::Command::new("sh")
        .args(["-c", cmd])
        .envs(environment.iter().copied())
        .status()?;
    if !status.success() {
        anyhow::bail!("Command failed: {cmd}");
    }
    Ok(())
}

/// Run a command with given arguments, logging but not failing if the binary is
/// absent or the command fails. Used for optional ecosystem-specific lockfile
/// refresh commands (pnpm, cargo, composer, mix) that may not be installed.
///
/// Logs and returns gracefully if the binary cannot be found or the command exits
/// with a non-zero status. This allows lockfile refresh to be best-effort in
/// environments where not all language ecosystems are installed.
pub fn run_optional(bin: &str, args: &[&str]) {
    let cmd = format!("{} {}", bin, args.join(" "));
    info!("Running (optional): {cmd}");
    match std::process::Command::new(bin).args(args).status() {
        Ok(status) => {
            if !status.success() {
                // The command is declared optional; a non-zero exit here is not a problem the
                // caller needs to see at warning level. ~keep
                info!("Optional command failed with exit code {:?}: {cmd}", status.code());
            }
        }
        Err(e) => {
            // The command is declared optional; a missing binary here is not a problem the
            // caller needs to see at warning level. ~keep
            info!("Optional command not found or failed to execute: {cmd} ({})", e);
        }
    }
}

/// Prepend `KEY=VALUE` exports inside the shell command string. macOS SIP
/// strips `DYLD_*` env vars when re-execing through `/bin/sh`, so passing them
/// via `Command::env` alone is unreliable. Inlining the export into the shell
/// command itself keeps the values in the shell's own environment, which then
/// propagates to its children normally.
///
/// The value is *prepended* to any existing value of the variable rather than
/// replacing it: these vars are search paths (`PATH`, `LD_LIBRARY_PATH`,
/// `DYLD_LIBRARY_PATH`). On Windows the library search path is `PATH` itself —
/// replacing it wholesale would wipe out `uv`, `python`, and every other tool
/// on the path. The `${KEY:+:$KEY}` guard appends the original value with a
/// `:` separator only when it is non-empty, so there is no stray leading or
/// trailing `:` when the variable was previously unset.
fn inline_env_in_shell_cmd(cmd: &str, env_vars: &[(&str, String)]) -> String {
    if env_vars.is_empty() {
        return cmd.to_string();
    }
    let mut prefix = String::new();
    for (key, value) in env_vars {
        let escaped = value.replace('\'', "'\\''");
        prefix.push_str(&format!("export {key}='{escaped}'\"${{{key}:+:${key}}}\"; "));
    }
    format!("{prefix}{cmd}")
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
    run_command_streamed_with_env(cmd, label, &[])
}

/// Run a shell command with stdout/stderr streamed and optional environment variables.
///
/// `env_vars` is a list of (key, value) tuples to set in the spawned process.
pub(crate) fn run_command_streamed_with_env(
    cmd: &str,
    label: Option<&str>,
    path_env: &[(&str, String)],
) -> anyhow::Result<()> {
    run_command_streamed_with_envs(cmd, label, path_env, &[])
}

pub(crate) fn run_command_streamed_with_envs(
    cmd: &str,
    label: Option<&str>,
    path_env: &[(&str, String)],
    exact_env: &[(&str, String)],
) -> anyhow::Result<()> {
    // Log (and use for error text) the ORIGINAL `cmd` plus env var NAMES only, never
    // `cmd_with_env` -- that string carries every env var's literal value (an e2e-policy
    // token, a mock-server URL that may itself embed a secret query param), and this line was
    // previously the only thing standing between such a value and CI log output. `description`
    // below also becomes what `run_prepared_command` puts in its own error messages, so this
    // redaction covers both the success and failure logging paths in one place.
    let description = match env_key_names(path_env.iter().chain(exact_env.iter()).map(|(key, _)| *key)) {
        Some(keys) => format!("{cmd} (env: {keys})"),
        None => cmd.to_string(),
    };
    info!("Running: {description}");
    let command = prepare_shell_command(cmd, path_env, exact_env);

    process_exec::run_prepared_command(command, label, &description)
}

/// Comma-joined env var NAMES for logging, omitting every value. `None` when `env_vars` is
/// empty (nothing to append to the logged command).
fn env_key_names<'a>(env_vars: impl Iterator<Item = &'a str>) -> Option<String> {
    let keys = env_vars.collect::<Vec<_>>();
    if keys.is_empty() {
        return None;
    }
    Some(keys.join(", "))
}

fn prepare_shell_command(
    cmd: &str,
    path_env: &[(&str, String)],
    exact_env: &[(&str, String)],
) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", &inline_env_in_shell_cmd(cmd, path_env)]);
    command.envs(exact_env.iter().map(|(key, value)| (*key, value)));
    command
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

/// Streamed variant with an optional working directory and timeout.
///
/// `cwd` is the directory the child shell inherits as its working directory. Used
/// by `setup` so install commands run from each binding's manifest directory
/// (e.g. `packages/swift` for `swift package resolve`). Output is piped to the
/// parent's stderr live (line-prefixed when `label` is set).
///
/// When `timeout_secs` is set the child leads its own process group and the deadline kills that
/// whole group -- the `sh` wrapper alone is never what needs killing, since it is the `gradlew`
/// and the daemon underneath it that outlive the budget. The untimed path is left in the
/// terminal's foreground group deliberately, where Ctrl-C already reaches it by delivery. ~keep
pub(crate) fn run_command_streamed_with_cwd_and_timeout(
    cmd: &str,
    label: Option<&str>,
    timeout_secs: Option<u64>,
    cwd: Option<&Path>,
) -> anyhow::Result<()> {
    run_command_streamed_full(cmd, label, timeout_secs, &[], cwd)
}

fn run_command_streamed_full(
    cmd: &str,
    label: Option<&str>,
    timeout_secs: Option<u64>,
    env_vars: &[(&str, String)],
    cwd: Option<&Path>,
) -> anyhow::Result<()> {
    let Some(secs) = timeout_secs else {
        return run_command_streamed_with_env(cmd, label, env_vars);
    };
    let cmd_with_env = inline_env_in_shell_cmd(cmd, env_vars);
    // See `run_command_streamed_with_env`'s matching comment: log `cmd` plus env var NAMES
    // only, never `cmd_with_env`, which carries every value verbatim.
    let logged = match env_key_names(env_vars.iter().map(|(key, _)| *key)) {
        Some(keys) => format!("{cmd} (env: {keys})"),
        None => cmd.to_string(),
    };
    if let Some(dir) = cwd {
        info!("Running (timeout {secs}s, cwd={}): {logged}", dir.display());
    } else {
        info!("Running (timeout {secs}s): {logged}");
    }
    let prefix = label.map(|l| format!("[{l}] "));

    let mut command = std::process::Command::new("sh");
    command.args(["-c", &cmd_with_env]);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    if prefix.is_some() {
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }

    let mut child = GroupChild::spawn(&mut command).with_context(|| format!("failed to spawn: {cmd}"))?;
    let out_pump = prefix.clone().zip(child.take_stdout()).map(pump_in_background);
    let err_pump = prefix.clone().zip(child.take_stderr()).map(pump_in_background);

    let waited = child
        .wait_within(std::time::Duration::from_secs(secs), &cmd)
        .with_context(|| format!("failed to wait on: {cmd}"))?;
    let Deadline::Exited(status) = waited else {
        anyhow::bail!("Command timed out after {secs}s: {cmd}");
    };

    finish_pumping_or_kill(&mut child, [out_pump, err_pump], cmd);
    if !status.success() {
        anyhow::bail!("Command failed: {cmd}");
    }
    Ok(())
}

/// Copies one of the child's streams to alef's stderr on a thread that reports when it reaches
/// end of stream, rather than one that is joined unconditionally.
fn pump_in_background<R: std::io::Read + Send + 'static>((prefix, stream): (String, R)) -> StreamDrain {
    spawn_drain(move || {
        pump_lines(stream, &prefix);
        Ok(())
    })
}

/// Waits out the pumps that were still running when the command exited, then kills the tree if
/// any of them is still holding a pipe.
///
/// The pumps used to be `join`ed. A child hands its stdout and stderr to every descendant it
/// starts, so a descendant that outlives the command -- a Gradle daemon, anything a hook
/// backgrounded -- keeps the write end open and that `join` never returns: a bounded wait
/// followed by an unbounded drain is still unbounded, and it is how a command under a 1800s
/// budget ran past half an hour without ever being killed. ~keep
fn finish_pumping_or_kill(child: &mut GroupChild, pumps: [Option<StreamDrain>; 2], cmd: &str) {
    let pending = pumps.iter().flatten().collect::<Vec<_>>();
    if pending.is_empty() {
        return;
    }
    if wait_for_drains(pending, OUTPUT_DRAIN_GRACE).unwrap_or(false) {
        return;
    }
    tracing::warn!(
        command = cmd,
        grace_seconds = OUTPUT_DRAIN_GRACE.as_secs(),
        "a descendant outlived the command still holding its output pipes; killing the process group"
    );
    child.kill_tree();
}

/// Run a shell command with an optional timeout.
///
/// If `timeout_secs` is `Some(n)`, kills the child's whole process group after `n` seconds and
/// returns a "timed out" error; the returned output is bounded by that deadline plus
/// [`OUTPUT_DRAIN_GRACE`], whatever the child's descendants do with the pipes they inherited.
/// Otherwise behaves identically to [`run_command_captured`].
pub(crate) fn run_command_captured_with_timeout(
    cmd: &str,
    timeout_secs: Option<u64>,
) -> anyhow::Result<(String, String)> {
    let Some(secs) = timeout_secs else {
        return run_command_captured(cmd);
    };
    info!("Running (timeout {secs}s): {cmd}");
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", cmd])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = GroupChild::spawn(&mut command).with_context(|| format!("failed to spawn: {cmd}"))?;
    // The readers run alongside the wait, not after it. A child that fills the OS pipe buffer
    // blocks on the write and never exits, so a wait that does not drain concurrently turns every
    // chatty before-hook into a command that can only end by timing out. ~keep
    let stdout = child.take_stdout().map(output_reader);
    let stderr = child.take_stderr().map(output_reader);

    let waited = child
        .wait_within(std::time::Duration::from_secs(secs), &cmd)
        .with_context(|| format!("failed to wait on: {cmd}"))?;
    let Deadline::Exited(status) = waited else {
        anyhow::bail!("Command timed out after {secs}s: {cmd}");
    };

    let drained = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE)
        .with_context(|| format!("failed to read the output of: {cmd}"))?;
    if !drained.complete {
        tracing::warn!(
            command = cmd,
            grace_seconds = OUTPUT_DRAIN_GRACE.as_secs(),
            "a descendant outlived the command still holding its output pipes; killing the process group"
        );
        child.kill_tree();
    }
    if !status.success() {
        anyhow::bail!("Command failed: {cmd}\n{}", drained.stderr);
    }
    Ok((drained.stdout, drained.stderr))
}

/// Run a shell command, capturing stdout and stderr.
///
/// Returns the captured output on success.  On failure the error includes
/// the command string, captured stderr **and** stdout — many tools (pnpm,
/// napi, cargo when wrapped by sccache) write diagnostics to stdout, so
/// surfacing only stderr leaves CI failures opaque.
pub(crate) fn run_command_captured(cmd: &str) -> anyhow::Result<(String, String)> {
    run_command_captured_with_env(cmd, &[])
}

/// Run a shell command with child-scoped environment variables while capturing output.
///
/// This is the runner every per-language backend build command (`cargo build`, `wasm-pack
/// build`, `napi build`, `maturin develop`, ...) goes through -- see
/// `cli::pipeline::commands::build::build_command_for`. It used to buffer both streams entirely
/// in memory via `Command::output()` and only ever look at them after the child exited, so a
/// build that failed after minutes of real compiler output could still surface nothing: a plain
/// `Command::output()`/`wait()` error (the child never producing a clean `Output`, e.g. because a
/// descendant kept a pipe open past this process's own lifetime) carries no captured bytes at
/// all, only the OS-level wait failure. Streams are now read on background threads as they
/// arrive, mirrored to alef's own stderr live (so a long build still looks alive) and captured at
/// the same time, matching the fix `run_run_command` already applies to post-build steps -- see
/// `cli::pipeline::commands::build::run_run_command` and its `output_reader_tee` doc comment.
/// Taken before the wait, not after: a child that fills the OS pipe buffer blocks on the write and
/// never exits if nothing is draining it concurrently. ~keep
pub(crate) fn run_command_captured_with_env(
    cmd: &str,
    environment: &[(&str, &str)],
) -> anyhow::Result<(String, String)> {
    info!("Running: {cmd}");
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", cmd])
        .envs(environment.iter().copied())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().with_context(|| format!("failed to spawn: {cmd}"))?;

    let stdout = child.stdout.take().map(output_reader_tee);
    let stderr = child.stderr.take().map(output_reader_tee);

    let status = child.wait().with_context(|| format!("failed to wait on: {cmd}"))?;
    let drained = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE)
        .with_context(|| format!("failed to read the output of: {cmd}"))?;
    if !drained.complete {
        tracing::warn!(
            command = cmd,
            grace_seconds = OUTPUT_DRAIN_GRACE.as_secs(),
            "a descendant outlived the command still holding its output pipes; the captured output \
             below may be incomplete"
        );
    }
    if !status.success() {
        anyhow::bail!(
            "Command failed: {cmd}\n--- stderr ---\n{}\n--- stdout ---\n{}",
            drained.stderr,
            drained.stdout
        );
    }
    Ok((drained.stdout, drained.stderr))
}

/// Run a precondition command and report only whether it succeeded.
///
/// Says nothing about what a failure means — a caller that treats a failing precondition as
/// something other than "skip this language" (e.g. `build`'s dependency preconditions, which are
/// actionable and fail the run) needs the verdict without the skip warning attached to it. ~keep
pub(crate) fn precondition_passes(label: &str, cmd: &str) -> bool {
    info!("Checking precondition for {label}: {cmd}");
    let status = std::process::Command::new("sh")
        .args(["-c", cmd])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success())
}

/// Check a precondition command. Returns `true` if the command succeeds (or
/// is absent), `false` if it fails (language should be skipped).
pub(crate) fn check_precondition(lang: Language, precondition: Option<&str>) -> bool {
    let Some(cmd) = precondition else {
        return true;
    };
    if precondition_passes(&lang.to_string(), cmd) {
        return true;
    }
    // A precondition is the user's own declared skip switch; a working-as-designed skip is not
    // a warning. ~keep
    info!("Skipping {lang}: precondition failed ({cmd})");
    false
}

/// Like [`check_precondition`] but keyed by a free-form label (e.g. a registry
/// test-app name such as `brew`) rather than a [`Language`]. Returns `true` when
/// the precondition succeeds or is absent, `false` when it fails (skip).
pub(crate) fn check_precondition_named(label: &str, precondition: Option<&str>) -> bool {
    let Some(cmd) = precondition else {
        return true;
    };
    if precondition_passes(label, cmd) {
        return true;
    }
    // A precondition is the user's own declared skip switch; a working-as-designed skip is not
    // a warning. ~keep
    info!("Skipping {label}: precondition failed ({cmd})");
    false
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
///
/// Refuses to run against a path that already exists: `alef.toml` is the single most
/// hand-edited file alef ever touches (crate list, extensions, scaffold overrides), and an
/// unconditional `std::fs::write` here had no guard of any kind — not even the wrong-node
/// `exists()` check the fixture scaffolder mistakenly used elsewhere, just no check at all. A
/// second `alef init` in an already-initialized repo (muscle memory from another project, a
/// typo'd flag) would have silently discarded every hand-edit with no diagnostic. ~keep
pub fn init(config_path: &std::path::Path, languages: Option<Vec<String>>) -> anyhow::Result<()> {
    if config_path.exists() {
        anyhow::bail!(
            "refusing to overwrite existing config at {}; edit it in place, or remove it first for a fresh one",
            config_path.display()
        );
    }
    let metadata = read_crate_metadata()?;

    let langs = languages.unwrap_or_else(|| vec!["python".to_string(), "node".to_string(), "ffi".to_string()]);

    let config_content = generate_init_config(&metadata, &langs);

    std::fs::write(config_path, config_content)
        .with_context(|| format!("failed to write config to {}", config_path.display()))?;
    info!("Created {}", config_path.display());

    Ok(())
}

struct CrateMetadata {
    name: String,
    #[allow(dead_code)]
    version: String,
    repository: Option<String>,
}

fn read_crate_metadata() -> anyhow::Result<CrateMetadata> {
    let content = std::fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let value: toml::Value = toml::from_str(&content).context("failed to parse Cargo.toml")?;

    let extract = |table: &toml::Value| -> Option<CrateMetadata> {
        let name = table.get("name").and_then(|v| v.as_str())?.to_string();
        let version = table.get("version").and_then(|v| v.as_str())?.to_string();
        let repository = table.get("repository").and_then(|v| v.as_str()).map(|s| s.to_string());
        Some(CrateMetadata {
            name,
            version,
            repository,
        })
    };

    if let Some(workspace_pkg) = value.get("workspace").and_then(|w| w.get("package"))
        && let Some(meta) = extract(workspace_pkg)
    {
        return Ok(meta);
    }
    if let Some(pkg) = value.get("package")
        && let Some(meta) = extract(pkg)
    {
        return Ok(meta);
    }

    anyhow::bail!("Could not find package name and version in Cargo.toml")
}

fn generate_init_config(metadata: &CrateMetadata, languages: &[String]) -> String {
    let crate_name = metadata.name.as_str();
    let source_path = format!("crates/{}/src/lib.rs", crate_name);

    let mut config = String::new();

    config.push_str("[workspace]\n");
    config.push_str("languages = [");
    for (i, lang) in languages.iter().enumerate() {
        if i > 0 {
            config.push_str(", ");
        }
        config.push('"');
        config.push_str(lang);
        config.push('"');
    }
    config.push_str("]\n");
    config.push_str(&format!("alef_version = \"{}\"\n", env!("CARGO_PKG_VERSION")));

    config.push_str(
        "\n[workspace.tools]\n\
         # python_package_manager = \"uv\"   # uv | pip | poetry\n\
         # node_package_manager = \"pnpm\"   # pnpm | npm | yarn\n\
         # rust_dev_tools = [\"cargo-edit\", \"cargo-sort\", \"cargo-machete\", \"cargo-deny\", \"cargo-llvm-cov\"]\n",
    );

    config.push_str(&format!(
        "\n[[crates]]\nname = \"{}\"\nsources = [\"{}\"]\nversion_from = \"Cargo.toml\"\n",
        crate_name, source_path
    ));

    if let Some(repo) = metadata.repository.as_deref() {
        config.push_str(&format!("\n[crates.scaffold]\nrepository = \"{repo}\"\n"));
    }

    if languages.contains(&"python".to_string()) {
        config.push_str(&format!(
            "\n[crates.python]\nmodule_name = \"_{}\"\n",
            crate_name.replace('-', "_")
        ));
    }

    if languages.contains(&"node".to_string()) {
        config.push_str(&format!("\n[crates.node]\npackage_name = \"{crate_name}\"\n"));
    }

    if languages.contains(&"ffi".to_string()) {
        config.push_str(&format!(
            "\n[crates.ffi]\nprefix = \"{}\"\n",
            crate_name.replace('-', "_")
        ));
    }

    if languages.contains(&"go".to_string()) {
        match metadata
            .repository
            .as_deref()
            .and_then(crate::core::config::derive_go_module_from_repo)
        {
            Some(module) => config.push_str(&format!("\n[crates.go]\nmodule = \"{module}\"\n")),
            None => {
                config.push_str("\n[crates.go]\n# module = \"github.com/<org>/<repo>\"  # set the Go module path\n");
            }
        }
    }

    if languages.contains(&"ruby".to_string()) {
        config.push_str(&format!(
            "\n[crates.ruby]\ngem_name = \"{}\"\n",
            crate_name.replace('-', "_")
        ));
    }

    if languages.contains(&"java".to_string()) {
        match metadata
            .repository
            .as_deref()
            .and_then(crate::core::config::derive_reverse_dns_package)
        {
            Some(pkg) => config.push_str(&format!("\n[crates.java]\npackage = \"{pkg}\"\n")),
            None => {
                config.push_str("\n[crates.java]\n# package = \"com.example.<org>\"  # set the Java package\n");
            }
        }
    }

    if languages.contains(&"csharp".to_string()) {
        config.push_str(&format!(
            "\n[crates.csharp]\nnamespace = \"{}\"\n",
            to_pascal_case(crate_name)
        ));
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
    fn init_refuses_to_overwrite_an_existing_config() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let config_path = directory.path().join("alef.toml");
        let hand_edited = "[workspace]\nlanguages = [\"python\"]\n\n[[crates]]\nname = \"hand-edited\"\n";
        std::fs::write(&config_path, hand_edited).expect("seed hand-edited config");

        let result = init(&config_path, Some(vec!["node".to_string()]));

        assert!(result.is_err(), "init must refuse an already-initialized config path");
        let preserved = std::fs::read_to_string(&config_path).expect("read config after refused init");
        assert_eq!(
            preserved, hand_edited,
            "a refused init must leave the existing config byte-for-byte untouched"
        );
    }

    #[test]
    fn run_before_aborts_on_first_failing_command() {
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

    fn fixture_metadata(name: &str, repository: Option<&str>) -> CrateMetadata {
        CrateMetadata {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            repository: repository.map(|s| s.to_string()),
        }
    }

    #[test]
    fn generate_init_config_includes_version() {
        let config = generate_init_config(&fixture_metadata("my-lib", None), &["python".to_string()]);
        let expected = format!("alef_version = \"{}\"", env!("CARGO_PKG_VERSION"));
        assert!(
            config.contains(&expected),
            "config should contain alef_version key: {config}"
        );
    }

    #[test]
    fn generate_init_config_parses_as_valid_new_alef_config() {
        let config_str = generate_init_config(&fixture_metadata("my-lib", None), &["python".to_string()]);
        let cfg: crate::core::config::NewAlefConfig =
            toml::from_str(&config_str).expect("generated config should parse as valid NewAlefConfig");
        let resolved = cfg.resolve().expect("generated config should resolve");
        let config = &resolved[0];
        assert_eq!(config.name, "my-lib");
    }

    #[test]
    fn generate_init_config_derives_java_and_go_from_repository() {
        let meta = fixture_metadata("my-lib", Some("https://github.com/foo-org/my-lib"));
        let config = generate_init_config(&meta, &["java".to_string(), "go".to_string()]);
        assert!(
            config.contains("repository = \"https://github.com/foo-org/my-lib\""),
            "expected scaffold.repository: {config}"
        );
        assert!(
            config.contains("module = \"github.com/foo-org/my-lib\""),
            "expected derived go.module: {config}"
        );
        assert!(
            config.contains("package = \"com.github.foo_org\""),
            "expected derived java.package: {config}"
        );
    }

    #[test]
    fn generate_init_config_emits_todo_when_repository_missing() {
        let config = generate_init_config(
            &fixture_metadata("my-lib", None),
            &["java".to_string(), "go".to_string()],
        );
        assert!(
            !config.contains("sample_crate-dev"),
            "config must not leak sample_crate-dev defaults: {config}"
        );
        assert!(
            config.contains("# module ="),
            "expected commented-out go.module placeholder: {config}"
        );
        assert!(
            config.contains("# package ="),
            "expected commented-out java.package placeholder: {config}"
        );
    }

    #[test]
    fn run_command_captured_with_timeout_succeeds_within_limit() {
        let result = run_command_captured_with_timeout("echo hello", Some(5));
        assert!(result.is_ok(), "Quick command should succeed with timeout");
        let (stdout, _) = result.unwrap();
        assert!(stdout.contains("hello"), "Command output should be captured");
    }

    #[test]
    fn run_command_captured_with_timeout_kills_on_timeout() {
        let result = run_command_captured_with_timeout("sleep 5", Some(1));
        assert!(result.is_err(), "Command that exceeds timeout should return error");
        let err_msg = format!("{:?}", result);
        assert!(err_msg.contains("timed out"), "Error should mention timeout");
    }

    #[test]
    fn run_command_captured_without_timeout() {
        let result = run_command_captured_with_timeout("echo test", None);
        assert!(result.is_ok(), "Command without timeout should succeed");
        let (stdout, _) = result.unwrap();
        assert!(stdout.contains("test"), "Command output should be captured");
    }

    /// Regression coverage for the "backend build swallows a failed command's output" defect:
    /// every per-language backend build (`build_command_for` + `run_command_captured_with_env`)
    /// went through `Command::output()`, which blocks reading each pipe to end-of-stream. A
    /// background job started with `&` and never waited on inherits the parent shell's stdout,
    /// so the shell itself exits immediately while the OS pipe write end stays open in the leaked
    /// child -- `Command::output()` then blocks for as long as that descendant holds the pipe,
    /// which is exactly how a real `wasm-pack build` run produced zero lines of diagnostic output
    /// after 375 seconds. The fix drains for at most `OUTPUT_DRAIN_GRACE` after the *direct*
    /// child exits, so this must return quickly even though the leaked descendant is still
    /// running. Reverting to `Command::output()` makes this test hang for the descendant's whole
    /// sleep instead of returning within the assertion's bound. ~keep
    #[cfg(unix)]
    #[test]
    fn run_command_captured_with_env_does_not_hang_on_a_leaked_descendant_holding_the_pipe() {
        let cmd = "echo direct-child-output; (sleep 10 &) ; true";
        let started = std::time::Instant::now();
        let result = run_command_captured_with_env(cmd, &[]);
        let elapsed = started.elapsed();

        let (stdout, _stderr) = result.expect("the direct child exits zero");
        assert!(
            stdout.contains("direct-child-output"),
            "the direct child's own output must still be captured: {stdout}"
        );
        crate::test_support::assert_elapsed_under(
            "must return within the output drain grace period, not hang for the leaked descendant's full sleep",
            elapsed,
            std::time::Duration::from_secs(8),
        );
    }

    /// Negative control for the test above: a command with no descendants at all must still
    /// return its output promptly -- proving the drain-grace bound isn't itself adding a fixed
    /// delay to the common case.
    #[test]
    fn run_command_captured_with_env_returns_promptly_with_no_leaked_descendants() {
        let started = std::time::Instant::now();
        let result = run_command_captured_with_env("echo quick", &[]);
        let elapsed = started.elapsed();

        let (stdout, _stderr) = result.expect("a plain command exits zero");
        assert!(stdout.contains("quick"), "output must be captured: {stdout}");
        crate::test_support::assert_elapsed_under(
            "a command with no leaked descendants must not pay the drain grace period",
            elapsed,
            std::time::Duration::from_secs(2),
        );
    }

    #[test]
    fn inline_env_in_shell_cmd_with_no_env_returns_cmd_unchanged() {
        let result = inline_env_in_shell_cmd("echo hi", &[]);
        assert_eq!(result, "echo hi", "Empty env_vars should leave the command untouched");
    }

    #[test]
    fn inline_env_in_shell_cmd_prepends_to_existing_var_value() {
        let env = vec![("PATH", "/abs/target/release".to_string())];
        let result = inline_env_in_shell_cmd("uv run pytest", &env);
        assert_eq!(
            result, "export PATH='/abs/target/release'\"${PATH:+:$PATH}\"; uv run pytest",
            "PATH must be prepended via the ${{PATH:+:$PATH}} guard, not replaced"
        );
    }

    #[test]
    fn inline_env_in_shell_cmd_uses_prepend_guard_for_each_var() {
        let env = vec![
            ("DYLD_FALLBACK_LIBRARY_PATH", "/lib/dir".to_string()),
            ("DYLD_LIBRARY_PATH", "/lib/dir".to_string()),
        ];
        let result = inline_env_in_shell_cmd("cargo test", &env);
        assert_eq!(
            result,
            "export DYLD_FALLBACK_LIBRARY_PATH='/lib/dir'\"${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}\"; \
             export DYLD_LIBRARY_PATH='/lib/dir'\"${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}\"; cargo test",
            "Every var must use the prepend guard so a pre-existing value is preserved"
        );
    }

    #[test]
    fn inline_env_in_shell_cmd_escapes_single_quotes_in_value() {
        let env = vec![("PATH", "/weird'dir".to_string())];
        let result = inline_env_in_shell_cmd("run", &env);
        assert_eq!(
            result, "export PATH='/weird'\\''dir'\"${PATH:+:$PATH}\"; run",
            "Single quotes in the value must be escaped for the shell"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inline_env_in_shell_cmd_prepend_guard_evaluates_correctly_in_shell() {
        let original_path = std::env::var("PATH").expect("PATH should be set in the test environment");
        let env = vec![("PATH", "/new/dir".to_string())];
        let cmd = inline_env_in_shell_cmd("printf '%s' \"$PATH\"", &env);
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output()
            .expect("sh should run the generated command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout,
            format!("/new/dir:{original_path}"),
            "Generated command must prepend the new dir and keep the original PATH intact"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inline_env_in_shell_cmd_prepend_guard_has_no_stray_colon_when_var_empty() {
        let env = vec![("LD_LIBRARY_PATH", "/lib/dir".to_string())];
        let cmd = inline_env_in_shell_cmd("printf '%s' \"$LD_LIBRARY_PATH\"", &env);
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .env_remove("LD_LIBRARY_PATH")
            .output()
            .expect("sh should run the generated command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout, "/lib/dir",
            "An unset variable must yield just the new dir with no stray colon"
        );
    }

    #[test]
    fn exact_environment_value_is_not_part_of_shell_argv() {
        let exact_value = "literal'; touch /tmp/alef-env-injection; #".to_string();
        let command = prepare_shell_command(
            "printf '%s' \"$ALEF_EXACT_VALUE\"",
            &[],
            &[("ALEF_EXACT_VALUE", exact_value.clone())],
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(
            arguments.iter().all(|argument| !argument.contains(&exact_value)),
            "exact environment values must never enter the sh -c argument"
        );
        assert!(command.get_envs().any(|(key, value)| {
            key == "ALEF_EXACT_VALUE" && value.is_some_and(|value| value == exact_value.as_str())
        }));
    }

    #[cfg(unix)]
    #[test]
    fn exact_environment_value_reaches_shell_without_interpretation() {
        let marker = std::env::temp_dir().join(format!("alef-env-injection-{}", std::process::id()));
        let exact_value = format!("literal'; touch {}; #", marker.display());
        let mut command = prepare_shell_command(
            "test \"$ALEF_EXACT_VALUE\" = \"$EXPECTED_ALEF_EXACT_VALUE\"",
            &[],
            &[
                ("ALEF_EXACT_VALUE", exact_value.clone()),
                ("EXPECTED_ALEF_EXACT_VALUE", exact_value),
            ],
        );
        let status = command.status().expect("shell command should start");

        assert!(status.success(), "the child must receive the exact configured value");
        assert!(!marker.exists(), "shell syntax inside the value must remain inert");
    }
}
