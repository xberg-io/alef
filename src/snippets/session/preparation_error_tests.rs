//! Alef defect #142: a session's `before` hook builds this language's artifacts (`cargo build
//! --release -p <crate>-jni`, `pnpm run build:all`, ...) before any of its snippets can validate.
//! When that hook itself outlives `timeout_secs`, the resulting `Error::Timeout` used to collapse
//! into the exact same opaque `Error::Other` every other preparation failure produces --
//! indistinguishable from a missing manifest or a broken directory once it reached
//! `record_preparation_error`. `SessionPreparationError::ordering` is what lets a caller tell "the
//! build never finished in time" apart from "this session is misconfigured", and the message has
//! to name the ordering problem instead of reading as a bare timeout.

use super::*;
use std::collections::BTreeMap;

#[cfg(unix)]
fn sleep_hook(seconds: u64) -> String {
    format!("sleep {seconds}")
}

/// `timeout /t` refuses to run at all when stdin is not a console -- `run_command` pipes the
/// child's output and CI hands the test binary a redirected stdin, so it exited immediately
/// with "ERROR: Input redirection is not supported" and the hook never outlived the timeout
/// this fixture exists to trigger. `ping` against loopback is the console-free cmd sleep: one
/// packet goes out immediately and each subsequent one waits a second, so `seconds + 1`
/// packets take about `seconds`, whether or not the pings are answered. ~keep
#[cfg(windows)]
fn sleep_hook(seconds: u64) -> String {
    format!("ping -n {} 127.0.0.1", seconds + 1)
}

#[cfg(unix)]
fn failing_hook() -> String {
    "exit 1".to_string()
}

#[cfg(windows)]
fn failing_hook() -> String {
    "cmd /C exit 1".to_string()
}

fn spec_with_before(directory: &Path, before: String) -> SessionSpec {
    SessionSpec {
        language: Language::TypeScript,
        working_directory: directory.to_path_buf(),
        manifest: None,
        before: vec![before],
        env: BTreeMap::new(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: BTreeMap::new(),
    }
}

#[test]
fn a_before_hook_timeout_is_classified_as_an_ordering_problem() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut specs = HashMap::new();
    specs.insert(
        "typescript".to_string(),
        spec_with_before(directory.path(), sleep_hook(2)),
    );

    let prepared = prepare_sessions_isolated(&specs, 1);
    let error = prepared
        .errors
        .get("typescript")
        .expect("a before hook that outlives the timeout must fail preparation");

    assert!(
        error.ordering,
        "a before-hook timeout must be classified as an ordering problem: {}",
        error.message
    );
    assert!(
        error.message.contains("Run `alef build` before validating"),
        "the message must point at the build ordering, not just report the raw timeout: {}",
        error.message
    );
    assert!(
        !error
            .message
            .contains("preparing typescript snippet validation session: preparing"),
        "the ordering message must not double-wrap the underlying timeout error: {}",
        error.message
    );
}

/// The negative control: a `before` hook that runs to completion and then fails on its own terms
/// (not a timeout) is a real configuration problem, not an ordering gap -- it must keep
/// `ordering == false` so it is never folded into the same bucket as an unbuilt artifact.
#[test]
fn a_before_hook_failure_that_is_not_a_timeout_is_not_an_ordering_problem() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut specs = HashMap::new();
    specs.insert(
        "typescript".to_string(),
        spec_with_before(directory.path(), failing_hook()),
    );

    let prepared = prepare_sessions_isolated(&specs, 5);
    let error = prepared
        .errors
        .get("typescript")
        .expect("a before hook that exits non-zero must fail preparation");

    assert!(
        !error.ordering,
        "a before hook that ran to completion and failed on its own terms is not an ordering \
         problem: {}",
        error.message
    );
}
