//! Locating toolchain executables before spawning them.

/// Build a [`std::process::Command`] for `program`, resolving it on `PATH` first.
///
/// Windows resolves a bare program name by appending `.exe` and nothing else, while
/// [`which::which`] resolves it against the whole of `PATHEXT`. A toolchain shipped as a batch
/// shim -- `kotlinc.bat`, `gradle.bat`, `mix.bat`, `npm.cmd` -- is therefore *found* by every
/// `is_available()` check in this crate and then fails to spawn at all, with
/// `NotFound: program not found` and no mention of the extension. Handing the resolved path to
/// `Command` closes that gap: the path carries the `.bat`/`.cmd` suffix, which std routes
/// through `cmd.exe`.
///
/// Falls back to the bare name when resolution fails, so the caller still gets std's own
/// spawn error rather than a different one from here. ~keep
pub fn tool_command(program: &str) -> std::process::Command {
    which::which(program).map_or_else(|_| std::process::Command::new(program), std::process::Command::new)
}

/// Build a [`std::process::Command`] for `bash`, skipping Windows' WSL launcher.
///
/// `C:\Windows\System32\bash.exe` is not a shell. It is the WSL launcher, and on a machine with
/// no distribution installed it prints "Windows Subsystem for Linux has no installed
/// distributions" and exits non-zero for every invocation. It also sits on `PATH` ahead of Git
/// for Windows, so a `which("bash")` probe finds it, reports bash as available, and every
/// `bash -c` against it then fails -- which is exactly how five generated-shell-script oracles
/// (the Homebrew run-tests renderer, the C download script, the PHP registry installer) came to
/// red `Test (windows-latest)` while passing everywhere else.
///
/// Git for Windows ships a real `bash.exe` and is present on the GitHub Windows images, so
/// prefer it, then any `PATH` entry that is not the System32 launcher, and only then fall back
/// to the bare name so the caller still reports std's own spawn error. ~keep
pub fn bash_command() -> std::process::Command {
    if !cfg!(windows) {
        return tool_command("bash");
    }
    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            let candidate = std::path::Path::new(&root).join("Git").join("bin").join("bash.exe");
            if candidate.is_file() {
                return std::process::Command::new(candidate);
            }
        }
    }
    if let Ok(candidates) = which::which_all("bash") {
        for candidate in candidates {
            if !is_wsl_launcher(&candidate) {
                return std::process::Command::new(candidate);
            }
        }
    }
    std::process::Command::new("bash")
}

/// Whether a resolved `bash` path is Windows' WSL launcher rather than a shell.
///
/// Matched on the parent directory, not the file name: the launcher is always
/// `System32\\bash.exe` (or its `SysWOW64` redirect), and a real bash never lives in either.
///
/// Splits the string on both separators instead of going through [`std::path::Path`], because a
/// Windows path has no parent at all under Unix path semantics -- a `Path`-based check would
/// answer "not the launcher" for every input when this crate's own tests run on Linux or macOS,
/// which is precisely where the Windows behaviour needs to be pinned. ~keep
fn is_wsl_launcher(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy();
    let mut segments = text.rsplit(['/', '\\']);
    let _file_name = segments.next();
    segments
        .next()
        .is_some_and(|parent| parent.eq_ignore_ascii_case("system32") || parent.eq_ignore_ascii_case("syswow64"))
}

#[cfg(test)]
mod tests {
    use super::tool_command;

    /// A resolvable tool must be spawned by absolute path, not by the bare name it was asked
    /// for -- that substitution is the entire fix, and a `Command` still holding the bare name
    /// is the Windows failure this guards. ~keep
    #[test]
    fn a_resolvable_tool_is_spawned_by_its_resolved_path() {
        let resolved = which::which("cargo").expect("cargo is on PATH for the test suite");

        let command = tool_command("cargo");

        assert_eq!(std::path::Path::new(command.get_program()), resolved);
        assert!(std::path::Path::new(command.get_program()).is_absolute());
    }

    /// An unresolvable name must still produce a runnable `Command` so the caller reports std's
    /// own spawn failure, rather than this helper inventing an error of its own. ~keep
    #[test]
    fn an_unresolvable_tool_falls_back_to_the_bare_program_name() {
        let command = tool_command("alef-no-such-toolchain-exists");

        assert_eq!(command.get_program(), "alef-no-such-toolchain-exists");
    }
}

#[cfg(test)]
mod bash_tests {
    use super::{bash_command, is_wsl_launcher};

    /// The System32 entry is the WSL launcher, never a shell. Recognising it by directory is
    /// what lets `bash_command` walk past it to a real bash instead of reporting the launcher
    /// as an available shell. ~keep
    #[test]
    fn the_system32_entry_is_recognised_as_the_wsl_launcher() {
        assert!(is_wsl_launcher(std::path::Path::new(r"C:\Windows\System32\bash.exe")));
        assert!(is_wsl_launcher(std::path::Path::new(r"C:\Windows\SysWOW64\bash.exe")));
    }

    /// A real bash -- Git for Windows', or any Unix one -- must not be mistaken for the
    /// launcher, or the helper would walk past the only working shell it has. ~keep
    #[test]
    fn a_real_bash_is_not_mistaken_for_the_launcher() {
        assert!(!is_wsl_launcher(std::path::Path::new(
            r"C:\Program Files\Git\bin\bash.exe"
        )));
        assert!(!is_wsl_launcher(std::path::Path::new("/bin/bash")));
        assert!(!is_wsl_launcher(std::path::Path::new("/usr/bin/bash")));
    }

    /// Off Windows the helper must stay exactly `tool_command("bash")` -- the launcher does not
    /// exist there and a preference for a Git install would be wrong. ~keep
    #[cfg(not(windows))]
    #[test]
    fn a_unix_host_resolves_bash_the_same_way_every_other_tool_is_resolved() {
        let resolved = which::which("bash").expect("bash is on PATH for the test suite");

        assert_eq!(std::path::Path::new(bash_command().get_program()), resolved);
    }
}
