#!/usr/bin/env sh
#
# Print the effective toolchain-census directory: `<cargo target_directory>/toolchain-census`.
#
# The Rust census writers resolve the target directory through Cargo metadata.
# Everything that resets or reads that directory
# has to resolve the same path or it silently examines an empty one: `scripts/toolchain-census.sh`
# (its `--dir` default), the mise `test` task's pre-run reset, and
# `.github/workflows/ci.yml`'s "Clear the toolchain fixture census" step all shell out to this
# script instead of assuming `target/toolchain-census` themselves.
#
# `cargo metadata`'s `target_directory` field already resolves `CARGO_TARGET_DIR` for us, so this
# only falls back to `${CARGO_TARGET_DIR:-target}` when cargo itself is unavailable.

set -eu

target_dir=""
if command -v cargo >/dev/null 2>&1; then
  metadata="$(cargo metadata --format-version 1 --no-deps 2>/dev/null)" || metadata=""
  if [ -n "$metadata" ]; then
    # cargo metadata's JSON is emitted on a single line with no whitespace around `:`, so a
    # plain field extraction is enough; jq is not guaranteed to be on PATH here.
    target_dir=$(printf '%s' "$metadata" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    # A Windows target directory arrives JSON-escaped (`D:\\a\\alef\\target`); the doubled
    # backslashes are not a path the Git-for-Windows bash `rm -rf` in CI understands, while a
    # forward-slash spelling is accepted everywhere.
    target_dir=$(printf '%s' "$target_dir" | sed 's|\\\\|/|g')
  fi
fi

if [ -z "$target_dir" ]; then
  target_dir="${CARGO_TARGET_DIR:-target}"
fi

printf '%s/toolchain-census\n' "$target_dir"
