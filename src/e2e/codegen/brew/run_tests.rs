use crate::core::hash::{self, CommentStyle};
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;

/// Emit a bash snippet that exports every `[e2e.env]` entry using `setdefault`
/// semantics: each var is only set when not already present in the parent
/// environment. Returns an empty string when the map is empty. Keys are sorted
/// alphabetically for deterministic output.
pub(super) fn render_env_block(env: &BTreeMap<String, String>) -> String {
    if env.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(out, "# Suite-level environment defaults from [e2e.env]. Each entry");
    let _ = writeln!(out, "# uses setdefault semantics: only applied when not already set.");
    // `env` is a `BTreeMap`, so this already iterates in key order -- no separate sort
    // needed to keep generation reproducible. ~keep
    for (key, value) in env {
        let _ = writeln!(out, "if [[ -z \"${{{key}+x}}\" ]]; then");
        let _ = writeln!(out, "  export {key}={}", crate::core::config::shell::quote_word(value));
        let _ = writeln!(out, "fi");
    }
    let _ = writeln!(out);
    out
}

/// Render the main `run_tests.sh` runner script.
pub(super) fn render_run_tests(categories: &[String], env: &BTreeMap<String, String>, binary_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# shellcheck disable=SC1091");
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let env_block = render_env_block(env);
    if !env_block.is_empty() {
        out.push_str(&env_block);
    }
    let _ = writeln!(out, "# Auto-spawn mock-server if MOCK_SERVER_URL is not pre-set.");
    let _ = writeln!(
        out,
        "# Mirrors the C test_app Makefile's run_with_mock_server macro: builds the"
    );
    let _ = writeln!(
        out,
        "# fixture-driven mock-server from ../rust/ on demand, launches it in the"
    );
    let _ = writeln!(
        out,
        "# background, harvests MOCK_SERVER_URL + MOCK_SERVERS from its stdout, and"
    );
    let _ = writeln!(
        out,
        "# tears it down on exit. Without this the `task test-apps:smoke:brew` entry"
    );
    let _ = writeln!(
        out,
        "# point — which just calls `bash run_tests.sh` — fails at the require-check"
    );
    let _ = writeln!(
        out,
        "# above because nothing else in the smoke task spawns a mock-server."
    );
    let _ = writeln!(out, "if [ -z \"${{MOCK_SERVER_URL:-}}\" ]; then");
    let _ = writeln!(
        out,
        "  MOCK_SERVER_BIN=\"${{MOCK_SERVER_BIN:-../rust/target/release/mock-server}}\""
    );
    let _ = writeln!(
        out,
        "  MOCK_SERVER_MANIFEST=\"${{MOCK_SERVER_MANIFEST:-../rust/Cargo.toml}}\""
    );
    let _ = writeln!(out, "  FIXTURES_DIR=\"${{FIXTURES_DIR:-../../fixtures}}\"");
    let _ = writeln!(out, "  if [ ! -x \"$MOCK_SERVER_BIN\" ]; then");
    let _ = writeln!(
        out,
        "    echo \"Building mock-server from $MOCK_SERVER_MANIFEST...\" >&2"
    );
    let _ = writeln!(
        out,
        "    cargo build --release --manifest-path \"$MOCK_SERVER_MANIFEST\" --bin mock-server >&2"
    );
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "  rm -f mock_server.stdout");
    let _ = writeln!(out, "  : > mock_server.stdout");
    let _ = writeln!(
        out,
        "  \"$MOCK_SERVER_BIN\" \"$FIXTURES_DIR\" >mock_server.stdout 2>&1 &"
    );
    let _ = writeln!(out, "  __MOCK_PID=$!");
    let _ = writeln!(
        out,
        "  trap '[ -n \"${{__MOCK_PID:-}}\" ] && kill \"$__MOCK_PID\" 2>/dev/null || true' EXIT"
    );
    let _ = writeln!(out, "  for _i in $(seq 1 200); do");
    let _ = writeln!(
        out,
        "    if grep -q '^MOCK_SERVER_URL=' mock_server.stdout 2>/dev/null; then"
    );
    let _ = writeln!(out, "      break");
    let _ = writeln!(out, "    fi");
    let _ = writeln!(out, "    sleep 0.05");
    let _ = writeln!(out, "  done");
    let _ = writeln!(
        out,
        "  if ! grep -q '^MOCK_SERVER_URL=' mock_server.stdout 2>/dev/null; then"
    );
    let _ = writeln!(
        out,
        "    echo 'error: mock-server did not emit MOCK_SERVER_URL within 10s' >&2"
    );
    let _ = writeln!(out, "    cat mock_server.stdout >&2 || true");
    let _ = writeln!(out, "    exit 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(
        out,
        "  MOCK_SERVER_URL=\"$(grep '^MOCK_SERVER_URL=' mock_server.stdout | tail -1 | cut -d= -f2-)\""
    );
    let _ = writeln!(out, "  export MOCK_SERVER_URL");
    let _ = writeln!(
        out,
        "  if grep -q '^MOCK_SERVERS=' mock_server.stdout 2>/dev/null; then"
    );
    let _ = writeln!(
        out,
        "    MOCK_SERVERS=\"$(grep '^MOCK_SERVERS=' mock_server.stdout | tail -1 | cut -d= -f2-)\""
    );
    let _ = writeln!(out, "    export MOCK_SERVERS");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(out, "# MOCK_SERVER_URL must be set to the base URL of the mock server.");
    let _ = writeln!(out, ": \"${{MOCK_SERVER_URL:?MOCK_SERVER_URL is required}}\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Verify that jq is available.");
    let _ = writeln!(out, "if ! command -v jq &>/dev/null; then");
    let _ = writeln!(out, "  echo 'error: jq is required but not found in PATH' >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    // The brew test_app exercises the formula-installed CLI binary; emit a
    // pre-flight check so the failure is reported as "install via brew" rather
    // than a stream of opaque `command not found` errors from each category
    // test script. CLI binary name is the resolved package name for the brew
    // language entry; require exactly that binary on PATH so another installed
    // CLI cannot mask a missing configured binary.
    let _ = writeln!(out, "# Verify the brew-installed CLI is on PATH.");
    let _ = writeln!(
        out,
        "BINARY_NAME={}",
        crate::core::config::shell::quote_word(binary_name)
    );
    let _ = writeln!(out, "if ! command -v \"$BINARY_NAME\" &>/dev/null; then");
    let _ = writeln!(
        out,
        "  echo 'error: brew test_app requires the Homebrew formula to be installed' >&2"
    );
    let _ = writeln!(out, "  printf '       run: brew install %s\\n' \"$BINARY_NAME\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    // Harness core: pass/fail counters, the assertion helpers the generated
    // category tests call, and `run_test`. Emitted from a template because it is a
    // single logical shell unit with no per-suite parameters.
    out.push_str(&crate::e2e::template_env::render(
        "brew/harness.sh.jinja",
        minijinja::context! {},
    ));
    let _ = writeln!(out);

    // Source per-category files.
    let script_dir = r#"SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)""#;
    let _ = writeln!(out, "{script_dir}");
    let _ = writeln!(out);
    for category in categories {
        let _ = writeln!(out, "# shellcheck source=test_{category}.sh");
        let _ = writeln!(out, "source \"$SCRIPT_DIR/test_{category}.sh\"");
    }
    let _ = writeln!(out);

    // Gather all test function names from category files then call them.
    // We enumerate them at code-generation time so the runner doesn't need
    // introspection at runtime.
    let _ = writeln!(out, "# Run all generated test functions.");
    for category in categories {
        let _ = writeln!(out, "# Category: {category}");
        // We emit a placeholder comment — the actual list is per-category.
        // The run_test calls are emitted inline below based on known IDs.
        let _ = writeln!(out, "run_tests_{category}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"\"");
    let _ = writeln!(out, "echo \"Results: $PASS passed, $FAIL failed\"");
    let _ = writeln!(out, "[ \"$FAIL\" -eq 0 ]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every leading-whitespace prefix in an emitted shell line must be a
    /// multiple of 2 spaces. shfmt's default indent step rewrites any other
    /// indent step, which then causes the alef-emitted scripts to be rewritten
    /// by pre-commit hooks on every project run.
    fn assert_shfmt_canonical_indent(script: &str, context: &str) {
        for (lineno, line) in script.lines().enumerate() {
            let leading_spaces = line.chars().take_while(|c| *c == ' ').count();
            assert!(
                leading_spaces.is_multiple_of(2),
                "{context}: line {lineno} has {leading_spaces}-space indent (must be a multiple of 2 for shfmt compatibility): {line:?}",
            );
        }
    }

    #[test]
    fn render_run_tests_uses_two_space_indent() {
        let categories = vec!["auth".to_string(), "crawl".to_string()];
        let script = render_run_tests(&categories, &BTreeMap::new(), "sample-cli");
        assert_shfmt_canonical_indent(&script, "render_run_tests");
        assert!(
            script.lines().any(|l| l.starts_with("  ") && !l.starts_with("   ")),
            "render_run_tests should emit at least one 2-space-indented line; got:\n{script}",
        );
    }

    /// Assertion values are extracted with `jq -r`, which renders JSON null and empty
    /// containers as the literal text "null", "[]" and "{}". A bare `-z` test therefore
    /// passed on every one of them while still reading as coverage. A legitimate `0` or
    /// `false` also renders as text and must keep passing.
    #[test]
    fn not_empty_for_brew_rejects_the_json_renderings_of_empty_values() {
        let script = render_run_tests(&["auth".to_string()], &BTreeMap::new(), "sample-cli");
        assert!(
            script.contains(
                "  if [ -z \"$actual\" ] || [ \"$actual\" = \"null\" ] || [ \"$actual\" = \"[]\" ] \
                 || [ \"$actual\" = \"{}\" ]; then"
            ),
            "got: {script}"
        );
    }

    #[test]
    fn render_env_block_emits_setdefault_with_sorted_keys() {
        let mut env = BTreeMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        env.insert("ALEF_FOO".to_string(), "bar".to_string());
        let block = render_env_block(&env);
        assert!(block.contains("export ALEF_FOO='bar'"), "got: {block}");
        assert!(
            block.contains("export E2E_ALLOW_PRIVATE_NETWORK='true'"),
            "got: {block}"
        );
        assert!(block.contains("export ALEF_FOO"), "got: {block}");
        assert!(block.contains("export E2E_ALLOW_PRIVATE_NETWORK"), "got: {block}");
        let alef_pos = block.find("ALEF_FOO").unwrap();
        let e2e_pos = block.find("E2E_ALLOW_PRIVATE_NETWORK").unwrap();
        assert!(alef_pos < e2e_pos, "keys must be sorted alphabetically; got: {block}");
    }

    #[test]
    fn render_env_block_empty_when_no_env_configured() {
        let env = BTreeMap::new();
        assert_eq!(render_env_block(&env), "");
    }

    #[test]
    fn render_run_tests_omits_env_block_when_env_empty() {
        let categories = vec!["smoke".to_string()];
        let script = render_run_tests(&categories, &BTreeMap::new(), "sample-cli");
        assert!(
            !script.contains("Suite-level environment defaults"),
            "no env block when env empty; got: {script}"
        );
    }

    /// Regression: the brew test_app must check that the formula-installed CLI is
    /// on PATH before invoking it from category tests. Without this preflight the
    /// failure surfaces as a cascade of `sample-cli: command not found` lines from
    /// each test, drowning the actionable signal (run `brew install …`).
    #[test]
    fn render_run_tests_emits_brew_cli_preflight_check() {
        let categories = vec!["smoke".to_string()];
        let script = render_run_tests(&categories, &BTreeMap::new(), "sample-cli");
        assert!(
            script.contains("Verify the brew-installed CLI is on PATH"),
            "expected brew CLI preflight check; got:\n{script}"
        );
        assert!(
            script.contains("brew install %s") && script.contains("BINARY_NAME='sample-cli'"),
            "expected install instruction in brew CLI preflight; got:\n{script}"
        );
        // The check must require only the configured binary name so another
        // installed CLI cannot mask a missing configured binary.
        assert!(
            script.contains("BINARY_NAME='sample-cli'") && script.contains("command -v \"$BINARY_NAME\""),
            "expected single-binary preflight; got:\n{script}"
        );
        assert!(
            !script.contains("command -v sibling-cli "),
            "preflight must not OR with sibling CLI; got:\n{script}"
        );
    }

    #[test]
    fn render_run_tests_preflight_uses_parameterized_binary_name() {
        let categories = vec!["smoke".to_string()];
        let script = render_run_tests(&categories, &BTreeMap::new(), "mytool");
        assert!(
            script.contains("BINARY_NAME='mytool'") && script.contains("command -v \"$BINARY_NAME\""),
            "expected preflight to use parameterized binary; got:\n{script}"
        );
        assert!(
            script.contains("brew install %s") && script.contains("\"$BINARY_NAME\""),
            "expected install hint to use parameterized binary; got:\n{script}"
        );
        assert!(
            !script.contains("sample-cli") && !script.contains("sibling-cli"),
            "preflight must not leak hardcoded sibling names; got:\n{script}"
        );
    }

    #[test]
    fn render_run_tests_includes_env_block_when_env_configured() {
        let mut env = BTreeMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        let categories = vec!["smoke".to_string()];
        let script = render_run_tests(&categories, &env, "sample-cli");
        assert!(
            script.contains("export E2E_ALLOW_PRIVATE_NETWORK='true'"),
            "got: {script}"
        );
        assert!(script.contains("export E2E_ALLOW_PRIVATE_NETWORK"), "got: {script}");
        // Env block must precede the MOCK_SERVER_URL bootstrap so the binding's
        // first call already sees the configured environment.
        let env_pos = script.find("${E2E_ALLOW_PRIVATE_NETWORK").unwrap();
        let mock_pos = script.find("MOCK_SERVER_URL").unwrap();
        assert!(env_pos < mock_pos, "env block must precede mock-server bootstrap");
    }

    #[test]
    fn render_env_block_keeps_hostile_values_in_single_quoted_data() {
        let mut env = BTreeMap::new();
        env.insert(
            "ALEF_EXACT".to_string(),
            "literal'; touch /tmp/alef-brew-env; #".to_string(),
        );
        let block = render_env_block(&env);
        assert!(
            block.contains("export ALEF_EXACT='literal'\\''; touch /tmp/alef-brew-env; #'"),
            "got: {block}"
        );
    }

    /// Executing the emitted runner is the only way to prove the harness reports
    /// a failing assertion, because the bug being guarded against was invisible
    /// in the emitted text: `if "$name"; then` reads like a correct check and
    /// silently disables errexit for the test function's whole body.
    ///
    /// Unix-only: the generated harness is bash, and the stubs it needs on PATH
    /// are chmod'd executables.
    #[cfg(unix)]
    fn run_generated_harness(category_body: &str) -> (bool, String) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        // The runner's preflight requires both `jq` and the configured CLI on
        // PATH before it runs a single test; stub both so the harness itself is
        // what the test exercises.
        for stub in ["jq", "sample-cli"] {
            let path = root.join(stub);
            std::fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").expect("write stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
        }
        let script = render_run_tests(&["smoke".to_string()], &BTreeMap::new(), "sample-cli");
        std::fs::write(root.join("run_tests.sh"), &script).expect("write runner");
        std::fs::write(root.join("test_smoke.sh"), category_body).expect("write category file");

        let path_var = format!(
            "{}:{}",
            root.display(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string())
        );
        let output = std::process::Command::new("bash")
            .arg("run_tests.sh")
            .current_dir(root)
            .env("PATH", path_var)
            // Pre-set so the runner skips the mock-server bootstrap; no test here
            // actually reaches out over the network.
            .env("MOCK_SERVER_URL", "http://127.0.0.1:1")
            .output()
            .expect("bash runs the generated runner");
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        (output.status.success(), combined)
    }

    #[cfg(unix)]
    fn three_assertion_category(first_expected: &str) -> String {
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\n\n\
             test_three_assertions() {{\n\
             \x20 assert_equals \"2\" \"{first_expected}\" 'first'\n\
             \x20 assert_equals 'ok' 'ok' 'second'\n\
             \x20 assert_equals 'ok' 'ok' 'third'\n\
             }}\n\n\
             run_tests_smoke() {{\n\
             \x20 run_test test_three_assertions\n\
             }}\n"
        )
    }

    /// Regression: a test whose FIRST of three assertions fails must be reported
    /// as FAIL and must fail the runner. `run_test` used to invoke the test
    /// function as an `if` condition, which disables errexit for the whole call,
    /// so the function's status was just its last command's — every assertion but
    /// the last could fail while the suite reported PASS and exited 0.
    #[cfg(unix)]
    #[test]
    fn runner_fails_when_only_the_first_of_three_assertions_fails() {
        let (success, output) = run_generated_harness(&three_assertion_category("99"));
        assert!(
            output.contains("FAIL [first]: expected '99', got '2'"),
            "the failing assertion must report itself; got:\n{output}"
        );
        assert!(
            output.contains("FAIL: test_three_assertions"),
            "the test must be reported as FAIL; got:\n{output}"
        );
        assert!(
            output.contains("Results: 0 passed, 1 failed"),
            "the tally must count the test as failed; got:\n{output}"
        );
        assert!(!success, "the runner must exit non-zero; got:\n{output}");
    }

    /// The other half of the guard: the fix must not turn every test red. A
    /// three-assertion test that passes all three still reports PASS and exits 0.
    #[cfg(unix)]
    #[test]
    fn runner_passes_when_every_assertion_passes() {
        let (success, output) = run_generated_harness(&three_assertion_category("2"));
        assert!(
            output.contains("PASS: test_three_assertions"),
            "an all-passing test must be reported as PASS; got:\n{output}"
        );
        assert!(
            output.contains("Results: 1 passed, 0 failed"),
            "the tally must count the test as passed; got:\n{output}"
        );
        assert!(success, "the runner must exit zero; got:\n{output}");
    }
}
