//! Termination propagation for the subprocess trees alef spawns into their own process groups.
//!
//! Every child spawned through [`super::configure_process_group`] gets a process group of its
//! own, so a timeout can tear the whole tree down with one `kill(-pgid)`. That detachment costs
//! the tree the terminal's own signals:
//! `SIGINT` from Ctrl-C reaches the foreground process group only -- alef's -- so a child in its
//! own group never sees it, keeps running after alef exits 130, and reparents to PID 1 along with
//! every descendant it started. Having taken the children out of the terminal's reach, alef owns
//! forwarding termination to them, which is what this module does. ~keep

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

/// The empty-slot marker. Zero is safe as a sentinel because no process group id is ever 0 from a
/// child's point of view: `process_group(0)` makes the child its own group leader, so the id
/// recorded here is always its own (non-zero) pid. ~keep
#[cfg(unix)]
const UNUSED_SLOT: i32 = 0;

/// How many live child process groups can be tracked at once. A run validates snippets across at
/// most a few dozen sessions in parallel and drives one pipeline command per language beside them,
/// so this is far above the working set; a spawn that finds the table full is still killed by the
/// timeout path, it just loses signal forwarding. The table
/// is fixed-size and lock-free because the signal handler that reads it may only call
/// async-signal-safe functions -- a `Vec` behind a `Mutex` would be neither. ~keep
#[cfg(unix)]
const MAX_TRACKED_PROCESS_GROUPS: usize = 512;

#[cfg(unix)]
static TRACKED_PROCESS_GROUPS: [AtomicI32; MAX_TRACKED_PROCESS_GROUPS] =
    [const { AtomicI32::new(UNUSED_SLOT) }; MAX_TRACKED_PROCESS_GROUPS];

/// The signals that mean "this run is over": Ctrl-C, a `kill` with no arguments, and a closed
/// terminal. Each is forwarded to the child groups and then re-raised, so alef still exits with
/// the conventional `128 + signo` rather than swallowing the signal. ~keep
#[cfg(unix)]
const FORWARDED_SIGNALS: &[libc::c_int] = &[libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// Holds one registry slot for as long as the child process group it names may still be alive.
///
/// On Windows there is no registry and no signal to forward -- a child stays in alef's console
/// group and receives Ctrl-C by delivery -- so what this holds instead is the job object that
/// makes [`super::kill_process_tree`] reach the whole tree. Either way the value has to outlive
/// the child: dropping it early costs the tree its kill. ~keep
pub(crate) struct TrackedProcessGroup {
    #[cfg(unix)]
    slot: Option<usize>,
    #[cfg(windows)]
    job: Option<crate::process::job::JobObject>,
}

#[cfg(windows)]
impl TrackedProcessGroup {
    /// Terminates the tracked child and every process it created, reporting whether the job
    /// accepted the request. `false` when no job could be created at spawn time, which is the
    /// caller's cue to fall back to killing the direct child.
    pub(crate) fn terminate_tree(&self) -> bool {
        self.job.as_ref().is_some_and(crate::process::job::JobObject::terminate)
    }
}

#[cfg(unix)]
impl Drop for TrackedProcessGroup {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            TRACKED_PROCESS_GROUPS[slot].store(UNUSED_SLOT, Ordering::Release);
        }
    }
}

/// Registers `child`'s process group for signal forwarding and installs the forwarding handlers
/// on first use.
#[cfg(unix)]
pub(crate) fn track(child: &std::process::Child) -> TrackedProcessGroup {
    install_termination_forwarding();
    TrackedProcessGroup {
        slot: claim_slot(&TRACKED_PROCESS_GROUPS, child.id().cast_signed()),
    }
}

/// Puts `child` in a job object of its own, so every process it goes on to create can be killed
/// with it. A child that cannot be assigned -- one that already exited, most often -- is tracked
/// without a job and falls back to a direct-child kill.
#[cfg(windows)]
pub(crate) fn track(child: &std::process::Child) -> TrackedProcessGroup {
    TrackedProcessGroup {
        job: crate::process::job::JobObject::holding(child),
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn track(_child: &std::process::Child) -> TrackedProcessGroup {
    TrackedProcessGroup {}
}

/// Records `process_group` in the first free slot of `slots`, or `None` when every slot is taken.
///
/// `slots` is a parameter rather than a read of [`TRACKED_PROCESS_GROUPS`] so the kill path can be
/// exercised against a table only one test owns: a test that swept the process-wide table would
/// kill the children of every other test running beside it. ~keep
#[cfg(unix)]
fn claim_slot(slots: &[AtomicI32], process_group: i32) -> Option<usize> {
    slots.iter().position(|slot| {
        slot.compare_exchange(UNUSED_SLOT, process_group, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    })
}

/// Sends `SIGKILL` to every process group recorded in `slots`.
///
/// Called from a signal handler, so every call it makes must be async-signal-safe: relaxed atomic
/// loads and `kill(2)` are, and nothing here allocates, locks, or formats. ~keep
#[cfg(unix)]
fn kill_tracked_process_groups(slots: &[AtomicI32]) {
    for slot in slots {
        let process_group = slot.load(Ordering::Acquire);
        if process_group != UNUSED_SLOT {
            // SAFETY: `kill` is async-signal-safe and a negative pid addresses a process group.
            // A group id is not reused while the group has members, so this cannot reach an
            // unrelated tree.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
extern "C" fn forward_termination(signal: libc::c_int) {
    kill_tracked_process_groups(&TRACKED_PROCESS_GROUPS);
    // SAFETY: both calls are async-signal-safe. Restoring the default disposition and re-raising
    // is what gives alef the conventional `128 + signo` exit status instead of returning from the
    // handler as if nothing had happened.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

#[cfg(unix)]
fn install_termination_forwarding() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        for signal in FORWARDED_SIGNALS {
            // SAFETY: `forward_termination` is a plain `extern "C" fn` with the handler signature.
            unsafe {
                let previous = libc::signal(*signal, forward_termination as *const () as libc::sighandler_t);
                // A process launched with a signal already ignored -- `nohup`, a daemon
                // supervisor, a CI shell -- must keep ignoring it. Installing a handler over
                // `SIG_IGN` would resurrect a signal the parent deliberately disarmed. ~keep
                if previous == libc::SIG_IGN {
                    libc::signal(*signal, libc::SIG_IGN);
                }
            }
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::{TRACKED_PROCESS_GROUPS, UNUSED_SLOT, claim_slot, kill_tracked_process_groups};
    use std::sync::atomic::{AtomicI32, Ordering};

    const SLOT_COUNT: usize = 4;
    const PROCESS_SETTLE_POLL: std::time::Duration = std::time::Duration::from_millis(20);
    const PROCESS_SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

    /// Names the file the probe below announces its child's process group in. Its presence is also
    /// what tells the probe it is running as a probe rather than as an ordinary ignored test.
    const ORPHAN_PROBE_MARKER: &str = "ALEF_TERMINATION_ORPHAN_PROBE";
    const ORPHAN_PROBE_NAME: &str = "process::termination::tests::orphan_probe_child";

    fn slots() -> Vec<AtomicI32> {
        (0..SLOT_COUNT).map(|_| AtomicI32::new(UNUSED_SLOT)).collect()
    }

    fn is_alive(pid: i32) -> bool {
        // SAFETY: signal 0 performs error checking only and sends nothing.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_until_gone(pid: i32) -> bool {
        let deadline = std::time::Instant::now() + PROCESS_SETTLE_LIMIT;
        while std::time::Instant::now() < deadline {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        }
        !is_alive(pid)
    }

    /// Spawns `sh -c 'sleep 60 & echo $! >file; sleep 60'` in its own process group and returns
    /// the shell's pid together with its grandchild's, once both are confirmed running.
    fn spawn_group_with_grandchild(directory: &std::path::Path) -> (std::process::Child, i32, i32) {
        use std::os::unix::process::CommandExt;

        let marker = directory.join("grandchild.pid");
        let mut command = std::process::Command::new("sh");
        command
            .args(["-c", &format!("sleep 60 & echo $! > {}; sleep 60", marker.display())])
            .process_group(0);
        let child = command.spawn().expect("spawn the process group");
        let parent = child.id().cast_signed();

        let deadline = std::time::Instant::now() + PROCESS_SETTLE_LIMIT;
        let grandchild = loop {
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild never announced itself"
            );
            if let Ok(contents) = std::fs::read_to_string(&marker)
                && let Ok(pid) = contents.trim().parse::<i32>()
                && is_alive(pid)
            {
                break pid;
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        };
        assert!(is_alive(parent), "the tracked shell must be running before the sweep");
        (child, parent, grandchild)
    }

    /// The property Ctrl-C used to violate: a tracked group's *whole tree*, grandchildren
    /// included, is gone after the sweep the signal handler runs. Asserting that the sweep was
    /// entered would prove nothing -- the orphaned `sh -> gradlew -> daemon` tree this fixes was
    /// produced by code that did enter its cleanup path. ~keep
    #[test]
    fn sweeping_a_tracked_group_kills_its_grandchildren_too() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let (mut child, parent, grandchild) = spawn_group_with_grandchild(directory.path());
        let table = slots();
        claim_slot(&table, parent).expect("a free slot");

        kill_tracked_process_groups(&table);

        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived the sweep"
        );
        let _ = child.wait();
        assert!(wait_until_gone(parent), "tracked shell {parent} survived the sweep");
    }

    /// The sabotage check for the test above: if the sweep killed everything in sight rather than
    /// what the table names, both tests would pass for the wrong reason. ~keep
    #[test]
    fn sweeping_leaves_an_untracked_group_running() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let (mut child, parent, grandchild) = spawn_group_with_grandchild(directory.path());
        let table = slots();

        kill_tracked_process_groups(&table);
        std::thread::sleep(PROCESS_SETTLE_POLL);

        assert!(is_alive(parent), "an untracked shell must not be swept");
        assert!(is_alive(grandchild), "an untracked grandchild must not be swept");
        // SAFETY: a negative pid addresses the process group this test created.
        unsafe {
            libc::kill(-parent, libc::SIGKILL);
        }
        let _ = child.wait();
    }

    #[test]
    fn a_released_slot_is_reused_and_no_longer_swept() {
        let table = slots();
        let slot = claim_slot(&table, 4242).expect("a free slot");
        table[slot].store(UNUSED_SLOT, Ordering::Release);

        assert_eq!(claim_slot(&table, 5353), Some(slot));
        assert_eq!(table[slot].load(Ordering::Acquire), 5353);
    }

    #[test]
    fn a_full_table_refuses_further_slots() {
        let table = slots();
        for index in 0..SLOT_COUNT {
            let pid = i32::try_from(index).expect("slot index fits an i32");
            assert_eq!(claim_slot(&table, 100 + pid), Some(index));
        }

        assert_eq!(claim_slot(&table, 999), None);
    }

    /// The wiring the sweep is useless without: the handler must actually own `SIGINT`, `SIGTERM`
    /// and `SIGHUP` by the time a child is running. Compared against the concrete handler address
    /// rather than merely "not `SIG_DFL`", so an unrelated handler installed by something else
    /// cannot satisfy it. ~keep
    #[test]
    fn termination_forwarding_owns_every_interactive_signal() {
        super::install_termination_forwarding();
        let expected = super::forward_termination as *const () as libc::sighandler_t;

        for signal in super::FORWARDED_SIGNALS {
            let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
            // SAFETY: a null `act` reads the current disposition without changing it.
            let read = unsafe { libc::sigaction(*signal, std::ptr::null(), &raw mut current) };

            assert_eq!(read, 0, "reading the disposition of signal {signal}");
            assert_eq!(
                current.sa_sigaction, expected,
                "signal {signal} must be forwarded to the tracked child groups"
            );
        }
    }

    /// Not a test: the child half of
    /// [`a_signalled_alef_does_not_orphan_its_child_tree`]. Runs a `run_command` whose child
    /// announces its own process group and then outlives any signal the parent will send, so the
    /// parent can signal *this* process and inspect what is left behind. Inert unless the
    /// environment names a marker file, so an ordinary `--ignored` run does nothing. ~keep
    #[test]
    #[ignore = "spawned as a subprocess by a_signalled_alef_does_not_orphan_its_child_tree"]
    fn orphan_probe_child() {
        let Ok(marker) = std::env::var(ORPHAN_PROBE_MARKER) else {
            return;
        };
        let mut command = std::process::Command::new("sh");
        command.args(["-c", &format!("echo $$ > {marker}; sleep 30")]);
        let _ = crate::snippets::validators::run_command(&mut command, 30);
    }

    /// The end-to-end measurement of the orphaning half of this module's reason to exist.
    ///
    /// A real alef process is spawned, given a child that outlives everything, and then sent
    /// `SIGINT` -- the same disposition Ctrl-C produces. Because `run_command` puts that child in
    /// its own process group, the signal cannot reach it by delivery; the only thing that can kill
    /// it is alef forwarding the signal on. Asserting on the child's continued existence after the
    /// parent is gone is the whole point: with forwarding removed this passes only if the tree
    /// really is orphaned. ~keep
    #[test]
    fn a_signalled_alef_does_not_orphan_its_child_tree() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("group.pid");
        let mut probe = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", ORPHAN_PROBE_NAME, "--ignored", "--test-threads=1"])
            .env(ORPHAN_PROBE_MARKER, &marker)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn the probe");

        let deadline = std::time::Instant::now() + PROCESS_SETTLE_LIMIT;
        let orphan = loop {
            assert!(
                std::time::Instant::now() < deadline,
                "the probe never announced a child"
            );
            if let Ok(contents) = std::fs::read_to_string(&marker)
                && let Ok(pid) = contents.trim().parse::<i32>()
                && is_alive(pid)
            {
                break pid;
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        };

        // SAFETY: signalling one pid this test spawned and owns.
        let signalled = unsafe { libc::kill(probe.id().cast_signed(), libc::SIGINT) };
        assert_eq!(signalled, 0, "signalling the probe");
        probe.wait().expect("the probe exits");

        let survived = !wait_until_gone(orphan);
        if survived {
            // SAFETY: a negative pid addresses the process group the probe created.
            unsafe {
                libc::kill(-orphan, libc::SIGKILL);
            }
        }
        assert!(
            !survived,
            "child group {orphan} outlived the alef process that spawned it -- the tree was orphaned"
        );
    }

    /// `snippets::validators::run_command` must put its child in the process-wide table, not just
    /// in whatever table a test hands the sweep. Without this the two halves could drift apart and every unit above
    /// would still pass. The child announces its own pid, and that exact value is looked for --
    /// "some slot is occupied" would be satisfied by any other test's child running beside this
    /// one. ~keep
    #[test]
    fn run_command_registers_its_child_in_the_process_wide_table() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let marker = directory.path().join("child.pid");
        let script = format!("echo $$ > {}; sleep 5", marker.display());
        let worker = std::thread::spawn(move || {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", &script]);
            let _ = crate::snippets::validators::run_command(&mut command, 2);
        });

        let deadline = std::time::Instant::now() + PROCESS_SETTLE_LIMIT;
        let mut tracked = None;
        while std::time::Instant::now() < deadline && tracked.is_none() {
            if let Ok(contents) = std::fs::read_to_string(&marker)
                && let Ok(pid) = contents.trim().parse::<i32>()
            {
                tracked = TRACKED_PROCESS_GROUPS
                    .iter()
                    .any(|slot| slot.load(Ordering::Acquire) == pid)
                    .then_some(pid);
            }
            std::thread::sleep(PROCESS_SETTLE_POLL);
        }
        worker.join().expect("the run_command worker");

        assert!(
            tracked.is_some(),
            "run_command must register its own child's process group while it runs"
        );
    }
}
