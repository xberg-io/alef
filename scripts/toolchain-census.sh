#!/usr/bin/env sh
#
# Report how many toolchain-dependent fixtures actually executed, and fail when a toolchain this
# platform installed executed none of them.
#
# `src/test_support/toolchain.rs` writes one TSV per test binary into
# `<target-dir>/toolchain-census/`, each row `<toolchain>\t<attempted>\t<executed>\t<skipped>`.
# This script sums them and prints the result. It exists because there is nowhere inside the test
# run to print it from: `libtest` offers no end-of-run hook and captures the stdout and stderr of
# passing tests, so a fixture that skipped itself has no way to say so in a run that is otherwise
# green -- which is exactly the run where it matters.
#
# The check is deliberately about cardinality, not about the toolchain being on PATH. A fixture
# family that was deleted, renamed out of its filter, or `#[ignore]`d reports zero attempts, and
# that fails here just as loudly as a missing toolchain does: both are runs that examined nothing
# while reporting success.
#
# Usage:
#   scripts/toolchain-census.sh [--dir <census-dir>] [--require <toolchain>]...
#
# `--require` names a toolchain the caller guarantees is installed on this platform, so anything
# other than "every attempt executed" is a failure. Toolchains not named are reported with their
# counts and never fail the run.

set -eu

census_dir="target/toolchain-census"
required=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dir)
      [ $# -ge 2 ] || { echo "toolchain-census: --dir needs a value" >&2; exit 2; }
      census_dir="$2"
      shift 2
      ;;
    --require)
      [ $# -ge 2 ] || { echo "toolchain-census: --require needs a value" >&2; exit 2; }
      required="$required $2"
      shift 2
      ;;
    *)
      echo "toolchain-census: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
done

echo "alef toolchain fixture census -- $census_dir"

if [ -d "$census_dir" ]; then
  # `find`, not a glob, so an empty directory produces no rows rather than a literal `*.tsv`.
  find "$census_dir" -name '*.tsv' -type f -exec cat {} +
fi | awk -v required="$required" '
  BEGIN {
    split(required, wanted, " ")
    for (index_ in wanted) {
      if (wanted[index_] != "") {
        is_required[wanted[index_]] = 1
        seen[wanted[index_]] = 1
      }
    }
  }
  NF == 4 {
    attempted[$1] += $2
    executed[$1] += $3
    skipped[$1] += $4
    seen[$1] = 1
  }
  END {
    failures = 0
    count = 0
    for (name in seen) { count++ }
    if (count == 0) {
      print "  (no toolchain-gated fixture reported in; nothing was measured)"
    }
    for (name in seen) {
      total = attempted[name] + 0
      ran = executed[name] + 0
      missed = skipped[name] + 0
      status = is_required[name] ? "required" : "optional"
      printf "  %-10s %2d of %2d fixtures executed (%d skipped) [%s]", name, ran, total, missed, status
      if (is_required[name] && ran == 0) {
        printf "  <-- FAILED: nothing ran\n"
        failures++
      } else if (is_required[name] && missed > 0) {
        printf "  <-- FAILED: %d skipped on a platform that installs %s\n", missed, name
        failures++
      } else if (ran == 0) {
        printf "  <-- NOT RUN: %s is absent, so these fixtures verified nothing\n", name
      } else {
        printf "  ok\n"
      }
    }
    if (failures > 0) {
      printf "\ntoolchain-census: %d required toolchain(s) executed fewer fixtures than this\n", failures
      print "platform guarantees. A green test run that skipped these examined nothing; treat it"
      print "as a red one. Install the toolchain, or stop declaring it required for this platform."
      exit 1
    }
  }
'
