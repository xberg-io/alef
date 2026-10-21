//! The anti-vacuity half of the Elixir oracles: proof that something still selects them.
//!
//! Every lane in the parent module is `#[ignore]`d. That is deliberate — a machine without
//! `elixir` must not report a green having evaluated nothing — but it moves the whole failure mode
//! one level up: `cargo test` runs none of them and exits 0, so if the CI step that passes
//! `--ignored` is edited away, the suite stays green and the oracles quietly stop existing.
//!
//! Neither test here is ignored and neither needs a toolchain.
//! [`ignored_elixir_oracle_lanes_are_selected_and_nonzero`] asks libtest itself how many tests the
//! oracle selection actually resolves to, so "zero tests ran" becomes a failure rather than a
//! pass. [`ci_workflow_selects_the_ignored_elixir_oracle_lanes`] checks the step that does the
//! selecting still does it.
//!
//! The workflow check reads the step's actual `run:` line and skips comment lines on the way. That
//! is not defensive styling: the comment sitting above that very step explains why `--ignored`
//! matters and therefore CONTAINS the string `--ignored`, so a `block.contains("--ignored")` would
//! keep passing after the flag was deleted from the real command. Matching the comment instead of
//! the command is precisely the vacuous shape these gates exist to prevent. ~keep
//!
//! # Porting this gate to another suite
//!
//! [`ignored_elixir_oracle_lanes_are_selected_and_nonzero`] is deliberately generic: it asks the
//! RUNNING test binary to `--list` what a filter resolves to, so it catches the one failure a
//! command-text check cannot see -- a filter that matches nothing. `cargo test --lib
//! definitely_no_such_test -- --ignored` runs zero tests and exits 0, and a wiring check that
//! only reads the `run:` line calls that green. This does not.
//!
//! To port it, copy this file and change the four constants below and nothing else:
//! [`ORACLE_FILTER`] (the substring the CI step passes), [`EXPECTED_IGNORED_LANES`] (how many
//! `#[ignore]`d tests that filter must resolve to), [`TEST_JOB`] and [`ORACLE_STEP`] (the job and
//! step names in `.github/workflows/ci.yml`). The two helpers at the bottom and their own
//! `the_workflow_readers_narrow_to_what_they_name` test travel unchanged. ~keep

use std::path::PathBuf;
use std::process::Command;

/// Substring that selects the oracle lanes, shared by the CI step and the count below.
const ORACLE_FILTER: &str = "elixir_oracle";

/// How many `#[ignore]`d lanes the parent module defines. Pinned rather than merely checked for
/// "more than zero" so that deleting three of the four is a failure, not a silent halving.
const EXPECTED_IGNORED_LANES: usize = 6;

const CI_WORKFLOW: &str = ".github/workflows/ci.yml";

/// The job that has `elixir` on PATH, and so is the only one that can run these lanes.
const TEST_JOB: &str = "test";

/// The step inside [`TEST_JOB`] that selects the ignored lanes.
const ORACLE_STEP: &str = "Run the Elixir escaping oracles";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Ask the running test binary to LIST the ignored tests the oracle filter selects, and require
/// that number to be exactly the number of lanes the parent module defines.
///
/// The count comes from libtest's own selection, not from reading this file's text — a text search
/// would be satisfied by a comment, and would keep passing if the lanes were renamed out of the
/// filter's reach. `--list` resolves the same filter the CI step uses and exits without running
/// anything, so this is cheap and cannot recurse.
#[test]
fn ignored_elixir_oracle_lanes_are_selected_and_nonzero() {
    let test_binary = std::env::current_exe().expect("the running test binary has a path");
    let output = Command::new(&test_binary)
        .args(["--ignored", "--list", ORACLE_FILTER])
        .output()
        .unwrap_or_else(|error| panic!("list tests via {}: {error}", test_binary.display()));
    assert!(
        output.status.success(),
        "listing ignored tests failed ({}):\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let listing = String::from_utf8_lossy(&output.stdout);
    let selected: Vec<&str> = listing
        .lines()
        .filter(|line| line.ends_with(": test"))
        .map(|line| line.trim_end_matches(": test"))
        .collect();

    assert!(
        !selected.is_empty(),
        "`--ignored --list {ORACLE_FILTER}` selected NO tests. Every Elixir oracle is #[ignore]d, \
         so a selection that resolves to zero means the CI step runs nothing and still exits 0 -- \
         the exact green-that-examined-nothing this gate exists to make impossible. Listing was:\n\
         {listing}"
    );
    assert_eq!(
        selected.len(),
        EXPECTED_IGNORED_LANES,
        "the oracle filter selects {} ignored lane(s), not the {EXPECTED_IGNORED_LANES} this \
         module defines. Either a lane was added without updating EXPECTED_IGNORED_LANES, or one \
         was deleted, renamed out of the `{ORACLE_FILTER}` filter, or had its #[ignore] removed. \
         Selected: {selected:?}",
        selected.len()
    );
}

/// The CI step that passes `--ignored` must still exist, still pass it, still name the filter, and
/// sit in a job that installs Elixir.
#[test]
fn ci_workflow_selects_the_ignored_elixir_oracle_lanes() {
    let workflow_path = repo_root().join(CI_WORKFLOW);
    let workflow = std::fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    let block = workflow_job_block(&workflow, TEST_JOB).unwrap_or_else(|| {
        panic!(
            "{} has no `{TEST_JOB}` job, so nothing selects the #[ignore]d Elixir oracles at all",
            workflow_path.display()
        )
    });

    let run_command = step_run_command(&block, ORACLE_STEP).unwrap_or_else(|| {
        panic!(
            "the `{TEST_JOB}` job has no `{ORACLE_STEP}` step with a `run:` command. Every Elixir \
             oracle is #[ignore]d; without that step none of them run anywhere."
        )
    });

    assert!(
        run_command.contains("--ignored"),
        "`{ORACLE_STEP}` no longer passes --ignored, so it selects none of the lanes and still \
         exits 0. Command was: {run_command}"
    );
    assert!(
        run_command.contains(ORACLE_FILTER),
        "`{ORACLE_STEP}` no longer names the `{ORACLE_FILTER}` filter, so it would select every \
         #[ignore]d test in the crate rather than these lanes. Command was: {run_command}"
    );
    assert!(
        block
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| line.contains("setup-elixir")),
        "the `{TEST_JOB}` job must install Elixir, or `{ORACLE_STEP}` fails for want of a \
         toolchain rather than for a real regression"
    );
}

/// Extract one job's block, from its `  <name>:` header to the next line at the same indent.
/// Scoped to the job so a needle satisfied by a neighbouring job's steps cannot pass this.
fn workflow_job_block(workflow: &str, job: &str) -> Option<String> {
    let header = format!("  {job}:");
    let mut lines = workflow.lines().skip_while(|line| line.trim_end() != header);
    let first = lines.next()?;
    let mut block = String::from(first);
    for line in lines {
        let is_sibling_job = line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':');
        if is_sibling_job {
            break;
        }
        block.push('\n');
        block.push_str(line);
    }
    Some(block)
}

/// The shell command a named step actually runs, ignoring comment lines entirely.
///
/// Scans forward from the step's `- name:` line to its `run:` line and stops at the next step, so
/// a later step's command cannot stand in for a missing one. Assumes a single-line `run:` (what
/// this step uses) rather than a `run: |` block.
fn step_run_command<'a>(block: &'a str, step_name: &str) -> Option<&'a str> {
    let header = format!("- name: {step_name}");
    let mut lines = block
        .lines()
        .map(|line| line.trim_start())
        .filter(|line| !line.starts_with('#'))
        .skip_while(|line| line.trim_end() != header);
    lines.next()?;
    for line in lines {
        if line.starts_with("- ") {
            return None;
        }
        if let Some(command) = line.strip_prefix("run:") {
            return Some(command.trim());
        }
    }
    None
}

/// The block extractor and the step reader both have to actually narrow, or every assertion above
/// would be satisfied by some other job's or step's text.
#[test]
fn the_workflow_readers_narrow_to_what_they_name() {
    let workflow = concat!(
        "jobs:\n",
        "  first:\n",
        "    steps:\n",
        "      # a comment mentioning --ignored and elixir_oracle\n",
        "      - name: Target\n",
        "        run: cargo test --lib something\n",
        "      - name: Other\n",
        "        run: cargo test --lib other -- --ignored\n",
        "  second:\n",
        "    steps:\n",
        "      - run: marker-in-second\n",
    );

    let block = workflow_job_block(workflow, "first").expect("first job block");
    assert!(
        !block.contains("marker-in-second"),
        "the block leaked into the next job, so job-scoped assertions would prove nothing"
    );
    assert_eq!(
        step_run_command(&block, "Target"),
        Some("cargo test --lib something"),
        "the reader must return the named step's own command"
    );
    assert!(
        !step_run_command(&block, "Target")
            .expect("target command")
            .contains("--ignored"),
        "the reader must not pick up the preceding comment's `--ignored`, nor the FOLLOWING \
         step's -- matching either is how a wiring check keeps passing after the flag is deleted"
    );
    assert_eq!(
        step_run_command(&block, "Absent"),
        None,
        "a step that does not exist must not resolve to a command"
    );
}
