//! One-process batch runner for the generated-Go compile-and-run fixtures.
//!
//! The fixtures in this directory used to spawn one `go test` per case, paying process
//! startup and module resolution every time. The cases are independent Go *packages*, not
//! independent *modules*, so a single module root with one package directory per case runs
//! all of them in one `go test -v ./...` while keeping compilation isolated per case: an
//! unused import, a build error, or a failing assertion still lands on exactly one case.
//!
//! `-v` rather than `-json` is deliberate: `go test -json` drops build diagnostics entirely
//! (they appear neither in the JSON stream nor on stderr), which would silently discard the
//! compiler error a failing compile fixture exists to report. ~keep

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::test_support::toolchain;

/// One case in a batch: a single Go package directory inside the shared batch module.
pub(super) struct GoBatchCase {
    /// Directory name under the batch module root. Must be unique and a valid Go import
    /// path element; it is also the case identity reported back in failures.
    pub name: String,
    /// Files written inside the case directory, as (path relative to the case dir, content).
    pub files: Vec<(String, String)>,
}

/// Where the batch module is laid out and how `go test` is invoked over it.
pub(super) struct GoBatchLayout {
    /// Files written relative to the batch root before any case: the `go.mod`, and any
    /// package the cases resolve through a `replace` directive.
    pub root_files: Vec<(PathBuf, String)>,
    /// Directory relative to the batch root holding `go.mod` and the case packages. This is
    /// the working directory `go test` runs in.
    pub module_dir: PathBuf,
    /// Import path declared by the batch module's `go.mod`, used to map the package paths
    /// `go test` reports back onto case names.
    pub module_path: String,
    /// Extra `go test` flags inserted before `-v ./...`.
    pub extra_args: Vec<String>,
}

/// Whether `go test` reported a case's package as passing or failing. A build failure is a
/// failure: Go reports it as `FAIL <package> [build failed]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GoCaseOutcome {
    Passed,
    Failed,
}

/// The slice of one batched `go test` run belonging to a single case.
pub(super) struct GoCaseReport {
    pub outcome: GoCaseOutcome,
    /// The case's `go test -v` block, followed by any build diagnostics attributed to it.
    pub output: String,
    /// Top-level `--- PASS:` / `--- FAIL:` / `--- SKIP:` lines observed for the case.
    pub test_case_count: usize,
}

/// Results of one batched `go test` run, keyed by case name.
pub(super) struct GoBatchReport {
    cases: BTreeMap<String, GoCaseReport>,
    stderr: String,
    /// Kept alive so the generated sources stay on disk while failures are being reported.
    _root: tempfile::TempDir,
}

impl GoBatchReport {
    /// The report for one case, panicking with the full observed inventory if `go test`
    /// never reported on it. A case that silently drops out of the batch is the failure
    /// mode this whole harness exists to prevent, so it is never a soft miss. ~keep
    pub fn case(&self, name: &str) -> &GoCaseReport {
        self.cases.get(name).unwrap_or_else(|| {
            panic!(
                "batched Go case `{name}` produced no `go test` result; observed cases: {:?}\nstderr:\n{}",
                self.cases.keys().collect::<Vec<_>>(),
                self.stderr
            )
        })
    }

    /// Total number of Go test functions the batch actually executed.
    pub fn total_test_cases(&self) -> usize {
        self.cases.values().map(|case| case.test_case_count).sum()
    }

    /// Assert set equality between the cases the batch was asked to run and the cases
    /// `go test` reported on. A batch that selects fewer packages than intended still exits
    /// 0 and is otherwise indistinguishable from a real pass, so the inventory — not just
    /// its size — is the gate. ~keep
    pub fn assert_inventory(&self, expected: &[String]) {
        let mut wanted: Vec<&str> = expected.iter().map(String::as_str).collect();
        wanted.sort_unstable();
        let unique = {
            let mut deduped = wanted.clone();
            deduped.dedup();
            deduped.len()
        };
        assert_eq!(
            unique,
            expected.len(),
            "batched Go case names must be unique: {expected:?}"
        );
        assert!(!expected.is_empty(), "a batched Go run must contain at least one case");
        let observed: Vec<&str> = self.cases.keys().map(String::as_str).collect();
        assert_eq!(
            observed, wanted,
            "batched `go test` did not report on exactly the requested cases\nstderr:\n{}",
            self.stderr
        );
    }

    /// Assert one case's outcome, reporting that case's own output on mismatch.
    pub fn assert_outcome(&self, name: &str, expected: GoCaseOutcome) {
        let case = self.case(name);
        assert_eq!(
            case.outcome, expected,
            "batched Go case `{name}` outcome mismatch:\n{}",
            case.output
        );
    }

    /// Assert a diagnostic appears in one case's own output block, not merely somewhere in
    /// the batch — otherwise one case's message could vouch for another's. ~keep
    pub fn assert_output_contains(&self, name: &str, needle: &str) {
        let case = self.case(name);
        assert!(
            case.output.contains(needle),
            "batched Go case `{name}` output is missing {needle:?}:\n{}",
            case.output
        );
    }
}

/// Write every case into one throwaway Go module and run them all in a single `go test`.
///
/// `None` means the Go toolchain is not installed, so nothing was compiled and the caller must
/// return without asserting. The skip is already counted by [`toolchain::ToolchainGate::open`],
/// which is what stops a batch that ran zero cases from reading as a batch that passed them
/// all. ~keep
pub(super) fn run_go_batch(layout: &GoBatchLayout, cases: &[GoBatchCase]) -> Option<GoBatchReport> {
    assert!(!cases.is_empty(), "a batched Go run must contain at least one case");
    let go = toolchain::GO.open()?;
    let root = tempfile::tempdir().expect("create batched Go module root");
    for (path, content) in &layout.root_files {
        write_batch_file(&root.path().join(path), content);
    }
    let module_root = root.path().join(&layout.module_dir);
    for case in cases {
        let case_root = module_root.join(&case.name);
        for (path, content) in &case.files {
            write_batch_file(&case_root.join(path), content);
        }
    }

    let mut command = Command::new(go);
    command.arg("test");
    for arg in &layout.extra_args {
        command.arg(arg);
    }
    let output = command
        .args(["-v", "./..."])
        .current_dir(&module_root)
        .output()
        .expect("run batched Go packages");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let parsed = parse_case_blocks(&stdout, &stderr, &layout.module_path);
    Some(GoBatchReport {
        cases: parsed,
        stderr,
        _root: root,
    })
}

fn write_batch_file(path: &Path, content: &str) {
    let parent = path.parent().expect("batched Go file has a parent directory");
    std::fs::create_dir_all(parent).expect("create batched Go case directory");
    std::fs::write(path, content).expect("write batched Go case file");
}

/// Split `go test -v` stdout into per-package blocks. Go buffers each package's verbose
/// output and prints it contiguously, terminated by the package's own `ok`/`FAIL`/`?` line.
fn parse_case_blocks(stdout: &str, stderr: &str, module_path: &str) -> BTreeMap<String, GoCaseReport> {
    let prefix = format!("{module_path}/");
    let mut parsed = BTreeMap::new();
    let mut block = String::new();
    for line in stdout.lines() {
        block.push_str(line);
        block.push('\n');
        let Some((status, package)) = package_terminator(line) else {
            continue;
        };
        let Some(name) = package.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let block_text = std::mem::take(&mut block);
        let test_case_count = block_text
            .lines()
            .filter(|line| {
                line.starts_with("--- PASS:") || line.starts_with("--- FAIL:") || line.starts_with("--- SKIP:")
            })
            .count();
        let outcome = if status == "FAIL" {
            GoCaseOutcome::Failed
        } else {
            GoCaseOutcome::Passed
        };
        let diagnostics = build_diagnostics(stderr, package);
        parsed.insert(
            name.to_owned(),
            GoCaseReport {
                outcome,
                output: format!("{block_text}{diagnostics}"),
                test_case_count,
            },
        );
    }
    parsed
}

/// Recognize `ok <pkg> <time>`, `FAIL <pkg> [build failed]`, and `? <pkg> [no test files]`.
/// The trailing bare `FAIL` summary line has no package field and is deliberately not a
/// terminator. ~keep
fn package_terminator(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    let status = fields.next()?;
    if !matches!(status, "ok" | "FAIL" | "?") {
        return None;
    }
    let package = fields.next()?;
    package.contains('/').then_some((status, package))
}

/// Build errors land on stderr in `# <package>` headed blocks rather than in the `-v`
/// stream, so re-attach each block to the case it belongs to.
fn build_diagnostics(stderr: &str, package: &str) -> String {
    let mut collected = String::new();
    let mut capturing = false;
    for line in stderr.lines() {
        if line.starts_with("# ") {
            capturing = diagnostic_header_matches(line, package);
        }
        if capturing {
            collected.push_str(line);
            collected.push('\n');
        }
    }
    collected
}

/// Headers read `# example.com/m/case_test [example.com/m/case.test]`. Match on the full
/// path plus its Go-added suffix so a case name that prefixes another case's name does not
/// steal its diagnostics. ~keep
fn diagnostic_header_matches(line: &str, package: &str) -> bool {
    line.split_whitespace().nth(1).is_some_and(|reported| {
        reported == package
            || reported
                .strip_prefix(package)
                .is_some_and(|rest| rest == "_test" || rest == ".test")
    })
}
