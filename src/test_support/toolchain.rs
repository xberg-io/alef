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
//! Every call is tallied per toolchain as attempted, and then as executed, absent, or unusable, and
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
use std::path::{Path, PathBuf};
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
    probe: CapabilityProbe,
    /// Environment variable that upgrades "absent" from a counted skip to a hard failure. CI sets
    /// it on every platform that installs the toolchain.
    require_env: &'static str,
}

#[derive(Clone, Copy)]
enum CapabilityProbe {
    VersionOnly,
    GoBuild,
    SwiftFoundation,
    RubyJson,
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
    probe: CapabilityProbe::GoBuild,
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
    probe: CapabilityProbe::SwiftFoundation,
    require_env: "ALEF_REQUIRE_SWIFT",
};

/// The Ruby interpreter, needed by the fixture that executes alef's generated magnus Data
/// variant classes under `sorbet-runtime`.
///
/// CI installs Ruby and the pinned `sorbet-runtime` gem on every leg of the test matrix, so
/// `ALEF_REQUIRE_RUBY` is set everywhere there and this gate never skips in CI. A developer
/// machine without Ruby skips, and the census reports the skip. ~keep
pub(crate) const RUBY: ToolchainGate = ToolchainGate {
    name: "ruby",
    binary: "ruby",
    version_arg: "--version",
    probe: CapabilityProbe::RubyJson,
    require_env: "ALEF_REQUIRE_RUBY",
};

/// The `wasm-bindgen` CLI, needed by the fixture that reads the JS glue wasm-bindgen emits for
/// alef's generated wasm crate.
///
/// The CLI is a separate artifact from the `wasm-bindgen` crate the generated `Cargo.toml`
/// depends on, and the two must agree exactly -- the CLI aborts on a schema-version mismatch
/// rather than degrading. The fixture therefore reads this binary's own version and pins the
/// crate to it, so whichever CLI a runner installed is the one the fixture builds against.
/// `rust-toolchain.toml` already carries `wasm32-unknown-unknown` on every leg; the CLI does
/// not ship with it, so CI installs it explicitly and sets `ALEF_REQUIRE_WASM_BINDGEN`. ~keep
pub(crate) const WASM_BINDGEN: ToolchainGate = ToolchainGate {
    name: "wasm-bindgen",
    binary: "wasm-bindgen",
    version_arg: "--version",
    probe: CapabilityProbe::VersionOnly,
    require_env: "ALEF_REQUIRE_WASM_BINDGEN",
};

/// Per-toolchain attempt/execution counts for this test binary.
#[derive(Clone, Copy, Default)]
struct Tally {
    attempted: u32,
    executed: u32,
    absent: u32,
    unusable: u32,
}

#[derive(Clone)]
enum Resolution {
    Available(PathBuf),
    Absent,
    Unusable(String),
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
    resolved: BTreeMap<&'static str, Resolution>,
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
        self.record(&resolved);
        self.require_available(resolved, std::env::var_os(self.require_env).is_some())
    }

    /// Turn an absent toolchain into a panic when `required`, and pass it through otherwise.
    ///
    /// `required` is a plain parameter, and `resolved` an already-performed lookup, rather than
    /// this reading the environment or `PATH` itself, so
    /// `required_mode_fails_when_the_toolchain_is_unavailable` below can prove the panic fires
    /// against a fabricated "unavailable" input instead of having to hide a real binary from
    /// `PATH` -- which no test in a shared process can do without racing every other test. ~keep
    fn require_available(&self, resolved: Resolution, required: bool) -> Option<PathBuf> {
        match resolved {
            Resolution::Available(path) => Some(path),
            Resolution::Absent if !required => None,
            Resolution::Unusable(_) if !required => None,
            Resolution::Absent => panic!(
                "{} is set but `{}` is absent: this fixture compiles alef's generated output \
                 with the real {} toolchain and verifies nothing without it",
                self.require_env, self.binary, self.name
            ),
            Resolution::Unusable(diagnostic) => panic!(
                "{} is set and `{}` is present but unusable for the {} fixture:\n{}",
                self.require_env, self.binary, self.name, diagnostic
            ),
        }
    }

    /// Look the binary up on `PATH` *and* prove it runs, caching the answer for the process.
    ///
    /// Resolution alone is not enough: a version-manager shim (`asdf`, `g`, `rbenv`-style) sits
    /// on `PATH` and spawns fine with no toolchain installed behind it, then exits non-zero. A
    /// `which`-only check would count that as executed and then fail the fixture's real
    /// assertions on every such machine. ~keep
    fn resolve(&self) -> Resolution {
        if let Some(cached) = lock().resolved.get(self.name) {
            return cached.clone();
        }
        let resolved = match which::which(self.binary) {
            Ok(path) => match self.probe(&path) {
                Ok(()) => Resolution::Available(path),
                Err(diagnostic) => Resolution::Unusable(diagnostic),
            },
            Err(_) => Resolution::Absent,
        };
        lock().resolved.entry(self.name).or_insert(resolved).clone()
    }

    fn probe(&self, binary: &Path) -> Result<(), String> {
        let version = spawn_from_stable_dir(binary)
            .arg(self.version_arg)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("failed to run `{}`: {error}", binary.display()))?;
        if !version.status.success() {
            return Err(command_diagnostic(binary, &version));
        }

        match self.probe {
            CapabilityProbe::VersionOnly => Ok(()),
            CapabilityProbe::RubyJson => {
                let output = spawn_from_stable_dir(binary)
                    .args(["-e", "require \"json\""])
                    .output()
                    .map_err(|error| format!("failed to run `{}`: {error}", binary.display()))?;
                output
                    .status
                    .success()
                    .then_some(())
                    .ok_or_else(|| command_diagnostic(binary, &output))
            }
            CapabilityProbe::GoBuild => probe_go_build(binary),
            CapabilityProbe::SwiftFoundation => probe_swift_foundation(binary),
        }
    }

    fn record(&self, resolution: &Resolution) {
        let mut census = lock();
        let tally = census.tallies.entry(self.name).or_default();
        tally.attempted += 1;
        match resolution {
            Resolution::Available(_) => tally.executed += 1,
            Resolution::Absent => tally.absent += 1,
            Resolution::Unusable(_) => tally.unusable += 1,
        }
        flush(&census.tallies);
    }
}

/// A poisoned census is still usable: a fixture that panicked while holding this lock left only
/// counters behind, and cascading that panic into every other toolchain fixture would replace one
/// legible failure with dozens.
fn lock() -> std::sync::MutexGuard<'static, Census> {
    CENSUS.lock().unwrap_or_else(|error| error.into_inner())
}

fn command_diagnostic(binary: &Path, output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "`{}` exited with {}\nstdout:\n{}\nstderr:\n{}",
        binary.display(),
        output.status,
        stdout.trim(),
        stderr.trim()
    )
}

fn probe_go_build(binary: &Path) -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| format!("failed to create Go probe directory: {error}"))?;
    let source = temp.path().join("main.go");
    let output_binary = temp.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source, "package main\nfunc main() {}\n")
        .map_err(|error| format!("failed to write Go probe source: {error}"))?;

    let output = spawn_from_stable_dir(binary)
        .args(["build", "-o"])
        .arg(&output_binary)
        .arg(&source)
        .current_dir(temp.path())
        .env("GOWORK", "off")
        .env("GO111MODULE", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("failed to run `{}`: {error}", binary.display()))?;
    output
        .status
        .success()
        .then_some(())
        .ok_or_else(|| command_diagnostic(binary, &output))
}

fn probe_swift_foundation(binary: &Path) -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| format!("failed to create Swift probe directory: {error}"))?;
    let source = temp.path().join("main.swift");
    let output_binary = temp.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source, "import Foundation\nlet data = Data()\nprint(data.count)\n")
        .map_err(|error| format!("failed to write Swift probe source: {error}"))?;
    let compiler = binary.with_file_name(if cfg!(windows) { "swiftc.exe" } else { "swiftc" });

    let output = spawn_from_stable_dir(&compiler)
        .arg(&source)
        .arg("-o")
        .arg(&output_binary)
        .current_dir(temp.path())
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("failed to run `{}`: {error}", compiler.display()))?;
    output
        .status
        .success()
        .then_some(())
        .ok_or_else(|| command_diagnostic(&compiler, &output))
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
            "{name}\t{}\t{}\t{}\t{}",
            tally.attempted, tally.executed, tally.absent, tally.unusable
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

    /// The tests below read the process-wide census before and after opening a gate, so two of
    /// them running on separate `cargo test` threads would see each other's attempt in their own
    /// delta. Serialize the ones that open a gate through this lock. ~keep
    static GATE_TESTS: Mutex<()> = Mutex::new(());

    fn serialize_gate_tests() -> std::sync::MutexGuard<'static, ()> {
        GATE_TESTS.lock().unwrap_or_else(|error| error.into_inner())
    }

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
        let _serialized = serialize_gate_tests();
        let before = tally_of(GO.name());

        let resolved = GO.open();

        let after = tally_of(GO.name());
        assert_eq!(
            after.attempted,
            before.attempted + 1,
            "opening the go gate must record exactly one attempt"
        );
        let executed = after.executed - before.executed;
        let absent = after.absent - before.absent;
        let unusable = after.unusable - before.unusable;
        assert_eq!(executed + absent + unusable, 1, "an attempt must land in one outcome");
        assert_eq!(
            executed,
            u32::from(resolved.is_some()),
            "only a usable toolchain executes"
        );
        assert_eq!(
            after.attempted,
            after.executed + after.absent + after.unusable,
            "attempted must equal the sum of every outcome"
        );
    }

    /// `scripts/toolchain-census.sh` fails a required toolchain unless `executed == attempted`, so
    /// the on-disk row must carry every outcome count. Reads the real file back rather than the
    /// in-memory tally so a broken flush is caught here and not in CI. ~keep
    #[test]
    fn the_flushed_row_reports_each_resolution_outcome() {
        let _serialized = serialize_gate_tests();
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
            5,
            "a census row is <toolchain>\\t<attempted>\\t<executed>\\t<absent>\\t<unusable>, got {row:?}"
        );
        let attempted: u32 = columns[1].parse().expect("attempted column is a number");
        let executed: u32 = columns[2].parse().expect("executed column is a number");
        let absent: u32 = columns[3].parse().expect("absent column is a number");
        let unusable: u32 = columns[4].parse().expect("unusable column is a number");
        assert!(
            attempted > 0,
            "the row must record the attempt that just happened: {row:?}"
        );
        assert_eq!(
            attempted,
            executed + absent + unusable,
            "flushed row does not add up: {row:?}"
        );
    }

    #[test]
    fn concurrent_records_flush_the_latest_complete_tally() {
        const CONCURRENT: ToolchainGate = ToolchainGate {
            name: "concurrent-record-test",
            binary: "unused",
            version_arg: "unused",
            probe: CapabilityProbe::VersionOnly,
            require_env: "ALEF_REQUIRE_UNUSED",
        };
        let before = tally_of(CONCURRENT.name());
        let threads: Vec<_> = (0..32)
            .map(|index| {
                std::thread::spawn(move || {
                    let resolution = if index % 2 == 0 {
                        Resolution::Available(PathBuf::from("unused"))
                    } else {
                        Resolution::Unusable("unused".to_owned())
                    };
                    CONCURRENT.record(&resolution);
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("recording thread");
        }

        let after = tally_of(CONCURRENT.name());
        assert_eq!(after.attempted - before.attempted, 32);
        let rendered = std::fs::read_to_string(census_file().expect("census path")).expect("read census");
        let expected = format!(
            "{}\t{}\t{}\t{}\t{}",
            CONCURRENT.name(),
            after.attempted,
            after.executed,
            after.absent,
            after.unusable
        );
        assert!(
            rendered.lines().any(|line| line == expected),
            "flushed census did not retain the latest tally:\n{rendered}"
        );
    }

    /// The hard half of the contract: on a platform CI installs the toolchain for, a missing
    /// toolchain must fail the run rather than be counted as a skip. Without this the census
    /// alone would let a regressed runner setup pass, since a skip is a legitimate outcome
    /// everywhere else. ~keep
    #[test]
    fn required_mode_fails_when_the_toolchain_is_unavailable() {
        let result = std::panic::catch_unwind(|| GO.require_available(Resolution::Absent, true));

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

    #[test]
    fn required_mode_preserves_an_unusable_toolchains_diagnostic() {
        let result = std::panic::catch_unwind(|| {
            GO.require_available(Resolution::Unusable("compiler exploded".to_owned()), true)
        });

        let panic = result.expect_err("required mode must fail for an unusable toolchain");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic");
        assert!(message.contains("present but unusable"), "unexpected panic: {message}");
        assert!(message.contains("compiler exploded"), "unexpected panic: {message}");
    }

    /// The other half: absent and *not* required is a skip, not a failure -- otherwise a
    /// contributor without Go installed cannot run the suite at all, which is how the Go fixtures
    /// came to be permanently red on two of the three CI legs. ~keep
    #[test]
    fn unrequired_mode_reports_an_absent_toolchain_as_a_skip() {
        assert_eq!(
            GO.require_available(Resolution::Absent, false),
            None,
            "an absent, unrequired toolchain must be reported as not-run rather than panicking"
        );
    }

    #[test]
    fn deterministic_capability_controls_accept_real_work_for_every_probed_language() {
        let tools = fake_toolchains(false);

        GO.probe(&tools.go).expect("Go control performs a build");
        SWIFT
            .probe(&tools.swift)
            .expect("Swift control compiles Foundation source");
        RUBY.probe(&tools.ruby).expect("Ruby control loads JSON");
    }

    #[test]
    fn version_only_tools_are_unusable_for_every_capability_probe() {
        let tools = fake_toolchains(true);

        for (gate, binary) in [(&GO, &tools.go), (&SWIFT, &tools.swift), (&RUBY, &tools.ruby)] {
            let Err(diagnostic) = gate.probe(binary) else {
                panic!("{} version-only control unexpectedly passed", gate.name());
            };
            assert!(
                diagnostic.contains("deliberately-broken"),
                "{} lost the real capability diagnostic: {diagnostic}",
                gate.name()
            );
        }
    }

    struct FakeToolchains {
        _root: tempfile::TempDir,
        go: PathBuf,
        swift: PathBuf,
        ruby: PathBuf,
    }

    fn fake_toolchains(broken: bool) -> FakeToolchains {
        let root = tempfile::tempdir().expect("fake toolchain root");
        let directory = root.path().join(if broken { "broken" } else { "working" });
        std::fs::create_dir(&directory).expect("create fake toolchain directory");
        let source = root.path().join("fake_tool.rs");
        let compiled = root
            .path()
            .join(if cfg!(windows) { "fake_tool.exe" } else { "fake_tool" });
        std::fs::write(&source, FAKE_TOOL_SOURCE).expect("write fake tool source");
        let rustc = which::which("rustc").expect("cargo tests require rustc");
        let output = std::process::Command::new(rustc)
            .arg(&source)
            .arg("-o")
            .arg(&compiled)
            .output()
            .expect("compile fake tool");
        assert!(output.status.success(), "fake tool compile failed: {output:?}");

        let named = |name: &str| {
            let path = directory.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            });
            std::fs::copy(&compiled, &path).expect("copy fake tool");
            path
        };
        let go = named("go");
        let swift = named("swift");
        let _swiftc = named("swiftc");
        let ruby = named("ruby");
        FakeToolchains {
            _root: root,
            go,
            swift,
            ruby,
        }
    }

    const FAKE_TOOL_SOURCE: &str = r#"
use std::path::PathBuf;

fn main() {
    let executable = std::env::current_exe().unwrap();
    let name = executable.file_stem().unwrap().to_string_lossy();
    let broken = executable.parent().unwrap().file_name().unwrap() == "broken";
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("version" | "--version")) {
        return;
    }
    if broken {
        eprintln!("deliberately-broken capability");
        std::process::exit(42);
    }
    let output = if name.starts_with("go") {
        assert_eq!(args.first().map(String::as_str), Some("build"));
        args.iter().position(|arg| arg == "-o").map(|index| PathBuf::from(&args[index + 1]))
    } else if name.starts_with("swiftc") {
        let source = std::fs::read_to_string(&args[0]).unwrap();
        assert!(source.contains("import Foundation"));
        args.iter().position(|arg| arg == "-o").map(|index| PathBuf::from(&args[index + 1]))
    } else if name.starts_with("ruby") {
        assert_eq!(args, ["-e", "require \"json\""]);
        None
    } else {
        panic!("unexpected fake tool invocation: {name} {args:?}");
    };
    if let Some(path) = output {
        std::fs::write(path, b"probe").unwrap();
    }
}
"#;

    fn tally_of(name: &'static str) -> Tally {
        lock().tallies.get(name).copied().unwrap_or_default()
    }
}
