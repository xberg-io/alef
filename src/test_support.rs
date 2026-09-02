//! Test-only support shared across the whole crate.
//!
//! `cargo test` runs every `#[test]` as a thread inside one process, so any state a test mutates
//! through a process-global API -- like `std::env::set_current_dir` -- is shared mutable state
//! across every other test in the binary, not just the tests in the same module. Before this
//! module existed, four separate `CWD_LOCK` statics lived in `cli::cache`,
//! `cli::breaking_changes`, `cli::pipeline::version_tests`, and
//! `cli::pipeline::generate::generation` (plus an unguarded fifth lock local to
//! `bin_cli::all_commands_tests`), each correctly serializing the tests in its own module but
//! doing nothing to serialize against the other four -- so two cwd-mutating tests from different
//! modules could still run concurrently and race. [`CWD_LOCK`] is the one lock every cwd-mutating
//! test in this crate now shares. ~keep

pub(crate) mod toolchain;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// The single lock serializing every test in this crate that mutates the process-global current
/// directory. See the module docs for why one shared lock is required rather than one per module.
pub(crate) static CWD_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that enters `dir` as the process current directory for its lifetime and restores
/// the original directory on drop -- including when the guarded scope panics, since `Drop` still
/// runs while a panic unwinds. Holds [`CWD_LOCK`] for its entire lifetime, so at most one
/// `CwdGuard` is ever live across the whole crate at a time.
///
/// A poisoned lock (an earlier guard's scope panicked while holding it) is still acquired: one
/// panicking test must not cascade into every other cwd-mutating test failing on a poisoned
/// mutex, and the poison carries no invalidated data here -- the guard that poisoned the lock had
/// already restored its own original directory via `Drop` before the panic finished unwinding
/// through it.
pub(crate) struct CwdGuard {
    _lock: MutexGuard<'static, ()>,
    original: PathBuf,
}

impl CwdGuard {
    /// Locks [`CWD_LOCK`] and enters `dir` as the process current directory, returning a guard
    /// that restores the original directory when dropped.
    pub(crate) fn enter(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original = std::env::current_dir().expect("read current directory");
        std::env::set_current_dir(dir).expect("enter directory");
        Self { _lock: lock, original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

/// The single lock serializing every test in this crate that mutates the process-global
/// `ALEF_SKIP_COMMANDS` env var. Mirrors [`CWD_LOCK`]'s rationale exactly: before
/// [`SkipCommandsGuard`] existed, `cli::pipeline::commands::build`'s `run_command_tests` module
/// held its own private `env_lock()` `Mutex`, which correctly serialized tests within that one
/// module but did nothing to stop a test in a different module (this crate's own `alef generate`
/// regression coverage for the post-build/format ordering fix) from mutating the same env var
/// concurrently -- the identical "two locks guarding one resource" shape `f968767b6` already had
/// to fix once for `frb_bridge_coverage.rs`'s equivalent hazard. One shared lock closes it for
/// every future caller instead of adding a third independent one. ~keep
pub(crate) static SKIP_COMMANDS_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that locks [`SKIP_COMMANDS_LOCK`], sets `ALEF_SKIP_COMMANDS` to `value` for its
/// lifetime, and restores whatever the env var held before on drop -- including when the guarded
/// scope panics. See [`SKIP_COMMANDS_LOCK`]'s doc for why every test that reads or writes this
/// env var must go through the one shared lock.
pub(crate) struct SkipCommandsGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
}

impl SkipCommandsGuard {
    /// Locks [`SKIP_COMMANDS_LOCK`] and sets `ALEF_SKIP_COMMANDS` to `value`, returning a guard
    /// that restores the previous value (or absence) when dropped.
    pub(crate) fn set(value: &str) -> Self {
        let lock = SKIP_COMMANDS_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        // SAFETY: `_lock` is held for the guard's entire lifetime, so no other thread in this
        // process can be reading or writing `ALEF_SKIP_COMMANDS` through this same guard type
        // concurrently.
        unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", value) };
        Self { _lock: lock, previous }
    }
}

impl Drop for SkipCommandsGuard {
    fn drop(&mut self) {
        // SAFETY: see `set`'s SAFETY comment -- `_lock` is still held here, during `Drop`.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("ALEF_SKIP_COMMANDS", value),
                None => std::env::remove_var("ALEF_SKIP_COMMANDS"),
            }
        }
    }
}

/// The single lock serializing every test in this crate that spawns a REAL `cargo` subprocess
/// (`cargo fmt --all`, `cargo sort -n -w`, `cargo sort --check`, ...) outside of alef's own
/// `ALEF_SKIP_COMMANDS` skip mechanism.
///
/// `ALEF_SKIP_COMMANDS`/[`SkipCommandsGuard`] only gates `PostBuildStep::RunCommand` --
/// `cli::pipeline::format::run_cargo_fmt`/`run_workspace_cargo_sort` (the residual passes
/// `converge_full_regen` folds into every full-regen `alef all`/`alef fmt` pass) run
/// unconditionally whenever a root `Cargo.toml` exists, so they are the one place a genuinely
/// real `cargo` process launches during this crate's own test suite -- and [`CWD_LOCK`] does not
/// reach them: several tests call `run_cargo_fmt`/`run_workspace_cargo_sort`/
/// `converge_full_regen_formatting` directly with an explicit `.current_dir(..)`-equivalent
/// `base` argument and never touch the process cwd at all, so they never take `CWD_LOCK` in the
/// first place.
///
/// Measured under load: `bin_cli::core_commands::post_build_format_order_tests`'s two `alef
/// all`-driving tests failed a full `cargo test --lib` run with `Blocking waiting for file lock
/// on package cache`, both passing in isolation and on re-run -- the signature of real, external
/// contention on cargo's own machine-wide package-cache lock file, not a defect in the code under
/// test. `cli::pipeline::format::tests` has four such unguarded direct-call tests
/// (`run_workspace_cargo_sort_sorts_every_member_regardless_of_language`,
/// `run_cargo_fmt_formats_workspace_rust_files_when_available`,
/// `converge_full_regen_formatting_leaves_workspace_sorted_and_poly_fmt_check_clean`,
/// `format_generated_full_regen_routes_through_convergence_loop`) that can and do run
/// concurrently with `alef all`'s own real cargo invocations under `cargo test`'s default
/// parallel scheduling -- the same shape [`CWD_LOCK`] and [`SKIP_COMMANDS_LOCK`] already closed
/// for their own process-global resources, applied to this one. ~keep
pub(crate) static REAL_CARGO_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that holds [`REAL_CARGO_LOCK`] for its lifetime. Every test that spawns a real
/// `cargo` subprocess -- directly, or indirectly through `alef all`/`alef fmt`/a full-regen
/// `format_generated(.., None)` -- must acquire this for the whole span during which that
/// subprocess might run. See [`REAL_CARGO_LOCK`]'s doc for why a dedicated lock is required
/// rather than reusing [`CWD_LOCK`].
pub(crate) struct RealCargoGuard {
    _lock: MutexGuard<'static, ()>,
}

impl RealCargoGuard {
    /// Locks [`REAL_CARGO_LOCK`] for the caller's scope.
    pub(crate) fn acquire() -> Self {
        let lock = REAL_CARGO_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        Self { _lock: lock }
    }
}

/// The single lock serializing every test in this crate that spawns a REAL `mvn` subprocess
/// against [`maven_local_repo_dir`].
///
/// alef task #529's Maven-specific follow-up: `java_checkstyle.rs`'s and `java_pom_compiler.rs`'s
/// three bite tests each shell out to a real `mvn`, and a prior audit flagged that all three used
/// to share the developer's ambient `~/.m2` -- a machine-global resource a genuinely different
/// process (a different worktree's own `mvn`/`cargo` run) could also be touching, with none of
/// this crate's own locks reaching it. Pinning `-Dmaven.repo.local` to a per-worktree, per-run
/// tempdir was rejected before: it closes the cross-worktree hazard but pays a full cold plugin
/// download on every single test run, which is not an acceptable trade for three tests that
/// already take real wall-clock time. [`maven_local_repo_dir`] instead pins the repository to a
/// path scoped to *this checkout* (`target/mvn-repo-cache-test`, derived from `CARGO_MANIFEST_DIR`
/// at compile time) -- so two different worktrees never share one repository directory (the
/// cross-worktree hazard the original audit flagged is closed the same way `CWD_LOCK`/
/// `SKIP_COMMANDS_LOCK`/`REAL_CARGO_LOCK` close their own machine-global resources), while a
/// second run in the *same* worktree reuses whatever the first run already downloaded instead of
/// paying a cold download again. That still leaves one race this lock exists to close: this
/// crate's own three mvn-driving tests running concurrently under `cargo test`'s default
/// parallelism would otherwise all populate that one shared per-worktree cache directory at once,
/// which is exactly the concurrent-write shape that made the ambient `~/.m2` unsafe in the first
/// place, just at smaller scope. ~keep
pub(crate) static REAL_MVN_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that holds [`REAL_MVN_LOCK`] for its lifetime. Every test that spawns a real `mvn`
/// subprocess against [`maven_local_repo_dir`] must acquire this for the whole span during which
/// that subprocess might run. See [`REAL_MVN_LOCK`]'s doc for why a dedicated lock and a dedicated
/// repository directory are both required.
pub(crate) struct RealMvnGuard {
    _lock: MutexGuard<'static, ()>,
}

impl RealMvnGuard {
    /// Locks [`REAL_MVN_LOCK`] for the caller's scope.
    pub(crate) fn acquire() -> Self {
        let lock = REAL_MVN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        Self { _lock: lock }
    }
}

/// The Maven local repository directory every real `mvn` invocation in this crate's test suite
/// must be pinned to via `-Dmaven.repo.local=<path>`, instead of the ambient `~/.m2` every other
/// process on the machine (including a completely different worktree's own build) also reads and
/// writes. See [`REAL_MVN_LOCK`]'s doc for the full rationale, including why this is a persistent
/// per-worktree cache rather than a fresh tempdir per test run.
pub(crate) fn maven_local_repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("mvn-repo-cache-test")
}

/// Whether `mvn` runs, not merely resolves: a version-manager shim (e.g. sdkman, asdf) spawns
/// fine then exits non-zero, so a spawn-only check (`.output().is_err()`) leaves the skip in
/// `java_pom_compiler.rs`'s and `java_checkstyle.rs`'s real-`mvn` bite tests unreachable and
/// fires their assertions everywhere Maven is absent. Shared here rather than duplicated per
/// file because both already route their real `mvn` spawns through [`spawn_from_stable_dir`]. ~keep
pub(crate) fn mvn_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        spawn_from_stable_dir("mvn")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Build a [`std::process::Command`] for `program`, pre-pinned to [`std::env::temp_dir`].
///
/// `cargo test` runs every test as a thread in one process (see the module docs), so a spawn
/// that never calls `.current_dir(..)` inherits whatever the process-wide cwd happens to be at
/// that instant -- including a tempdir another test entered via [`CwdGuard`] and has since
/// deleted. The unpinned spawn then fails with an OS-level "Could not locate working directory"
/// that has nothing to do with the code under test (see `commands::test::get_host_target` and
/// 22baa34ac for two prior instances of exactly this failure). Start every test-only subprocess
/// through this helper instead of `Command::new` directly, so a future call site can't be
/// written unpinned by omission. The system temp directory is a safe default for any spawn that
/// does not itself care what directory it runs in (a `--version` probe, or a tool invoked with
/// only absolute-path arguments); a caller that needs a specific working directory can still
/// chain `.current_dir(..)` again afterward to override this default. ~keep
pub(crate) fn spawn_from_stable_dir(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.current_dir(std::env::temp_dir());
    command
}

/// Assert that `elapsed` stayed under `bound`, embedding both values and the signed margin
/// between them in the panic message.
///
/// alef task #529: `cargo test --lib` had one transient failure under concurrent machine load
/// that a prior audit narrowed to "one of the wall-clock-bounded assertions in this suite, CPU
/// starved by a different process's build running in a different worktree" but could not name,
/// because every candidate's bare `assert!(elapsed < bound, ..)` reported only the elapsed side
/// -- never the bound it was measured against or how close the run actually came to it. Route a
/// bounded wall-clock assertion through this helper instead of hand-writing that comparison so
/// the *next* occurrence identifies itself without another audit: the panic states the measured
/// elapsed time, the asserted bound, and their margin, so a razor-thin or negative margin next to
/// an otherwise-clean run reads as "starved, not broken." `context` names what the bound is
/// timing, so the panic still reads standalone without the caller's source. This only changes
/// what a failure reports -- it must never be used to raise, lower, or otherwise pick a bound;
/// that stays the call site's decision. ~keep
pub(crate) fn assert_elapsed_under(context: &str, elapsed: Duration, bound: Duration) {
    let margin = bound.as_secs_f64() - elapsed.as_secs_f64();
    assert!(
        elapsed < bound,
        "{context}: elapsed {elapsed:?}, bound {bound:?}, margin {margin:+.3}s -- a negative \
         margin here next to an otherwise-clean run is the signature of CPU starvation from a \
         concurrent process, not a defect in the code under test"
    );
}

/// `cargo sort --check` conformance for the table ORDER of a generated `Cargo.toml`.
///
/// Consumers gate CI on `cargo sort --check --workspace`, so every manifest alef emits has to
/// already be in cargo-sort's canonical table order. This module encodes the ordering RULE
/// rather than any one expected manifest, so a table a future emitter adds is covered without
/// touching this file. ~keep
pub(crate) mod cargo_sort_order {
    /// cargo-sort's `DEF_TABLE_ORDER`, verbatim from its `src/fmt.rs` at v2.1.4 (the version
    /// pinned in CI). Tables absent from this list -- `lints`, `profile`, `patch`, `badges` --
    /// are sorted AFTER every listed one, which is why `[lints.*]` must be emitted last and not
    /// tucked between `[package]` and `[dependencies]`. ~keep
    pub(crate) const DEF_TABLE_ORDER: &[&str] = &[
        "package",
        "workspace",
        "lib",
        "bin",
        "features",
        "dependencies",
        "build-dependencies",
        "dev-dependencies",
    ];

    /// Split a table header's inner text on `.`, treating quoted spans as opaque so a dotted
    /// cfg predicate (`target.'cfg(target_os = "x.y")'.dependencies`) stays one segment. ~keep
    fn header_segments(inner: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        for character in inner.chars() {
            match quote {
                Some(open) if character == open => quote = None,
                Some(_) => current.push(character),
                None if character == '\'' || character == '"' => quote = Some(character),
                None if character == '.' => segments.push(std::mem::take(&mut current)),
                None => current.push(character),
            }
        }
        segments.push(current);
        segments
    }

    fn rank_of(name: &str) -> usize {
        DEF_TABLE_ORDER
            .iter()
            .position(|table| *table == name)
            .unwrap_or(DEF_TABLE_ORDER.len())
    }

    /// Sort key cargo-sort effectively assigns a top-level table header.
    ///
    /// The first segment picks the group, because a subtable is repositioned immediately after
    /// its parent (`[package.metadata.*]` rides along with `[package]`). `[target.*]` is
    /// cargo-sort's one special case: its nested dependency table is grouped with that
    /// dependency KIND rather than sorted under `target`, and lands just after the plain table
    /// of the same kind -- hence the second tuple element. Every unlisted table shares the
    /// trailing rank, so their order relative to each other is unconstrained, matching
    /// cargo-sort's preservation of document order among them. ~keep
    fn table_sort_key(inner: &str) -> (usize, u8) {
        let segments = header_segments(inner);
        let first = segments.first().map(String::as_str).unwrap_or_default();
        if first == "target" {
            let kind = segments.last().map(String::as_str).unwrap_or_default();
            return (rank_of(kind), 1);
        }
        (rank_of(first), 0)
    }

    /// The dependency tables cargo-sort sorts the KEYS of: its `MATCHER.heading` list, plus the
    /// `[workspace.<kind>]` entries of its `MATCHER.heading_key` list, plus the same three names
    /// nested under `[target.'cfg(...)']`. All three spellings reduce to these names. ~keep
    const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

    /// Assert every dependency table in `manifest` already has its KEYS in the order
    /// `cargo sort --check` requires, returning how many keys were compared.
    ///
    /// cargo-sort sorts a dependency table with `toml_edit`'s `Table::sort_values`, which is
    /// `IndexMap::sort_keys` over `Key: Ord`, and `Key::cmp` compares `Key::get()` -- the decoded
    /// text of one key segment. So this checker parses the manifest with `toml_edit` and compares
    /// the emitted key order against the sorted one, running cargo-sort's own comparison machinery
    /// rather than re-deriving a rule from the line text. That is what makes it see a dotted entry
    /// (`tracing.workspace = true`) as the single key `tracing`, which is exactly what alef's
    /// line-text sorters used to get wrong: raw text puts `tracing-core` first because `-` (0x2D)
    /// precedes `.` (0x2E), while cargo-sort compares `tracing` against `tracing-core`. ~keep
    pub(crate) fn assert_dependency_keys_sorted(label: &str, manifest: &str) -> usize {
        let document = manifest
            .parse::<toml_edit::DocumentMut>()
            .unwrap_or_else(|error| panic!("{label}: generated manifest must be valid TOML: {error}\n{manifest}"));
        let mut compared = 0usize;
        assert_table_keys_sorted(label, "", document.as_table(), manifest, &mut compared);
        compared
    }

    /// Recursive worker for [`assert_dependency_keys_sorted`].
    ///
    /// Descends through header tables only. A dotted sub-table (the `workspace` under
    /// `tracing.workspace = true`) is skipped, mirroring cargo-sort's own requirement that a
    /// matched nested table have a header position -- and keeping a dependency named
    /// `dependencies` from being mistaken for a dependency table. ~keep
    fn assert_table_keys_sorted(
        label: &str,
        path: &str,
        table: &toml_edit::Table,
        manifest: &str,
        compared: &mut usize,
    ) {
        for (name, item) in table.iter() {
            let Some(child) = item.as_table() else { continue };
            if child.is_dotted() {
                continue;
            }
            let child_path = if path.is_empty() {
                name.to_owned()
            } else {
                format!("{path}.{name}")
            };
            if DEPENDENCY_TABLES.contains(&name) {
                let emitted: Vec<&str> = child.iter().map(|(key, _)| key).collect();
                let mut expected = emitted.clone();
                expected.sort_unstable();
                assert_eq!(
                    emitted, expected,
                    "{label}: `[{child_path}]` keys are not in cargo-sort order -- it compares the \
                     bare dependency NAME (a dotted key like `foo.workspace` is the single key \
                     `foo`), so `cargo sort --check` would reorder this manifest and fail it:\n{manifest}"
                );
                *compared += emitted.len();
            }
            assert_table_keys_sorted(label, &child_path, child, manifest, compared);
        }
    }

    /// Assert every table header in `manifest` appears in cargo-sort's canonical order.
    ///
    /// `label` identifies the manifest in the failure message.
    pub(crate) fn assert_canonical_table_order(label: &str, manifest: &str) {
        let mut previous: Option<((usize, u8), &str)> = None;
        let mut header_count = 0usize;
        for line in manifest.lines() {
            let trimmed = line.trim();
            let Some(inner) = trimmed.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) else {
                continue;
            };
            let inner = inner
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(inner);
            header_count += 1;
            let key = table_sort_key(inner);
            if let Some((previous_key, previous_header)) = previous {
                assert!(
                    key >= previous_key,
                    "{label}: table `{trimmed}` must not follow `{previous_header}` -- cargo-sort \
                     orders tables {DEF_TABLE_ORDER:?} first and every other table after them, so \
                     `cargo sort --check` would reorder this manifest and fail it:\n{manifest}"
                );
            }
            previous = Some((key, trimmed));
        }
        assert!(
            header_count > 0,
            "{label}: no table headers found, so this check examined nothing:\n{manifest}"
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Guards the checker itself: it must REJECT the exact layout that broke consumers --
        /// `[lints.clippy]` emitted between `[package]` and `[dependencies]`. Without this, a
        /// checker that never fails is indistinguishable from a fixed emitter. ~keep
        #[test]
        fn should_reject_lints_table_placed_before_dependencies() {
            let manifest = "[package]\nname = \"demo\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\n\n\
                            [dependencies]\nserde = \"1\"\n";
            let result = std::panic::catch_unwind(|| assert_canonical_table_order("demo", manifest));
            assert!(result.is_err(), "checker must reject lints emitted before dependencies");
        }

        #[test]
        fn should_accept_lints_table_placed_last() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\n\n\
                            [lints.clippy]\ndbg_macro = \"deny\"\n";
            assert_canonical_table_order("demo", manifest);
        }

        /// `[package.metadata.*]` rides with `[package]`, and a `[target.*.dependencies]` block
        /// sits with the plain `[dependencies]` table rather than after `[dev-dependencies]`.
        #[test]
        fn should_accept_subtables_and_target_dependency_blocks() {
            let manifest = "[package]\nname = \"demo\"\n\n[package.metadata.cargo-machete]\n\
                            ignored = []\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[features]\n\
                            default = []\n\n[dependencies]\nserde = \"1\"\n\n\
                            [target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n\n\
                            [build-dependencies]\ncc = \"1\"\n\n[dev-dependencies]\n\
                            tempfile = \"3\"\n\n[lints.clippy]\ndbg_macro = \"deny\"\n";
            assert_canonical_table_order("demo", manifest);
        }

        /// Guards the key checker itself against being vacuous: it must REJECT the exact
        /// emitted order that failed downstream -- a dotted `alpha.workspace` entry placed after
        /// `alpha-parser`, which is what byte-wise line sorting produces. ~keep
        #[test]
        fn should_reject_dotted_key_ordered_by_raw_line_text() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\n\
                            alpha-parser = { version = \"1\", path = \"../core\" }\nalpha.workspace = true\n";
            let result = std::panic::catch_unwind(|| assert_dependency_keys_sorted("demo", manifest));
            assert!(
                result.is_err(),
                "checker must reject `alpha-parser` emitted before `alpha.workspace`"
            );
        }

        #[test]
        fn should_accept_dotted_key_ordered_by_dependency_name() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\n\
                            alpha.workspace = true\nalpha-parser = { version = \"1\", path = \"../core\" }\n";
            assert_eq!(
                assert_dependency_keys_sorted("demo", manifest),
                2,
                "both dependency keys must have been compared"
            );
        }

        /// The checker must reach dependency tables nested under `[target.'cfg(...)']` and under
        /// `[workspace]`, not just the top-level `[dependencies]`.
        #[test]
        fn should_check_target_and_workspace_dependency_tables() {
            let manifest = "[workspace]\nmembers = []\n\n[workspace.dependencies]\n\
                            serde = \"1\"\n\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n";
            assert_eq!(
                assert_dependency_keys_sorted("demo", manifest),
                2,
                "the workspace and target dependency tables must both have been visited"
            );
            let broken = "[target.'cfg(unix)'.dependencies]\nlibc-extra = \"1\"\nlibc.workspace = true\n";
            let result = std::panic::catch_unwind(|| assert_dependency_keys_sorted("demo", broken));
            assert!(result.is_err(), "checker must reach into target dependency tables");
        }

        #[test]
        fn should_reject_features_table_placed_after_dependencies() {
            let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\n\n\
                            [features]\ndefault = []\n";
            let result = std::panic::catch_unwind(|| assert_canonical_table_order("demo", manifest));
            assert!(
                result.is_err(),
                "checker must reject features emitted after dependencies"
            );
        }
    }
}
/// A `git` invocation rooted at `root` and isolated from the ambient environment's git
/// configuration.
///
/// Every fixture repository in this crate is built by shelling out to a real `git` rather than a
/// stub, because the behaviour under test is precisely what `git ls-files` and `git log` report: a
/// fake would encode the test's assumption about git's answer instead of measuring it, and the
/// defects these fixtures cover were all cases where the assumed answer and the real one differed.
///
/// Shelling out, though, means the child `git` reads the *developer's* `~/.gitconfig` unless told
/// not to, which makes a test's outcome a function of the machine running it. That is not
/// hypothetical: with `commit.gpgsign = true` set globally -- a common and entirely reasonable
/// developer setting -- every `git commit` below is signed, so these tests silently depend on a
/// working gpg-agent and fail with `gpg: signing failed` when it cannot serve a signature under
/// parallel test load. A test that passes on a box with no signing key and fails on one that has
/// it is not testing what it claims to.
///
/// `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` pointed at `/dev/null` neutralize the whole ambient
/// config surface in one move rather than denylisting settings one at a time -- not just signing,
/// but `core.hooksPath`, `core.excludesFile`, `init.defaultBranch`, aliases and anything a future
/// developer happens to set. The explicit `-c` overrides then supply the few values a fixture
/// genuinely needs, which the now-empty config no longer provides. ~keep
pub(crate) fn git_command(root: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Alef Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.excludesFile=/dev/null",
            "-c",
            "init.defaultBranch=main",
        ]);
    command
}

/// Initialize a fixture repository at `root` so git can report which files it tracks or ignores.
pub(crate) fn git_init(root: &Path) {
    let status = git_command(root).args(["init", "-q"]).status().expect("git init");
    assert!(status.success(), "git init must succeed for a tracked-ness fixture");
}

/// Stage `relative` (paths relative to `root`) into `root`'s index, making them tracked.
pub(crate) fn git_add(root: &Path, relative: &[&str]) {
    let status = git_command(root)
        .arg("add")
        .arg("--")
        .args(relative)
        .status()
        .expect("git add");
    assert!(status.success(), "git add must succeed for a tracked-ness fixture");
}

/// Stage every change under `root` and commit it as `message`.
pub(crate) fn git_commit_all(root: &Path, message: &str) {
    let status = git_command(root).args(["add", "-A"]).status().expect("git add -A");
    assert!(status.success(), "git add -A must succeed for a history fixture");
    let status = git_command(root)
        .args(["commit", "--quiet", "-m", message])
        .status()
        .expect("git commit");
    assert!(status.success(), "git commit must succeed for a history fixture");
}

/// Render `path` the way a portable test assertion should compare it: forward slashes only,
/// regardless of the host OS.
///
/// `Path::to_string_lossy`/`Path::display` render using the *host* separator. A test that then
/// compares the result against a forward-slash literal, or checks a forward-slash-shaped suffix
/// like `.ends_with("/LICENSE")`, passes only on Unix and fails on Windows even when the path
/// itself is correct for that platform — the path is right, the raw-string comparison is not
/// portable (alef task #527). Two different Windows renderings of the *same* logical path can
/// also disagree with each other: `PathBuf::join` inserts the native separator only BETWEEN the
/// pieces it is given, so `root.join("target/release/deps")` (one already-slashed literal, never
/// split) keeps its embedded `/` untouched while `root.join("target").join("release").join
/// ("deps")` (three separate calls) renders fully `\`-joined on Windows — two strings that name
/// the same file but do not string-match each other until both are normalized here. Route every
/// such comparison through this helper instead of hand-rolling `.replace('\\', "/")` at each call
/// site; it only touches the *rendering*, so a test that would still legitimately fail (wrong
/// component, missing segment) still fails after normalization. ~keep
pub(crate) fn portable_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Write `content` to `root/relative`, creating parent directories as needed.
pub(crate) fn write_file(root: &Path, relative: &str, content: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(&path, content).expect("write fixture file");
    path
}

#[cfg(test)]
mod git_hermeticity_tests {
    use super::{git_command, git_commit_all, git_init, write_file};

    /// Pin the property the shared git helper exists to guarantee: a fixture commit is
    /// unsigned no matter what the surrounding configuration asks for.
    ///
    /// The repository-local `commit.gpgsign = true` written here is a *stronger* demand than
    /// the developer `~/.gitconfig` that caused the original flake, because git ranks local
    /// config above global -- so a helper that survives this necessarily survives an ambient
    /// global setting too. Asserting on `%G?` rather than merely on the commit succeeding is
    /// what makes this a real check: a signature that happened to succeed would still leave
    /// the commit signed, and would still couple every fixture to a working gpg-agent. ~keep
    #[test]
    fn git_commit_all_is_unsigned_even_when_local_config_demands_signing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git_init(root);

        let configured = git_command(root)
            .args(["config", "--local", "commit.gpgsign", "true"])
            .status()
            .expect("git config");
        assert!(configured.success(), "the hostile local config must be written");
        let configured = git_command(root)
            .args(["config", "--local", "user.signingkey", "DEADBEEFDEADBEEF"])
            .status()
            .expect("git config");
        assert!(configured.success(), "the bogus signing key must be written");

        write_file(root, "build.zig", "const std = @import(\"std\");\n");
        git_commit_all(root, "scaffold");

        let signature = git_command(root)
            .args(["log", "-1", "--format=%G?"])
            .output()
            .expect("git log");
        assert!(signature.status.success(), "git log must succeed");
        assert_eq!(
            String::from_utf8_lossy(&signature.stdout).trim(),
            "N",
            "the fixture commit must carry no signature, so the test never depends on a working \
             gpg-agent on the machine running it"
        );
    }

    /// The ambient default branch name must not leak into a fixture either -- a developer with
    /// `init.defaultBranch` set to anything but `main` would otherwise build a differently
    /// shaped repository than CI does. ~keep
    #[test]
    fn git_init_uses_a_pinned_default_branch_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git_init(root);
        write_file(root, "seed.txt", "seed\n");
        git_commit_all(root, "scaffold");

        let branch = git_command(root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .expect("git rev-parse");
        assert_eq!(
            String::from_utf8_lossy(&branch.stdout).trim(),
            "main",
            "the fixture branch name must be pinned by the helper, not inherited from the host"
        );
    }
}

#[cfg(test)]
mod assert_elapsed_under_tests {
    use super::assert_elapsed_under;
    use std::time::Duration;

    /// The common, non-flaky case: elapsed comfortably under bound must not panic.
    #[test]
    fn does_not_panic_when_elapsed_is_under_the_bound() {
        assert_elapsed_under("probe", Duration::from_millis(100), Duration::from_secs(1));
    }

    /// The failure this helper exists for: the panic message must carry the elapsed value, the
    /// bound, and a margin -- not just "it was too slow" -- so a future occurrence is
    /// self-diagnosing without another audit. ~keep
    #[test]
    fn panics_with_elapsed_bound_and_margin_when_over_the_bound() {
        let result = std::panic::catch_unwind(|| {
            assert_elapsed_under("probe", Duration::from_millis(1200), Duration::from_secs(1));
        });
        let error = result.expect_err("elapsed over the bound must panic");
        let message = *error.downcast::<String>().expect("panic payload is a String");
        assert!(message.contains("probe"), "message must name the context: {message}");
        assert!(
            message.contains("1.2s"),
            "message must state the measured elapsed: {message}"
        );
        assert!(
            message.contains("1s"),
            "message must state the asserted bound: {message}"
        );
        assert!(
            message.contains("margin -0.200s"),
            "message must state the signed margin between elapsed and bound: {message}"
        );
    }
}

#[cfg(test)]
mod portable_path_string_tests {
    use super::portable_path_string;
    use std::path::Path;

    /// The behavior this crate's own test suite (run on macOS/Linux) never exercises for free:
    /// a `\`-separated rendering, the shape `Path::to_string_lossy` actually produces on
    /// Windows. Fed in explicitly here so the normalization is proven on any host, not just
    /// assumed. ~keep
    #[test]
    fn replaces_every_backslash_with_a_forward_slash() {
        assert_eq!(
            portable_path_string(Path::new("packages\\java\\src")),
            "packages/java/src"
        );
    }

    /// A path with no backslashes at all -- the common case on Unix hosts -- must render
    /// unchanged, so the helper is a normalization, not a rewrite of unrelated content.
    #[test]
    fn leaves_a_forward_slash_path_unchanged() {
        assert_eq!(
            portable_path_string(Path::new("packages/java/src")),
            "packages/java/src"
        );
    }

    /// A path mixing both separators -- the exact shape `PathBuf::join` can produce when one
    /// joined piece was itself a pre-slashed literal (see the helper's doc) -- must fully
    /// normalize, proving this is a blanket replace and not a prefix/suffix-shaped check.
    #[test]
    fn normalizes_a_path_mixing_both_separators() {
        assert_eq!(
            portable_path_string(Path::new("root\\target/release/deps\\lib.so")),
            "root/target/release/deps/lib.so"
        );
    }

    /// A genuinely different path must still compare different after normalization -- proving
    /// this helper cannot mask a real mismatch, only a separator-rendering one.
    #[test]
    fn does_not_equate_genuinely_different_paths() {
        assert_ne!(
            portable_path_string(Path::new("packages\\java")),
            portable_path_string(Path::new("packages\\kotlin"))
        );
    }
}
