//! Coverage for `main_test.go` / harness generation: mock-server bootstrap vs
//! harness-spawn import selection, the `MOCK_SERVER_NO_STDIN_WATCH` env var,
//! the `exitAfterDefer`-safe helper split, env-setup rendering, and the reserved-keyword
//! import-alias escape for `render_harness_main`.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::config::E2eConfig;

use super::{render_env_setup, render_main_test_go};

#[test]
fn main_test_go_http_fixtures_omits_net_http_and_strings_imports() {
    // When needs_mock_server_bootstrap=false (HTTP-fixtures harness path), the bootstrap uses
    // net.DialTimeout + io.Copy for readiness polling.
    // "net/http" and "strings" are NOT referenced, so they must not be imported.
    let out = render_main_test_go("testing_data", false, true, &Default::default());
    assert!(
        !out.contains("\t\"net/http\""),
        "main_test.go (http-fixtures harness path) must NOT import net/http; got:\n{out}"
    );
    assert!(
        !out.contains("\t\"strings\""),
        "main_test.go (http-fixtures harness path) must NOT import strings; got:\n{out}"
    );
    // But it must still import "net" and "io" for the harness path
    assert!(out.contains("\t\"net\""), "must import net; got:\n{out}");
    assert!(out.contains("\t\"io\""), "must import io; got:\n{out}");
}

#[test]
fn main_test_go_non_http_fixtures_includes_net_http_and_strings_imports() {
    // When needs_mock_server_bootstrap=true (mock-server path for function-call fixtures),
    // http.Get (net/http) and strings.HasPrefix/TrimPrefix are used — both must be imported.
    let out = render_main_test_go("testing_data", true, false, &Default::default());
    assert!(
        out.contains("\t\"net/http\""),
        "main_test.go (mock-server bootstrap path) must import net/http; got:\n{out}"
    );
    assert!(
        out.contains("\t\"strings\""),
        "main_test.go (mock-server bootstrap path) must import strings; got:\n{out}"
    );
    // io is now needed for the runTests helper's io.ReadCloser parameter
    assert!(
        out.contains("\t\"io\""),
        "main_test.go (mock-server bootstrap path) must import io for helper; got:\n{out}"
    );
    // And must NOT import "net" (that's http-fixtures harness path only)
    assert!(
        !out.contains("\t\"net\""),
        "main_test.go (mock-server bootstrap path) must NOT import net; got:\n{out}"
    );
}

/// The generated TestMain must set `MOCK_SERVER_NO_STDIN_WATCH=1` on the
/// mock-server subprocess so the server does not treat stdin EOF (from
/// Go's exec.Command defaulting Stdin to /dev/null) as a shutdown signal.
#[test]
fn main_test_go_sets_mock_server_no_stdin_watch_env() {
    let out = render_main_test_go("testing_data", true, false, &Default::default());
    assert!(
        out.contains("MOCK_SERVER_NO_STDIN_WATCH=1"),
        "main_test.go must set MOCK_SERVER_NO_STDIN_WATCH=1 on the mock-server subprocess; got:\n{out}"
    );
    // Must appear as cmd.Env assignment, not as a stray string in a comment.
    assert!(
        out.contains("cmdEnv := os.Environ()")
            && out.contains("cmdEnv = append(cmdEnv, \"MOCK_SERVER_NO_STDIN_WATCH=1\")")
            && out.contains("cmd.Env = cmdEnv"),
        "main_test.go must build cmdEnv before assigning cmd.Env; got:\n{out}"
    );
}

/// Regression test: TestMain must not trigger the 'exitAfterDefer' linter error.
/// This is avoided by extracting deferred cleanup into helper functions that
/// return int before os.Exit is called.
#[test]
fn main_test_go_avoids_exitafterdefer_linter_error() {
    // Mock-server bootstrap path: must have a runTests helper function
    let mock_server_out = render_main_test_go("testing_data", true, false, &Default::default());
    assert!(
        mock_server_out.contains("func runTests(m *testing.M, cmd *exec.Cmd, stdout io.ReadCloser) int"),
        "mock-server bootstrap path must emit runTests helper; got:\n{mock_server_out}"
    );
    assert!(
        mock_server_out.contains("code := runTests(m, cmd, stdout)"),
        "TestMain must call runTests to get int, not inline defer; got:\n{mock_server_out}"
    );
    assert!(
        mock_server_out.contains("os.Exit(code)"),
        "os.Exit must be called AFTER runTests returns; got:\n{mock_server_out}"
    );
    // Must NOT have os.Exit inside a function with defers still in scope
    assert!(
        !mock_server_out.contains("defer func() { _ = cmd.Process.Kill() }()")
            || mock_server_out.contains("func runTests"),
        "defers must be moved out of TestMain scope; got:\n{mock_server_out}"
    );

    // Harness-spawn path: must have runHarnessTests helper
    let harness_out = render_main_test_go("testing_data", false, true, &Default::default());
    assert!(
        harness_out.contains(
            "func runHarnessTests(m *testing.M, cmd *exec.Cmd, stdin io.WriteCloser, stdout io.ReadCloser) int"
        ),
        "harness-spawn path must emit runHarnessTests helper; got:\n{harness_out}"
    );
    assert!(
        harness_out.contains("code := runHarnessTests(m, cmd, stdin, stdout)"),
        "TestMain must call runHarnessTests to get int; got:\n{harness_out}"
    );
    assert!(
        harness_out.contains("os.Exit(code)"),
        "os.Exit must be called AFTER runHarnessTests returns; got:\n{harness_out}"
    );
}

#[test]
fn render_env_setup_empty_returns_empty_string() {
    let env = std::collections::BTreeMap::new();
    let out = render_env_setup(&env);
    assert_eq!(out, "", "empty env should produce empty output");
}

#[test]
fn render_env_setup_single_var_contains_key_and_value() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
    let out = render_env_setup(&env);
    assert!(
        out.contains("E2E_ALLOW_PRIVATE_NETWORK"),
        "output should contain env var name: {out}"
    );
    assert!(out.contains("true"), "output should contain env var value: {out}");
    assert!(
        out.contains("os.LookupEnv"),
        "output should use os.LookupEnv for setdefault behavior: {out}"
    );
    assert!(out.contains("os.Setenv"), "output should call os.Setenv: {out}");
}

#[test]
fn render_env_setup_multiple_vars_are_sorted() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("ZEBRA".to_string(), "value1".to_string());
    env.insert("APPLE".to_string(), "value2".to_string());
    env.insert("BANANA".to_string(), "value3".to_string());
    let out = render_env_setup(&env);
    let apple_idx = out.find("APPLE").expect("should contain APPLE");
    let banana_idx = out.find("BANANA").expect("should contain BANANA");
    let zebra_idx = out.find("ZEBRA").expect("should contain ZEBRA");
    assert!(
        apple_idx < banana_idx && banana_idx < zebra_idx,
        "env vars should be sorted alphabetically: {out}"
    );
}

#[test]
fn render_main_test_go_includes_env_setup_at_start() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("TEST_VAR".to_string(), "test_value".to_string());
    let out = render_main_test_go("test_documents", false, false, &env);

    let dir_idx = out
        .find("dir := filepath.Dir(filename)")
        .expect("should contain dir assignment");
    let test_var_idx = out.find("TEST_VAR").expect("should contain TEST_VAR");

    assert!(dir_idx < test_var_idx, "env setup should come after dir initialization");
}

/// A module path whose last segment is a Go reserved keyword (e.g. `.../packages/go`)
/// must not be emitted verbatim as an import alias — `import go "..."` is a compile
/// error because `go` is a reserved word. The alias is escaped to `go_`.
#[test]
fn render_harness_uses_escaped_alias_for_reserved_keyword_module_segment() {
    let out = super::render_harness_main(&E2eConfig::default(), &[], "github.com/example/acme/packages/go");
    assert!(
        !out.contains("go \"github.com/example/acme/packages/go\""),
        "reserved keyword `go` must not be used as a verbatim import alias, got:\n{out}"
    );
    assert!(
        out.contains("go_ \"github.com/example/acme/packages/go\""),
        "reserved keyword segment `go` should be escaped to alias `go_`, got:\n{out}"
    );
    assert!(
        out.contains("go_.NewApp()"),
        "escaped alias `go_` should be used as the package qualifier, got:\n{out}"
    );
}

/// `env` is a `BTreeMap`, so iteration is always in key order regardless of insertion order or
/// process-seeded hashing. Regression coverage for a real bug: before `E2eConfig::env` was typed
/// as a `BTreeMap`, this loop ran over a `HashMap` whose randomly-seeded iteration order emitted
/// the forwarding blocks differently on every run -- irreproducible generation that flapped the
/// freshness gate (the file's `alef:hash:` changed with no source change behind it). Two
/// consecutive generations of one consumer's `main_test.go` swapped two blocks. Asserted as a
/// full sorted sequence rather than a single pair so a regression cannot satisfy it by luck. ~keep
#[test]
fn main_test_go_forwards_env_vars_in_sorted_order() {
    let env: std::collections::BTreeMap<String, String> = [
        ("ZULU_LAST", "1"),
        ("ALPHA_FIRST", "2"),
        ("MIKE_MIDDLE", "3"),
        ("BRAVO_SECOND", "4"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let out = render_main_test_go("testing_data", true, false, &env);

    let positions: Vec<usize> = ["ALPHA_FIRST", "BRAVO_SECOND", "MIKE_MIDDLE", "ZULU_LAST"]
        .iter()
        .map(|key| {
            out.find(&format!("cmdEnv = append(cmdEnv, \"{key}=\"+v)"))
                .unwrap_or_else(|| panic!("main_test.go must forward {key}; got:\n{out}"))
        })
        .collect();

    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "main_test.go must forward env vars in sorted key order so generation is reproducible, \
         got offsets {positions:?} in:\n{out}"
    );
}
