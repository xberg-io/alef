//! Runs a fixture's own `mise.toml` task as a multi-step test harness driver.
//!
//! Shared by the crate's unit tests (`src/test_support.rs` includes this file by path) and by any
//! integration test that declares `#[path = "support/mise_task.rs"] mod mise_task;`, so it depends
//! on `std`, `which` and `toml` only.
//!
//! A fixture directory is untrusted to mise and sits below other mise configs: the alef checkout
//! when it is a committed fixture, and whatever encloses the system temp directory otherwise
//! (the polyrepo root's `mise.toml` encloses a worktree checkout). Without isolation mise refuses
//! the fixture's config, or merges a parent's tools and tasks into it, so the run would depend on
//! where the checkout lives. ~keep

#![allow(dead_code, reason = "each including test binary uses a subset of these helpers")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the executable a fixture harness needs on `PATH`.
pub(crate) const MISE: &str = "mise";

/// `mise run <task>` in `directory`, reading `directory`'s own `mise.toml` and no parent config.
///
/// The caller appends task arguments (after `--` when they start with a dash), environment and
/// output handling. Spawning fails like any other missing program when mise is not installed.
pub(crate) fn mise_run(directory: &Path, task: &str) -> Command {
    let mut command = Command::new(which::which(MISE).unwrap_or_else(|_| PathBuf::from(MISE)));
    command.args(["run", task]).current_dir(directory);
    isolate(&mut command, directory);
    command
}

/// Add only the fixture's required tools, using their exact committed repository declarations.
pub(crate) fn fixture_config_with_tools(tasks: &str, tools: &[&str]) -> String {
    let canonical: toml::Value = toml::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/mise.toml")))
        .expect("committed Mise configuration");
    let selected: toml::Table = tools
        .iter()
        .map(|tool| {
            let pin = canonical["tools"]
                .get(*tool)
                .unwrap_or_else(|| panic!("fixture tool {tool} must have a committed pin"));
            ((*tool).to_owned(), pin.clone())
        })
        .collect();
    let header = toml::to_string(&toml::Table::from_iter([(
        "tools".into(),
        toml::Value::Table(selected),
    )]))
    .expect("fixture tool declarations");
    format!("{header}\n{tasks}")
}

/// Trust exactly `directory`'s config and stop the config search above it.
fn isolate(command: &mut Command, directory: &Path) {
    let mut trusted = vec![directory.to_path_buf()];
    // ~keep mise compares canonical paths, and macOS temp directories sit behind /var -> /private/var.
    if let Ok(canonical) = directory.canonicalize()
        && canonical != directory
    {
        trusted.push(canonical);
    }
    let ceilings: Vec<PathBuf> = trusted
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    command
        .env("MISE_TRUSTED_CONFIG_PATHS", join(&trusted))
        .env("MISE_CEILING_PATHS", join(&ceilings))
        // Tests read a step's output verbatim, so mise must not prefix it with the task name. ~keep
        .env("MISE_TASK_OUTPUT", "interleave")
        .env("MISE_TASK_RUN_AUTO_INSTALL", "false")
        // ~keep a developer's global config or profile must not add tools, env or settings to a fixture.
        .env("MISE_GLOBAL_CONFIG_FILE", directory.join(".no-global-mise.toml"))
        .env_remove("MISE_ENV");
    if cfg!(windows) {
        // ~keep mise's Windows default is `cmd /c`, but the fixtures' steps are POSIX shell.
        let bash = git_bash().expect("Git for Windows bash.exe is required to run POSIX fixture tasks");
        let bash = bash.to_string_lossy().replace('\\', "/");
        command.env(
            "MISE_WINDOWS_DEFAULT_INLINE_SHELL_ARGS",
            format!("\"{bash}\" -o errexit -c"),
        );
    }
}

fn join(paths: &[PathBuf]) -> OsString {
    std::env::join_paths(paths).expect("fixture paths contain no path-list separator")
}

/// Git for Windows' `bash.exe`, never the System32 WSL launcher that precedes it on `PATH`.
fn git_bash() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| Path::new(&root).join("Git").join("bin").join("bash.exe"))
        .find(|candidate| candidate.is_file())
}
