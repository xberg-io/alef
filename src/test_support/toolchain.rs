//! Availability gate and execution census for the fixtures that shell out to a real language
//! toolchain.
//!
//! A fixture that compiles alef's generated Go with the real `go` command has exactly two honest
//! outcomes when `go` is not installed: fail, or report that it did not run. What it must never
//! do is report the third thing -- a pass -- because a pass that examined nothing is
//! indistinguishable from a pass that examined everything, and CI reads only the counts.
//!
//! Both of the obvious designs are wrong on their own. A hard panic (what the Go fixtures did
//! before this module existed) turns every runner without the toolchain permanently red, so
//! nobody reads that leg and real regressions hide inside the expected failures. A silent skip
//! turns it permanently green while testing less than it claims. [`ToolchainGate::open`] does
//! the third thing: it skips, and it *counts*.
//!
//! Every call is tallied per toolchain as attempted, and then as either executed or skipped, and
//! the running tally is flushed to `<target-dir>/toolchain-census/<test-binary>.tsv` after each
//! call. Nothing in this process reads those files back. That is deliberate: `libtest` gives a
//! test binary no end-of-run hook to report from, it captures the stdout *and* stderr of a
//! passing test (so an `eprintln!` skip notice is invisible in precisely the run that needs it),
//! and `cargo test` splits the suite across several binaries anyway. `scripts/toolchain-census.sh`
//! reads the files after the run, prints the per-toolchain counts, and fails when a toolchain the
//! caller declared required executed zero fixtures -- which is what keeps "12 of 12 go fixtures
//! executed" and "0 of 12 executed, go absent" distinguishable in CI output without reading any
//! code. ~keep

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use super::spawn_from_stable_dir;

/// Directory, relative to the cargo target directory, holding one tally file per test binary.
/// `scripts/toolchain-census.sh` reads this same name -- change both together. ~keep
const CENSUS_DIR_NAME: &str = "toolchain-census";

/// One external language toolchain a fixture cannot run without.
///
/// Constructed only as the `const`s below, so the census can never grow a toolchain whose name
/// no `scripts/toolchain-census.sh` invocation knows about.
pub(crate) struct ToolchainGate {
    /// Census key, and the name a census failure reports.
    name: &'static str,
    /// Binary looked up on `PATH`.
    binary: &'static str,
    /// Argument that makes the binary prove it can actually run. See [`ToolchainGate::resolve`].
    version_arg: &'static str,
    /// Environment variable that upgrades "absent" from a counted skip to a hard failure. CI sets
    /// it on every platform that installs the toolchain.
    require_env: &'static str,
}

/// The Go toolchain, needed by every fixture that compiles or runs alef's generated Go.
///
/// GitHub's Linux and Windows runner images preinstall Go; the arm64 macOS images do not, which
/// is how the `Test (macos-latest)` leg spent from 2026-08-31 onward permanently red on nine Go
/// fixtures. CI installs Go explicitly on all three platforms now, so `ALEF_REQUIRE_GO` is set
/// everywhere and this gate never skips there. ~keep
pub(crate) const GO: ToolchainGate = ToolchainGate {
    name: "go",
    binary: "go",
    version_arg: "version",
    require_env: "ALEF_REQUIRE_GO",
};

/// The Swift toolchain, needed by the SwiftPM compile gate for the trait-box generator.
///
/// Unlike Go this genuinely cannot be installed on every platform: Swift ships with Xcode on
/// macOS, and the Windows toolchain is a multi-gigabyte installer with no first-party setup
/// action. So the Windows and Linux legs skip it and say so, and the census enforces that the
/// macOS leg -- the one platform where the compile actually happens -- executed it.
pub(crate) const SWIFT: ToolchainGate = ToolchainGate {
    name: "swift",
    binary: "swift",
    version_arg: "--version",
    require_env: "ALEF_REQUIRE_SWIFT",
};

/// Per-toolchain attempt/execution counts for this test binary.
#[derive(Clone, Copy, Default)]
struct Tally {
    attempted: u32,
    executed: u32,
    skipped: u32,
}

/// The census, and the resolution cache, for this process.
///
/// One mutex covers both so a tally and the flush that publishes it cannot interleave with
/// another test thread's -- the file on disk is rewritten whole on every update, never appended
/// to, so a concurrent writer could otherwise publish a tally that skips a count.
static CENSUS: Mutex<Census> = Mutex::new(Census {
    tallies: BTreeMap::new(),
    resolved: BTreeMap::new(),
});

struct Census {
    tallies: BTreeMap<&'static str, Tally>,
    resolved: BTreeMap<&'static str, Option<PathBuf>>,
}

impl ToolchainGate {
    /// This gate's census name, for a fixture that wants to name the toolchain it skipped.
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// Resolve this gate's toolchain for one fixture invocation, recording the attempt.
    ///
    /// Returns `None` when the toolchain is absent and its `ALEF_REQUIRE_*` variable is unset:
    /// the caller must then return without asserting anything, because it has verified nothing.
    /// The skip is already counted by the time this returns, so a caller cannot forget to report
    /// it. Panics instead when the variable *is* set, which is how a runner whose toolchain setup
    /// silently regressed fails loudly rather than quietly testing less.
    pub(crate) fn open(&self) -> Option<PathBuf> {
        let resolved = self.resolve();
        self.record(resolved.is_some());
        self.require_available(resolved, std::env::var_os(self.require_env).is_some())
    }

    /// Turn an absent toolchain into a panic when `required`, and pass it through otherwise.
    ///
    /// `required` is a plain parameter, and `resolved` an already-performed lookup, rather than
    /// this reading the environment or `PATH` itself, so
    /// `required_mode_fails_when_the_toolchain_is_unavailable` below can prove the panic fires
    /// against a fabricated "unavailable" input instead of having to hide a real binary from
    /// `PATH` -- which no test in a shared process can do without racing every other test. ~keep
    fn require_available(&self, resolved: Option<PathBuf>, required: bool) -> Option<PathBuf> {
        assert!(
            resolved.is_some() || !required,
            "{} is set but `{}` is unavailable: this fixture compiles alef's generated output \
             with the real {} toolchain and verifies nothing without it",
            self.require_env,
            self.binary,
            self.name
        );
        resolved
    }

    /// Look the binary up on `PATH` *and* prove it runs, caching the answer for the process.
    ///
    /// Resolution alone is not enough: a version-manager shim (`asdf`, `g`, `rbenv`-style) sits
    /// on `PATH` and spawns fine with no toolchain installed behind it, then exits non-zero. A
    /// `which`-only check would count that as executed and then fail the fixture's real
    /// assertions on every such machine. ~keep
    fn resolve(&self) -> Option<PathBuf> {
        let mut census = lock();
        if let Some(cached) = census.resolved.get(self.name) {
            return cached.clone();
        }
        let resolved = which::which(self.binary).ok().filter(|_| self.is_runnable());
        census.resolved.insert(self.name, resolved.clone());
        resolved
    }

    fn is_runnable(&self) -> bool {
        spawn_from_stable_dir(self.binary)
            .arg(self.version_arg)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn record(&self, executed: bool) {
        let mut census = lock();
        let tally = census.tallies.entry(self.name).or_default();
        tally.attempted += 1;
        if executed {
            tally.executed += 1;
        } else {
            tally.skipped += 1;
        }
        let snapshot = census.tallies.clone();
        drop(census);
        flush(&snapshot);
    }
}

/// A poisoned census is still usable: a fixture that panicked while holding this lock left only
/// counters behind, and cascading that panic into every other toolchain fixture would replace one
/// legible failure with dozens.
fn lock() -> std::sync::MutexGuard<'static, Census> {
    CENSUS.lock().unwrap_or_else(|error| error.into_inner())
}

/// Rewrite this test binary's tally file. Best-effort: a fixture must not fail because the census
/// could not be written, and `scripts/toolchain-census.sh` reports a missing tally as zero
/// executed anyway, which fails the run for the right reason.
fn flush(tallies: &BTreeMap<&'static str, Tally>) {
    let Some(path) = census_file() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let mut rendered = String::new();
    for (name, tally) in tallies {
        let _ = writeln!(
            rendered,
            "{name}\t{}\t{}\t{}",
            tally.attempted, tally.executed, tally.skipped
        );
    }
    let _ = std::fs::write(&path, rendered);
}

/// `<target-dir>/toolchain-census/<test-binary>.tsv`.
///
/// Derived from the running test binary rather than `CARGO_MANIFEST_DIR/target`, so a custom
/// `CARGO_TARGET_DIR` or a `--release` run lands beside the binaries that produced it. Test
/// executables live at `<target-dir>/<profile>/deps/<name>-<hash>`, so the target directory is
/// three levels up.
fn census_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_str()?.to_owned();
    let target_dir = exe.parent()?.parent()?.parent()?;
    Some(target_dir.join(CENSUS_DIR_NAME).join(format!("{name}.tsv")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The census file has to land inside the cargo target directory, next to the binaries whose
    /// tallies it holds, or `scripts/toolchain-census.sh` reads an empty directory and reports a
    /// clean run as "no fixtures attempted". ~keep
    #[test]
    fn census_file_lands_under_the_cargo_target_directory() {
        let path = census_file().expect("census path resolves for a cargo-built test binary");
        let parent = path.parent().expect("census file has a parent directory");

        assert_eq!(
            parent.file_name().and_then(|name| name.to_str()),
            Some(CENSUS_DIR_NAME),
            "census file must sit in the {CENSUS_DIR_NAME} directory, got {path:?}"
        );
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("tsv"),
            "census file must be a TSV the census script can sum, got {path:?}"
        );
    }

    /// The whole point of the gate: a fixture invocation is counted before the caller can decide
    /// what to do about it, so a skip can never go unreported.
    #[test]
    fn opening_a_gate_records_exactly_one_attempt() {
        let before = tally_of(GO.name());

        let resolved = GO.open();

        let after = tally_of(GO.name());
        assert_eq!(
            after.attempted,
            before.attempted + 1,
            "opening the go gate must record exactly one attempt"
        );
        assert_eq!(
            (after.executed - before.executed, after.skipped - before.skipped),
            if resolved.is_some() { (1, 0) } else { (0, 1) },
            "an attempt must land in exactly one of the executed/skipped columns"
        );
        assert_eq!(
            after.attempted,
            after.executed + after.skipped,
            "attempted must always equal executed + skipped, or the census cannot be read as a ratio"
        );
    }

    /// `scripts/toolchain-census.sh` fails a required toolchain on `executed == 0`, so the on-disk
    /// row must carry the executed count, not just the attempt count. Reads the real file back
    /// rather than the in-memory tally so a broken flush is caught here and not in CI. ~keep
    #[test]
    fn the_flushed_row_reports_attempted_executed_and_skipped() {
        let _ = GO.open();
        let path = census_file().expect("census path resolves");

        let rendered = std::fs::read_to_string(&path).expect("census file was flushed to disk");

        let row = rendered
            .lines()
            .find(|line| line.starts_with(&format!("{}\t", GO.name())))
            .unwrap_or_else(|| panic!("no `{}` row in flushed census:\n{rendered}", GO.name()));
        let columns: Vec<&str> = row.split('\t').collect();
        assert_eq!(
            columns.len(),
            4,
            "a census row is <toolchain>\\t<attempted>\\t<executed>\\t<skipped>, got {row:?}"
        );
        let attempted: u32 = columns[1].parse().expect("attempted column is a number");
        let executed: u32 = columns[2].parse().expect("executed column is a number");
        let skipped: u32 = columns[3].parse().expect("skipped column is a number");
        assert!(
            attempted > 0,
            "the row must record the attempt that just happened: {row:?}"
        );
        assert_eq!(attempted, executed + skipped, "flushed row does not add up: {row:?}");
    }

    /// The hard half of the contract: on a platform CI installs the toolchain for, a missing
    /// toolchain must fail the run rather than be counted as a skip. Without this the census
    /// alone would let a regressed runner setup pass, since a skip is a legitimate outcome
    /// everywhere else. ~keep
    #[test]
    fn required_mode_fails_when_the_toolchain_is_unavailable() {
        let result = std::panic::catch_unwind(|| GO.require_available(None, true));

        let panic = result.expect_err("required mode must fail when the toolchain is unavailable");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic");
        assert!(
            message.contains("ALEF_REQUIRE_GO is set"),
            "the panic must name the variable that made the toolchain required, got: {message}"
        );
    }

    /// The other half: absent and *not* required is a skip, not a failure -- otherwise a
    /// contributor without Go installed cannot run the suite at all, which is how the Go fixtures
    /// came to be permanently red on two of the three CI legs. ~keep
    #[test]
    fn unrequired_mode_reports_an_absent_toolchain_as_a_skip() {
        assert_eq!(
            GO.require_available(None, false),
            None,
            "an absent, unrequired toolchain must be reported as not-run rather than panicking"
        );
    }

    fn tally_of(name: &'static str) -> Tally {
        lock().tallies.get(name).copied().unwrap_or_default()
    }
}
