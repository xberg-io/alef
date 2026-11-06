#!/usr/bin/env bash
set -euo pipefail

check_log() {
  local log_file="$1" observed
  grep -Eq '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log_file" || {
    echo "Swift ownership gate expected one executed test with none ignored" >&2
    return 1
  }
  observed="$(awk '/^owned_vector_verified=/ { count++ } END { print count + 0 }' "$log_file")"
  [[ "$observed" -eq 5 ]] || {
    echo "Swift ownership gate expected five runtime cases, observed $observed" >&2
    return 1
  }
  for runtime_case in unique moving-unique clone-reuse clone-async-reuse moving-state; do
    grep -Fxq "owned_vector_verified=$runtime_case" "$log_file" || return 1
  done
}

if [[ "${1:-}" == "--check-log" ]]; then
  [[ "$#" -eq 2 ]] || exit 2
  check_log "$2"
  exit
fi
[[ "$#" -eq 0 ]] || exit 2

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
compiler="${SWIFT_EXEC:-$(command -v swiftc)}"
echo "Swift ownership fixture compiler: $compiler"
"$compiler" --version
if command -v xcrun >/dev/null 2>&1; then
  xcrun --show-sdk-path
fi
log_file="$(mktemp)"
trap 'rm -f "$log_file"' EXIT
filter='backends::swift::gen_rust_crate::owned_inputs_tests::owned_vector_inputs_support_repeated_clone_handles_and_checked_moves'
MISE_TASK_OUTPUT=interleave mise run test:run -- --lib "$filter" -- --ignored --exact --nocapture --test-threads=1 2>&1 | tee "$log_file"
check_log "$log_file"
echo "Swift ownership gate verified one test and five healthy runtime cases"
