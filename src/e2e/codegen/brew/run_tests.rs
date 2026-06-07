use crate::core::hash::{self, CommentStyle};
use std::fmt::Write as FmtWrite;

/// Render the main `run_tests.sh` runner script.
pub(super) fn render_run_tests(categories: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# shellcheck disable=SC1091");
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
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
    let _ = writeln!(out, "PASS=0");
    let _ = writeln!(out, "FAIL=0");
    let _ = writeln!(out);

    // Helper functions.
    let _ = writeln!(out, "assert_equals() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$actual\" != \"$expected\" ]; then");
    let _ = writeln!(
        out,
        "    echo \"FAIL [$label]: expected '$expected', got '$actual'\" >&2"
    );
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_contains() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [[ \"$actual\" != *\"$expected\"* ]]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected to contain '$expected'\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_not_empty() {{");
    let _ = writeln!(out, "  local actual=\"$1\" label=\"$2\"");
    let _ = writeln!(out, "  if [ -z \"$actual\" ]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected non-empty value\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_count_min() {{");
    let _ = writeln!(out, "  local count=\"$1\" min=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$count\" -lt \"$min\" ]; then");
    let _ = writeln!(
        out,
        "    echo \"FAIL [$label]: expected at least $min elements, got $count\" >&2"
    );
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_greater_than() {{");
    let _ = writeln!(out, "  local val=\"$1\" threshold=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$(echo \"$val > $threshold\" | bc -l)\" != \"1\" ]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected $val > $threshold\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_greater_than_or_equal() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$actual\" -lt \"$expected\" ]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected $actual >= $expected\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_is_empty() {{");
    let _ = writeln!(out, "  local actual=\"$1\" label=\"$2\"");
    let _ = writeln!(out, "  if [ -n \"$actual\" ]; then");
    let _ = writeln!(
        out,
        "    echo \"FAIL [$label]: expected empty value, got '$actual'\" >&2"
    );
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_less_than() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$actual\" -ge \"$expected\" ]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected $actual < $expected\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_less_than_or_equal() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [ \"$actual\" -gt \"$expected\" ]; then");
    let _ = writeln!(out, "    echo \"FAIL [$label]: expected $actual <= $expected\" >&2");
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "assert_not_contains() {{");
    let _ = writeln!(out, "  local actual=\"$1\" expected=\"$2\" label=\"$3\"");
    let _ = writeln!(out, "  if [[ \"$actual\" == *\"$expected\"* ]]; then");
    let _ = writeln!(
        out,
        "    echo \"FAIL [$label]: expected not to contain '$expected'\" >&2"
    );
    let _ = writeln!(out, "    return 1");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
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

    // Run each test function and track pass/fail.
    let _ = writeln!(out, "run_test() {{");
    let _ = writeln!(out, "  local name=\"$1\"");
    let _ = writeln!(out, "  if \"$name\"; then");
    let _ = writeln!(out, "    echo \"PASS: $name\"");
    let _ = writeln!(out, "    PASS=$((PASS + 1))");
    let _ = writeln!(out, "  else");
    let _ = writeln!(out, "    echo \"FAIL: $name\"");
    let _ = writeln!(out, "    FAIL=$((FAIL + 1))");
    let _ = writeln!(out, "  fi");
    let _ = writeln!(out, "}}");
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
        let script = render_run_tests(&categories);
        assert_shfmt_canonical_indent(&script, "render_run_tests");
        assert!(
            script.lines().any(|l| l.starts_with("  ") && !l.starts_with("   ")),
            "render_run_tests should emit at least one 2-space-indented line; got:\n{script}",
        );
    }
}
