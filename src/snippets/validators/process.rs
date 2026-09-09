//! Subprocess execution for snippet validation: environment sanitisation on top of the shared
//! process-group lifecycle in [`crate::process`].

use crate::process::capture::{OUTPUT_DRAIN_GRACE, collect_output_within, output_reader};
use crate::process::timed::{Deadline, GroupChild};
use crate::snippets::error::Result;

fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.next(), Some('[')) {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// A finished command's exit verdict and its two output streams, kept apart for the callers that
/// parse one of them -- `swift build --show-bin-path` prints a path on stdout while SwiftPM's
/// resolution chatter lands on stderr, and merging the two makes the path unfindable. ~keep
pub struct CapturedStreams {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a child process with a timeout and capture combined stdout/stderr.
///
/// Returns within `timeout_secs` plus [`OUTPUT_DRAIN_GRACE`], whatever the child's descendants do
/// with the pipes they inherited.
///
/// # Errors
///
/// Returns an error when the child process cannot be spawned, waited on, or times out.
pub fn run_command(command: &mut std::process::Command, timeout_secs: u64) -> Result<(bool, String)> {
    let captured = run_command_streams(command, timeout_secs)?;
    Ok((captured.success, format!("{}{}", captured.stdout, captured.stderr)))
}

/// [`run_command`] with the two output streams reported separately, under the identical bound.
///
/// # Errors
///
/// Returns an error when the child process cannot be spawned, waited on, or times out.
pub fn run_command_streams(command: &mut std::process::Command, timeout_secs: u64) -> Result<CapturedStreams> {
    sanitize_environment(command);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = GroupChild::spawn(command)
        .map_err(|err| crate::snippets::error::Error::Other(format!("spawn failed: {err}")))?;
    let stdout = child.take_stdout().map(output_reader);
    let stderr = child.take_stderr().map(output_reader);

    let timeout = std::time::Duration::from_secs(timeout_secs);
    let waited = child.wait_within(timeout, &*command);
    match waited {
        Ok(Deadline::Exited(status)) => {
            let drained = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE)
                .map_err(crate::snippets::error::Error::from)?;
            if !drained.complete {
                tracing::warn!(
                    command = ?command,
                    grace_secs = OUTPUT_DRAIN_GRACE.as_secs(),
                    "a descendant outlived the command still holding its output pipes; killing the process group"
                );
                child.kill_tree();
            }
            Ok(CapturedStreams {
                success: status.success(),
                stdout: strip_ansi_codes(&drained.stdout),
                stderr: strip_ansi_codes(&drained.stderr),
            })
        }
        Ok(Deadline::Expired) => {
            let _ = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE);
            Err(crate::snippets::error::Error::Timeout {
                command: format!("{command:?}"),
                timeout_secs,
            })
        }
        Err(err) => {
            let _ = collect_output_within(stdout, stderr, OUTPUT_DRAIN_GRACE);
            Err(crate::snippets::error::Error::Other(format!("wait failed: {err}")))
        }
    }
}

/// `HOME` is the Unix counterpart of `USERPROFILE` below: cargo, gradle, `dart pub`, `gem`, `mix`,
/// and npm all resolve their cache or config directory through it, and `env_clear` otherwise hands
/// them none. It is passed through unmodified rather than pointed at a scratch directory -- like
/// `PATH`, `TMPDIR`, and `USERPROFILE` it names a machine identity a toolchain expects to resolve
/// structurally, not consumer-specific state the allowlist exists to withhold. Per-invocation
/// isolation is already provided by `ScratchDir` and `command.current_dir`, so redirecting `HOME`
/// as well would only cost every validated snippet its shared toolchain cache. ~keep
const SANITIZED_ENVIRONMENT_VARIABLES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "WINDIR",
    "HOME",
    "TMP",
    "TEMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "GOMODCACHE",
    "GOPATH",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "JAVA_HOME",
    "R_HOME",
    "R_LIBS",
    "R_LIBS_USER",
    "R_LIBS_SITE",
];

/// The variables that identify the machine itself on Windows, allowed through in addition to
/// [`SANITIZED_ENVIRONMENT_VARIABLES`].
///
/// Sanitisation clears the child environment and re-adds an allowlist. That allowlist was
/// Unix-shaped, and on Windows a toolchain that cannot see these does not degrade -- it fails
/// with an error that names none of them. NuGet resolves its global packages folder through
/// `USERPROFILE`, and without it every `dotnet build` dies in `NuGet.targets` with
/// `Value cannot be null. (Parameter 'path1')`. rustc locates the MSVC linker by running
/// `vswhere.exe` under `ProgramFiles(x86)`, and without it falls back to the first `link.exe`
/// on `PATH` -- which on any box with Git for Windows is GNU coreutils' `link`, producing
/// `link: extra operand` and advice to install the C++ build tools that are already installed.
/// These are the same class of variable as `SYSTEMROOT` and `WINDIR`, which the list above
/// already allows, and they carry no consumer-specific state. ~keep
const WINDOWS_ENVIRONMENT_VARIABLES: &[&str] = &[
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ALLUSERSPROFILE",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "COMSPEC",
    "SystemDrive",
    "PUBLIC",
    "USERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
];

fn sanitize_environment(command: &mut std::process::Command) {
    apply_environment_allowlist(command, cfg!(windows), |key| std::env::var_os(key));
}

/// Replace `command`'s inherited environment with the allowlisted subset `lookup` can resolve,
/// keeping any variable the caller set explicitly.
///
/// `include_windows_variables` and `lookup` are parameters rather than reads of the ambient
/// platform and environment so the allowlist can be asserted on any host: a test that has to
/// mutate the real process environment to check this would be racing every other test in the
/// binary. ~keep
fn apply_environment_allowlist(
    command: &mut std::process::Command,
    include_windows_variables: bool,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) {
    let windows_variables: &[&str] = if include_windows_variables {
        WINDOWS_ENVIRONMENT_VARIABLES
    } else {
        &[]
    };
    let values: Vec<_> = SANITIZED_ENVIRONMENT_VARIABLES
        .iter()
        .chain(windows_variables)
        .filter_map(|key| lookup(key).map(|value| (*key, value)))
        .collect();
    let explicit_values = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_os_string(), value.to_os_string())))
        .collect::<Vec<_>>();
    command.env_clear();
    command.envs(values);
    command.envs(explicit_values);
    command.env("NO_COLOR", "1");
}

#[cfg(test)]
mod environment_tests {
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// Every allowlisted name mapped to a recognisable value, so a dropped variable shows up as a
    /// missing key rather than as an empty string that could have come from anywhere.
    fn fake_environment() -> HashMap<&'static str, OsString> {
        super::SANITIZED_ENVIRONMENT_VARIABLES
            .iter()
            .chain(super::WINDOWS_ENVIRONMENT_VARIABLES)
            .map(|key| (*key, OsString::from(format!("value-of-{key}"))))
            .collect()
    }

    fn sanitized(include_windows_variables: bool) -> HashMap<String, String> {
        let environment = fake_environment();
        let mut command = std::process::Command::new("does-not-run");
        command.env("EXPLICIT", "kept");
        super::apply_environment_allowlist(&mut command, include_windows_variables, |key| {
            environment.get(key).cloned()
        });
        command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()))
            })
            .collect()
    }

    /// `HOME` is where cargo, gradle, `dart pub`, `gem`, and `mix` resolve their cache or config
    /// directory; without it, `env_clear` leaves the validated snippet's toolchain unable to find
    /// its own cache. ~keep
    #[test]
    fn home_survives_sanitisation_on_non_windows_hosts() {
        let passed = sanitized(false);

        assert_eq!(
            passed.get("HOME").map(String::as_str),
            Some("value-of-HOME"),
            "HOME must survive sanitisation so toolchains can resolve their cache/config directory"
        );
    }

    #[test]
    fn go_dependency_cache_paths_survive_sanitisation() {
        let passed = sanitized(false);

        assert_eq!(
            passed.get("GOMODCACHE").map(String::as_str),
            Some("value-of-GOMODCACHE")
        );
        assert_eq!(passed.get("GOPATH").map(String::as_str), Some("value-of-GOPATH"));
    }

    /// The two variables named here are the ones the Windows CI failures traced back to, and they
    /// are asserted individually rather than as "the list is non-empty" because dropping either
    /// one on its own is a whole language going dark: `USERPROFILE` for `dotnet`, and
    /// `ProgramFiles(x86)` for rustc's MSVC linker discovery. ~keep
    #[test]
    fn windows_toolchain_variables_survive_sanitisation_on_windows_hosts() {
        let passed = sanitized(true);

        assert_eq!(
            passed.get("USERPROFILE").map(String::as_str),
            Some("value-of-USERPROFILE"),
            "dotnet restore resolves its global packages folder through USERPROFILE"
        );
        assert_eq!(
            passed.get("ProgramFiles(x86)").map(String::as_str),
            Some("value-of-ProgramFiles(x86)"),
            "rustc finds vswhere.exe, and so link.exe, under ProgramFiles(x86)"
        );
        for key in super::WINDOWS_ENVIRONMENT_VARIABLES {
            assert!(passed.contains_key(*key), "{key} must survive sanitisation");
        }
    }

    /// The Windows names must not widen what a Unix child inherits: `USERNAME` and `PUBLIC` do
    /// exist on some Unix hosts, and sanitisation is an isolation boundary, not a convenience. ~keep
    #[test]
    fn windows_variables_are_withheld_from_non_windows_hosts() {
        let passed = sanitized(false);

        for key in super::WINDOWS_ENVIRONMENT_VARIABLES {
            assert!(
                !passed.contains_key(*key),
                "{key} must not leak into a non-Windows child"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn installed_toolchain_paths_reach_child_processes() {
        let keys = [
            "ANDROID_HOME",
            "ANDROID_SDK_ROOT",
            "JAVA_HOME",
            "R_HOME",
            "R_LIBS",
            "R_LIBS_USER",
            "R_LIBS_SITE",
        ];
        let environment: HashMap<_, _> = keys
            .iter()
            .map(|key| (*key, OsString::from(format!("path-for-{key}"))))
            .collect();
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "printf '%s\n' \"$ANDROID_HOME\" \"$ANDROID_SDK_ROOT\" \"$JAVA_HOME\" \"$R_HOME\" \"$R_LIBS\" \"$R_LIBS_USER\" \"$R_LIBS_SITE\""]);
        super::apply_environment_allowlist(&mut command, false, |key| environment.get(key).cloned());
        let output = command.output().expect("run child with injected environment");
        assert!(output.status.success());
        let expected = keys.iter().map(|key| format!("path-for-{key}\n")).collect::<String>();
        assert_eq!(String::from_utf8(output.stdout).expect("UTF-8 output"), expected);
    }

    #[cfg(unix)]
    #[test]
    fn unallowlisted_ambient_secret_is_removed_from_child_environment() {
        const CHILD_MARKER: &str = "ALEF_SANITIZER_CHILD_PROBE";
        const SECRET_KEY: &str = "ALEF_SANITIZER_SECRET_PROBE";
        const SYNTHETIC_VALUE: &str = "synthetic-ambient-value";
        const TIMEOUT_SECS: u64 = 10;
        if std::env::var_os(CHILD_MARKER).as_deref() == Some(std::ffi::OsStr::new("child")) {
            assert_eq!(
                std::env::var_os(SECRET_KEY),
                Some(OsString::from(SYNTHETIC_VALUE)),
                "probe must begin with the synthetic secret in its inherited environment"
            );
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", r#"printf '%s\n' "${ALEF_SANITIZER_SECRET_PROBE-unset}""#]);
            let output = super::run_command_streams(&mut command, TIMEOUT_SECS).expect("sanitized grandchild");
            assert!(output.success, "{}", output.stderr);
            assert_eq!(
                output.stdout, "unset\n",
                "unallowlisted ambient secret reached sanitized child"
            );
            return;
        }
        let mut command = std::process::Command::new(std::env::current_exe().expect("current test executable"));
        command.args([
            "--exact",
            "snippets::validators::process::environment_tests::unallowlisted_ambient_secret_is_removed_from_child_environment",
            "--nocapture",
        ]).env(CHILD_MARKER, "child").env(SECRET_KEY, SYNTHETIC_VALUE);
        let output = super::run_command_streams(&mut command, TIMEOUT_SECS).expect("isolated environment probe");
        assert!(
            output.success,
            "isolated sanitizer probe failed: {}{}",
            output.stdout, output.stderr
        );
        assert!(
            output.stdout.contains("1 passed; 0 failed"),
            "exact child test must run once: {}",
            output.stdout
        );
    }

    #[test]
    fn explicitly_set_variables_outlive_the_environment_clear() {
        let passed = sanitized(true);

        assert_eq!(passed.get("EXPLICIT").map(String::as_str), Some("kept"));
        assert_eq!(passed.get("NO_COLOR").map(String::as_str), Some("1"));
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    use std::time::{Duration, Instant};

    #[test]
    fn drains_output_larger_than_an_os_pipe_buffer() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "dd if=/dev/zero bs=131072 count=1 2>/dev/null"]);

        let (success, output) = super::run_command(&mut command, 5).expect("large-output command");

        assert!(success);
        assert_eq!(output.len(), 131_072);
    }

    const PROCESS_SETTLE_POLL: Duration = Duration::from_millis(20);
    const PROCESS_SETTLE_LIMIT: Duration = Duration::from_secs(5);

    fn is_alive(pid: i32) -> bool {
        // SAFETY: signal 0 performs error checking only and sends nothing.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_until_gone(pid: i32) -> bool {
        let deadline = Instant::now() + PROCESS_SETTLE_LIMIT;
        while Instant::now() < deadline {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        }
        !is_alive(pid)
    }

    /// Reads the pid a fixture shell wrote to `marker`, waiting for it to appear.
    fn announced_pid(marker: &std::path::Path) -> i32 {
        let started = Instant::now();
        loop {
            crate::test_support::assert_elapsed_under(
                "the fixture never announced a pid",
                started.elapsed(),
                PROCESS_SETTLE_LIMIT,
            );
            if let Ok(contents) = std::fs::read_to_string(marker)
                && let Ok(pid) = contents.trim().parse::<i32>()
            {
                return pid;
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        }
    }

    /// The deadline has to be enforced against the *tree*, and the tree has to be gone afterwards.
    ///
    /// Asserting that the timeout branch was entered would restate the bug rather than catch it:
    /// the orphaned hook tree this fixes was produced by code whose timeout branch ran. So the
    /// grandchild announces its own pid and the test waits for that pid to stop existing. ~keep
    #[test]
    fn an_overrunning_command_is_killed_at_the_deadline_along_with_its_grandchildren() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("grandchild.pid");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", &format!("sleep 60 & echo $! > {}; sleep 60", marker.display())]);
        let started = Instant::now();

        let error = super::run_command(&mut command, 1).expect_err("command must time out");
        let grandchild = announced_pid(&marker);

        assert!(matches!(error, crate::snippets::error::Error::Timeout { .. }));
        assert!(
            started.elapsed() < Duration::from_secs(1) + super::OUTPUT_DRAIN_GRACE + PROCESS_SETTLE_LIMIT,
            "run_command overran its own deadline by more than the drain grace"
        );
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} outlived the timeout that killed its parent"
        );
    }

    /// The regression this whole bound exists for. `sh` exits immediately and successfully, so the
    /// timed wait is satisfied at once -- but the descendant it backgrounded inherited stdout and
    /// stderr, and a drain that waits for end of stream waits for *that* process. Before the
    /// bound, this call took 20 seconds under a 1-second budget and returned `Ok`: a configured
    /// timeout that was present, entered, and did nothing. ~keep
    #[test]
    fn a_descendant_holding_the_pipes_cannot_outlive_the_drain_grace() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("holder.pid");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", &format!("sleep 60 & echo $! > {}; exit 0", marker.display())]);
        let started = Instant::now();

        let (success, _) = super::run_command(&mut command, 1).expect("the command itself succeeds");
        let elapsed = started.elapsed();
        let holder = announced_pid(&marker);

        assert!(success, "the command's own exit status must still be reported");
        crate::test_support::assert_elapsed_under(
            "draining a leaked pipe holder took longer than the drain grace",
            elapsed,
            super::OUTPUT_DRAIN_GRACE + PROCESS_SETTLE_LIMIT,
        );
        assert!(
            wait_until_gone(holder),
            "pipe holder {holder} was left running after run_command returned"
        );
    }

    /// A caller that parses one stream must not have to find it inside the other: SwiftPM prints
    /// the bin path on stdout while its resolution chatter goes to stderr, and merging them is
    /// what made bounding that call require a separated capture in the first place. ~keep
    #[test]
    fn the_two_output_streams_are_reported_separately_to_callers_that_parse_one() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo /path/to/bin; echo 'warning: resolving' 1>&2"]);

        let captured = super::run_command_streams(&mut command, 5).expect("well-behaved command");

        assert!(captured.success);
        assert_eq!(captured.stdout, "/path/to/bin\n");
        assert_eq!(captured.stderr, "warning: resolving\n");
    }

    /// The bound must not cost a well-behaved command its output: a child that writes and exits is
    /// still drained in full, not truncated by the grace. ~keep
    #[test]
    fn a_command_that_exits_cleanly_still_reports_all_of_its_output() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo out; echo err 1>&2"]);

        let (success, output) = super::run_command(&mut command, 5).expect("well-behaved command");

        assert!(success);
        assert_eq!(output, "out\nerr\n");
    }
}
