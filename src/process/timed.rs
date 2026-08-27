//! The one place that knows the order of a deadline-bounded subprocess's lifecycle.
//!
//! Spawn into a group, register that group for signal forwarding, wait, and on expiry kill the
//! group rather than the child. The order is not interchangeable: [`configure_process_group`] has
//! to run before `spawn` because it configures the `Command`, and [`termination::track`] has to
//! run after it because it needs the pid. Getting either wrong yields a tree that outlives its own
//! deadline, or one that Ctrl-C can no longer reach -- neither of which is visible at the call
//! site. Callers get [`GroupChild`] instead of the three calls in the right order. ~keep

use crate::process::termination::TrackedProcessGroup;
use crate::process::{WaitTimeout as _, configure_process_group, kill_process_tree, termination};

/// What a bounded wait found.
pub(crate) enum Deadline {
    /// The child exited on its own, with this status.
    Exited(std::process::ExitStatus),
    /// The budget elapsed first. The whole process group has already been killed and reaped.
    Expired,
}

/// A child that leads its own process group, held together with the registry slot that forwards
/// this process's termination signals to that group.
///
/// The slot is released when the value is dropped, so a group that is no longer being waited on
/// stops being swept by the Ctrl-C handler. On Windows the same field holds the job object that
/// makes the kill tree-wide, so dropping it early there costs the tree its kill rather than its
/// signal forwarding. ~keep
pub(crate) struct GroupChild {
    child: std::process::Child,
    tracked: TrackedProcessGroup,
}

impl GroupChild {
    /// Spawns `command` as the leader of a new process group and registers that group for
    /// termination forwarding.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be spawned.
    pub(crate) fn spawn(command: &mut std::process::Command) -> std::io::Result<Self> {
        configure_process_group(command);
        let child = command.spawn()?;
        let tracked = termination::track(&child);
        Ok(Self { child, tracked })
    }

    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Kills the child and every descendant it started, then reaps the child.
    ///
    /// Safe to call on a child that has already exited: the group kill fails, the fallback
    /// `Child::kill` fails, and the reap returns the status already collected.
    pub(crate) fn kill_tree(&mut self) {
        kill_process_tree(&mut self.child, &self.tracked);
        let _ = self.child.wait();
    }

    /// Waits up to `budget` for the child to exit, tearing its whole tree down when the budget
    /// runs out first.
    ///
    /// `command` names the command in the timeout warning and is only formatted when the deadline
    /// is actually missed.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be waited on. The tree is killed in that case too:
    /// a wait that failed leaves nothing else able to reap it.
    pub(crate) fn wait_within(
        &mut self,
        budget: std::time::Duration,
        command: &impl std::fmt::Debug,
    ) -> std::io::Result<Deadline> {
        match self.child.wait_timeout(budget) {
            Ok(Some(status)) => Ok(Deadline::Exited(status)),
            Ok(None) => {
                tracing::warn!(
                    command = ?command,
                    budget_seconds = budget.as_secs(),
                    "command exceeded its timeout; killing its process group"
                );
                self.kill_tree();
                Ok(Deadline::Expired)
            }
            Err(error) => {
                self.kill_tree();
                Err(error)
            }
        }
    }
}

/// Runs on every platform deliberately. The Unix arm of the tree kill was well covered and the
/// Windows arm by nothing at all, which is the entire reason `kill_process_tree` shipped there as
/// a direct-child kill wearing a tree-kill name. Anything gated on `unix` here re-opens that gap.
/// ~keep
#[cfg(test)]
mod tests {
    use super::{Deadline, GroupChild};
    use std::time::Duration;

    const SETTLE_POLL: Duration = Duration::from_millis(20);
    const SETTLE_LIMIT: Duration = Duration::from_secs(10);

    /// How long the probe stays alive once it has announced its grandchild. Only reached when a
    /// kill failed to arrive, so it is a diagnosis window rather than a wait anything depends on.
    /// ~keep
    const PROBE_LIFETIME: Duration = Duration::from_secs(60);

    /// Names the file the probe announces its grandchild's pid in. Its presence is also what tells
    /// the probe it is running as a probe rather than as an ordinary ignored test. ~keep
    const GRANDCHILD_PROBE_MARKER: &str = "ALEF_PROCESS_GRANDCHILD_PROBE";
    const GRANDCHILD_PROBE_NAME: &str = "process::timed::tests::grandchild_probe_child";

    /// A command that outlives every deadline in this module. Windows has no `sleep`, and
    /// `timeout /t` refuses to run without a console, so a loopback `ping` is the console-free
    /// stand-in there -- the same substitution `snippets::session`'s hook tests already make. ~keep
    fn long_lived_command() -> std::process::Command {
        let mut command = if cfg!(windows) {
            let mut ping = std::process::Command::new("ping");
            ping.args(["-n", "61", "127.0.0.1"]);
            ping
        } else {
            let mut sleep = std::process::Command::new("sleep");
            sleep.arg("60");
            sleep
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    /// A shell command that exits immediately with `code`.
    fn exiting_command(code: i32) -> std::process::Command {
        let (shell, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
        let mut command = std::process::Command::new(shell);
        command.args([flag, &format!("exit {code}")]);
        command
    }

    #[cfg(unix)]
    fn is_alive(pid: u32) -> bool {
        // SAFETY: signal 0 performs error checking only and sends nothing.
        unsafe { libc::kill(pid.cast_signed(), 0) == 0 }
    }

    /// Windows has no `kill(pid, 0)`. A process that is fully gone can no longer be opened at all,
    /// and one that has exited while a handle to it survives reports its real exit code, so
    /// `STILL_ACTIVE` is the only answer that means "running". ~keep
    #[cfg(windows)]
    fn is_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // SAFETY: opening by pid; failure returns a null handle, checked before any use.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0_u32;
        // SAFETY: `handle` is open and `exit_code` is a live, writable `u32`.
        let read = unsafe { GetExitCodeProcess(handle, &raw mut exit_code) };
        // SAFETY: the handle was opened here and is closed exactly once.
        unsafe {
            CloseHandle(handle);
        }
        read != 0 && exit_code == STILL_ACTIVE.cast_unsigned()
    }

    #[cfg(unix)]
    fn terminate(pid: u32) {
        // SAFETY: signalling one pid this test's own probe created.
        unsafe {
            libc::kill(pid.cast_signed(), libc::SIGKILL);
        }
    }

    #[cfg(windows)]
    fn terminate(pid: u32) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

        // SAFETY: opening by pid; failure returns a null handle, checked before any use.
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return;
        }
        // SAFETY: `handle` was opened with `PROCESS_TERMINATE` and is closed exactly once.
        unsafe {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }

    fn wait_until_gone(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + SETTLE_LIMIT;
        while std::time::Instant::now() < deadline {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(SETTLE_POLL);
        }
        !is_alive(pid)
    }

    /// Runs this test binary against [`grandchild_probe_child`], pointing it at `marker`.
    fn probe_command(marker: &std::path::Path) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().expect("the test binary"));
        command
            .args(["--exact", GRANDCHILD_PROBE_NAME, "--ignored", "--test-threads=1"])
            .env(GRANDCHILD_PROBE_MARKER, marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    /// Blocks until the probe has announced a pid that names a live process.
    fn announced_grandchild(marker: &std::path::Path) -> u32 {
        let deadline = std::time::Instant::now() + SETTLE_LIMIT;
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "the probe never announced a grandchild"
            );
            if let Ok(contents) = std::fs::read_to_string(marker)
                && let Ok(pid) = contents.trim().parse::<u32>()
                && is_alive(pid)
            {
                return pid;
            }
            std::thread::sleep(SETTLE_POLL);
        }
    }

    /// Not a test: the middle process of the two tests below. Starts a grandchild that outlives
    /// every deadline they set, announces its pid, and then waits to be killed, so those tests can
    /// ask what the kill actually reached rather than whether a kill branch was entered. The
    /// grandchild is a real spawn rather than a backgrounded shell job because `Child::id` is the
    /// one way to learn its pid that reads the same on both platforms -- `$!` has no `cmd`
    /// equivalent. Inert unless the environment names a marker file, so an ordinary `--ignored`
    /// run does nothing. ~keep
    #[test]
    #[ignore = "spawned as a subprocess by the process-tree kill tests"]
    #[expect(
        clippy::zombie_processes,
        reason = "the grandchild is meant to outlive this process; waiting on it is what the tree kill has to make unnecessary"
    )]
    fn grandchild_probe_child() {
        let Ok(marker) = std::env::var(GRANDCHILD_PROBE_MARKER) else {
            return;
        };
        let grandchild = long_lived_command().spawn().expect("spawn the grandchild");
        std::fs::write(&marker, grandchild.id().to_string()).expect("announce the grandchild");
        std::thread::sleep(PROBE_LIFETIME);
    }

    /// The defect this module exists to close, and the one Windows went on carrying: `Child::kill`
    /// ends the direct child alone, so the grandchild it started outlives the deadline -- a Gradle
    /// daemon in the incident that prompted the Unix fix, a `ping` holding alef's output pipes open
    /// past the timeout on Windows. Asserting that the kill branch was entered would prove nothing:
    /// the orphaned tree was produced by code that did enter its kill branch. ~keep
    #[test]
    fn an_expired_deadline_kills_the_grandchild_too() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("grandchild.pid");
        let mut child = GroupChild::spawn(&mut probe_command(&marker)).expect("spawn the process group");
        let grandchild = announced_grandchild(&marker);

        let outcome = child
            .wait_within(Duration::from_millis(200), &GRANDCHILD_PROBE_NAME)
            .expect("waiting on a live child is not an error");

        assert!(matches!(outcome, Deadline::Expired));
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived its parent's deadline"
        );
    }

    /// The sabotage check for the test above: the same probe, killed the way `Child::kill` kills,
    /// must leave its grandchild running. Without it that test stays green on any platform whose
    /// tree kill has quietly degraded to a direct-child kill, so long as the grandchild happens to
    /// die on its own -- which is the exact shape the Windows arm was in. ~keep
    #[test]
    fn killing_the_direct_child_alone_leaves_the_grandchild_running() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("grandchild.pid");
        let mut child = probe_command(&marker).spawn().expect("spawn the probe");
        let grandchild = announced_grandchild(&marker);

        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(SETTLE_POLL);

        let survived = is_alive(grandchild);
        terminate(grandchild);
        assert!(
            survived,
            "grandchild {grandchild} died without a tree kill, so the test above proves nothing"
        );
    }

    /// The second sabotage check: were the wait to report expiry for a child that had exited on
    /// its own, or the kill to run unconditionally, the tests above would pass for the wrong
    /// reason. The exact status is asserted because a bounded wait that loses the child's exit
    /// code turns every timed command into a silent success. ~keep
    #[test]
    fn a_child_that_beats_its_deadline_reports_its_own_exit_status() {
        let mut child = GroupChild::spawn(&mut exiting_command(7)).expect("spawn the process group");

        let outcome = child
            .wait_within(Duration::from_secs(30), &"exit 7")
            .expect("waiting on a live child is not an error");

        match outcome {
            Deadline::Exited(status) => assert_eq!(status.code(), Some(7)),
            Deadline::Expired => panic!("a command that exits immediately must not report expiry"),
        }
    }
}
