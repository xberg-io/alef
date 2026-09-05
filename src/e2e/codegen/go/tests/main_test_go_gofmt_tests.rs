//! `render_main_test_go` must emit a `gofmt`-canonical file by construction: import order,
//! multi-line `if err != nil { ... }` blocks, and `+`-concatenation spacing all matter because
//! a consumer's own `gofmt -w` (pre-commit hook, editor autosave, CI) silently rewrites any
//! drift after alef has already hashed and stamped the file — the next `alef generate` then
//! sees a byte mismatch, treats the file as hand-edited, and refuses to touch it again. These
//! tests pin the emitted bytes to `gofmt`'s own output so no formatter has to run after the
//! fact for this class of drift.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use super::render_main_test_go;

/// The stdlib import block for the mock-server bootstrap path (`needs_mock_server_bootstrap =
/// true`) must already be gofmt's alphabetical-by-path order: bufio, encoding/json, fmt, io,
/// net/http, os, os/exec, path/filepath, runtime, strings, testing, time. Verified against a
/// real `gofmt -l` on this exact byte sequence during development of this fix.
#[test]
fn mock_server_bootstrap_imports_are_gofmt_sorted() {
    let out = render_main_test_go("testing_data", true, false, &Default::default());
    let import_paths: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "import (")
        .skip(1)
        .take_while(|l| *l != ")")
        .map(|l| l.trim().trim_matches('"'))
        .collect();
    let mut sorted = import_paths.clone();
    sorted.sort_unstable();
    assert_eq!(
        import_paths, sorted,
        "mock-server bootstrap import block must already be gofmt-sorted; got:\n{out}"
    );
}

/// Same rule, harness-spawn path (`has_http_fixtures = true`, no mock-server bootstrap): fmt,
/// io, net, os, os/exec, path/filepath, runtime, testing, time.
#[test]
fn harness_spawn_imports_are_gofmt_sorted() {
    let out = render_main_test_go("testing_data", false, true, &Default::default());
    let import_paths: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "import (")
        .skip(1)
        .take_while(|l| *l != ")")
        .map(|l| l.trim().trim_matches('"'))
        .collect();
    let mut sorted = import_paths.clone();
    sorted.sort_unstable();
    assert_eq!(
        import_paths, sorted,
        "harness-spawn import block must already be gofmt-sorted; got:\n{out}"
    );
}

/// Negative control: the plain path (neither mock-server bootstrap nor HTTP fixtures) has an
/// import list that was already in alphabetical order before this fix (`os`, `path/filepath`,
/// `runtime`, `testing`) — sorting must not disturb it.
#[test]
fn plain_path_imports_are_unchanged_by_sorting() {
    let out = render_main_test_go("testing_data", false, false, &Default::default());
    let import_paths: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "import (")
        .skip(1)
        .take_while(|l| *l != ")")
        .map(|l| l.trim().trim_matches('"'))
        .collect();
    assert_eq!(
        import_paths,
        vec!["os", "path/filepath", "runtime", "testing"],
        "got:\n{out}"
    );
}

/// `if err != nil { panic(err) }` must never be emitted on one line — gofmt always splits a
/// braced block body onto its own line.
#[test]
fn err_check_panics_are_never_single_line() {
    let out = render_main_test_go("testing_data", true, false, &Default::default());
    assert!(
        !out.contains("{ panic(err) }"),
        "if-err-panic must be split across lines, not emitted as a one-liner; got:\n{out}"
    );
}

/// `go func() { for ... { } }()` must never be emitted on one line — gofmt always splits a
/// braced `for` body inside a closure onto its own lines.
#[test]
fn drain_goroutine_is_never_single_line() {
    let out = render_main_test_go("testing_data", true, false, &Default::default());
    assert!(
        !out.contains("go func() { for scanner.Scan() { } }()"),
        "drain goroutine must be split across lines, not emitted as a one-liner; got:\n{out}"
    );
}

/// String concatenation (`"KEY=" + v`) must be emitted with gofmt's spacing (`"KEY="+v`, no
/// spaces around `+`) when the left operand is a short literal beside a single identifier.
#[test]
fn env_concat_has_no_spaces_around_plus() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("SOME_VAR".to_string(), "1".to_string());
    let out = render_main_test_go("testing_data", true, false, &env);
    assert!(
        out.contains("\"SOME_VAR=\"+v"),
        "env concatenation must have no spaces around `+`; got:\n{out}"
    );
    assert!(
        !out.contains("\"SOME_VAR=\" + v"),
        "env concatenation must not have spaces around `+`; got:\n{out}"
    );
}

/// Full round-trip against the real `gofmt` binary for both branches, mirroring
/// `e2e::codegen::go::snippet`'s `snippet_matches_gofmt_when_available`: skips (rather than
/// failing) when `gofmt` is not on `PATH`, so the structural assertions above still run on
/// every machine and this test only exercises the real formatter where it can.
#[test]
fn rendered_main_test_go_matches_gofmt_when_available() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("SOME_VAR".to_string(), "1".to_string());

    for out in [
        render_main_test_go("testing_data", true, false, &env),
        render_main_test_go("testing_data", false, true, &Default::default()),
        render_main_test_go("testing_data", false, false, &Default::default()),
    ] {
        assert_gofmt_no_op(&out);
    }
}

fn assert_gofmt_no_op(code: &str) {
    use std::io::Write as _;

    let Ok(mut child) = std::process::Command::new("gofmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(code.as_bytes())
        .expect("write Go source");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout).expect("gofmt output is UTF-8"),
        code,
        "render_main_test_go output must already be gofmt-canonical"
    );
}
