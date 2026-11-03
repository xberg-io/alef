#!/usr/bin/env bash
#MISE description="Set package version (usage: mise run set-version 0.18.0 or 0.18.0-rc.1)"
#USAGE arg "<version>" help="Semver such as 0.18.0 or 0.18.0-rc.1, with or without a leading v"
# shellcheck disable=SC2154 # usage_* are set by mise from the #USAGE lines
set -euo pipefail

version="${usage_version#v}"
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z._-]+)?$'; then
  echo "set-version: expected semver like '0.18.0' or '0.18.0-rc.1' (with or without leading 'v'); got '$usage_version'" >&2
  exit 2
fi
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm -f Cargo.toml.bak
# The two component crates ship in lockstep with the root `alef` crate (one release, one
# `cargo publish` per crate, all at the same version) -- see tests/component_crate_versions_test.rs.
# Their own `[package] version` and the dependency pins that reference them all move together here.
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" crates/alef-component-abi/Cargo.toml
rm -f crates/alef-component-abi/Cargo.toml.bak
sed -i.bak "s/^version = \".*\"/version = \"$version\"/" crates/alef-component-runtime/Cargo.toml
rm -f crates/alef-component-runtime/Cargo.toml.bak
sed -i.bak -E "s#(alef-component-abi = \{ path = \"[^\"]*\", version = \")[^\"]*(\" \})#\1$version\2#" crates/alef-component-runtime/Cargo.toml
rm -f crates/alef-component-runtime/Cargo.toml.bak
sed -i.bak -E "s#(alef-component-(abi|runtime) = \{ path = \"[^\"]*\", version = \")[^\"]*(\" \})#\1$version\3#" Cargo.toml
rm -f Cargo.toml.bak
sed -i.bak "s/^alef_version = \".*\"/alef_version = \"$version\"/" alef.toml
rm -f alef.toml.bak
sed -i.bak -E "s|pub const ALEF_REV: &str = \"v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z._-]+)?\";|pub const ALEF_REV: \&str = \"v$version\";|" src/core/template_versions.rs
rm -f src/core/template_versions.rs.bak
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/schema-task}" \
  cargo run --quiet -- schema --schema-version "$version" --output schemas/alef.schema.json
cargo update --workspace
