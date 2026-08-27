//! Subprocess-tree lifecycle for every alef path that runs a child under a deadline.
//!
//! Two facts drive everything here, and they were learned twice -- once by the snippet validators,
//! then again by the `setup`/`build` pipeline, because the second path carried its own copy of the
//! naive shape. A `sh -c` child is a *tree*: `Child::kill` signals the shell alone, so the
//! `gradlew` it started, and the Gradle daemon under that, survive their own deadline and reparent
//! to PID 1. Killing the tree means killing a process group, which means spawning into one. And a
//! child in its own process group is no longer in the terminal's foreground group, so it never
//! receives the `SIGINT` Ctrl-C delivers -- alef has to forward that itself.
//!
//! The two halves are inseparable: [`configure_process_group`] without [`termination::track`]
//! trades an orphaned tree on timeout for an orphaned tree on Ctrl-C. Call them together. ~keep

pub(crate) mod capture;
#[cfg(windows)]
pub(crate) mod job;
pub(crate) mod termination;
pub(crate) mod timed;

/// Puts `command`'s child in a new process group of its own, so its whole tree can be addressed
/// by one signal.
///
/// Only meaningful alongside [`termination::track`] -- see the module docs.
#[cfg(unix)]
pub(crate) fn configure_process_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows has nothing to configure before the spawn. Its tree is addressed by a job object,
/// which can only be joined by a process that already exists, so the whole Windows half of this
/// happens in [`termination::track`] instead -- and a Windows child stays in alef's own console
/// group, so it receives Ctrl-C by delivery and needs no forwarding. ~keep
#[cfg(not(unix))]
pub(crate) fn configure_process_group(_command: &mut std::process::Command) {}

/// Kills `child` and every descendant it started.
///
/// `tracked` is the registry slot [`termination::track`] handed back for this same child. It is
/// not decoration on Windows: the job object that makes the kill tree-wide lives in it, and
/// passing the wrong one -- or none -- silently degrades this to killing the direct child.
///
/// Falls back to signalling the child alone when the tree kill fails, which is the best that can
/// be done for a child that was not spawned through [`configure_process_group`] and
/// [`termination::track`].
#[cfg(unix)]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, _tracked: &termination::TrackedProcessGroup) {
    let process_group = format!("-{}", child.id());
    let killed_group = std::process::Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status()
        .is_ok_and(|status| status.success());
    if !killed_group {
        let _ = child.kill();
    }
}

#[cfg(windows)]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, tracked: &termination::TrackedProcessGroup) {
    if !tracked.terminate_tree() {
        let _ = child.kill();
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, _tracked: &termination::TrackedProcessGroup) {
    let _ = child.kill();
}

/// The first gap between `try_wait` polls. A fixed 50ms interval charged every subprocess about
/// 25ms of pure sleep on average — invisible for one `cargo check`, tens of seconds across a run
/// with thousands of snippets, because most snippet toolchain invocations finish in single-digit
/// milliseconds. ~keep
pub(crate) const INITIAL_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// The ceiling the backoff grows to, so a genuinely long compile still costs at most one wakeup
/// per 50ms rather than a thousand. ~keep
const MAX_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Doubles a poll interval up to [`MAX_WAIT_POLL_INTERVAL`].
pub(crate) fn next_poll_interval(current: std::time::Duration) -> std::time::Duration {
    current
        .checked_mul(2)
        .unwrap_or(MAX_WAIT_POLL_INTERVAL)
        .min(MAX_WAIT_POLL_INTERVAL)
}

pub(crate) trait WaitTimeout {
    /// Waits up to `timeout` for the child to exit, returning `Ok(None)` when it is still running.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be waited on.
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        let mut poll_interval = INITIAL_WAIT_POLL_INTERVAL;

        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Ok(None);
            }

            std::thread::sleep(poll_interval.min(timeout - elapsed));
            poll_interval = next_poll_interval(poll_interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    /// The backoff has to start far below the old fixed 50ms floor and still stop growing, so a
    /// short command returns almost immediately while a long compile is not polled a thousand
    /// times a second. ~keep
    ///
    /// ~keep This is the whole coverage for that property, deliberately. A companion test used to
    /// time 20 trivial `sh -c 'exit 0'` runs and assert the amortised cost stayed under the old
    /// fixed interval, but bare process-spawn overhead on a loaded machine reaches 60ms/command --
    /// more than the 50ms bound it was trying to prove we no longer pay -- so it failed on load
    /// rather than on regression, at two successive thresholds. Asserting the schedule directly
    /// proves the same thing and cannot be perturbed by what else the machine is doing. Do not
    /// re-add a wall-clock version.
    #[test]
    fn the_wait_backoff_starts_at_one_millisecond_and_caps_at_fifty() {
        assert_eq!(super::INITIAL_WAIT_POLL_INTERVAL, Duration::from_millis(1));

        let intervals = std::iter::successors(Some(super::INITIAL_WAIT_POLL_INTERVAL), |current| {
            Some(super::next_poll_interval(*current))
        })
        .take(8)
        .collect::<Vec<_>>();

        assert_eq!(
            intervals,
            vec![
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(4),
                Duration::from_millis(8),
                Duration::from_millis(16),
                Duration::from_millis(32),
                Duration::from_millis(50),
                Duration::from_millis(50),
            ]
        );
    }
}
