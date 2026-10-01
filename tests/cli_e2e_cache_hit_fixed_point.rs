//! A cached `alef e2e generate` run must preserve the post-format ownership stamp.
//!
//! The cache-hit path deliberately reruns formatters, but used to omit the hash finalisation
//! that follows formatting on a cache miss. `poly`'s uncomment pass removes the generated
//! `alef:hash:` line, so unchanged consumers alternated forever: hit (stamp removed), miss
//! (output regenerated and stamped), hit. With ownership-sensitive fixtures this also made
//! otherwise eligible tests disappear on every other invocation.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create source directory");
    fs::create_dir_all(root.join("fixtures/contract")).expect("create fixture directory");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"e2e-cache-fixed-point\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub fn extract_with_external_redaction(input: String) -> String { input }\n",
    )
    .expect("write source");
    fs::write(
        root.join("fixtures/contract/external_redaction.json"),
        r#"{
  "id": "extract_with_external_redaction",
  "category": "contract",
  "description": "eligible external redaction API remains generated",
  "input": { "value": "eligible" },
  "assertions": [{ "type": "not_error" }]
}"#,
    )
    .expect("write e2e fixture");
    fs::write(
        root.join("alef.toml"),
        format!(
            r#"[workspace]
alef_version = "{}"
languages = ["rust"]

[[crates]]
name = "e2e-cache-fixed-point"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["rust"]

[crates.e2e.call]
function = "extract_with_external_redaction"
result_var = "result"
async = false
returns_result = false

[[crates.e2e.call.args]]
name = "input"
field = "input.value"
type = "string"
"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("write alef.toml");
}

fn write_marker_stripping_poly(bin_dir: &Path) {
    fs::create_dir_all(bin_dir).expect("create fake tool directory");
    let poly = bin_dir.join("poly");
    fs::write(
        &poly,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "poly fixture"
  exit 0
fi
for argument in "$@"; do
  if [ -d "$argument" ]; then
    find "$argument" -type f -name '*.rs' | while IFS= read -r file; do
      awk '!/alef:hash:/' "$file" > "$file.tmp"
      mv "$file.tmp" "$file"
    done
  fi
done
"#,
    )
    .expect("write fake poly");
    let mut permissions = fs::metadata(&poly).expect("stat fake poly").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(poly, permissions).expect("make fake poly executable");
}

fn run_e2e_generate(root: &Path, bin_dir: &Path) -> Output {
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&existing_path)))
            .expect("construct PATH");
    Command::new(alef_binary())
        .current_dir(root)
        .env("PATH", path)
        .args(["e2e", "generate", "--strict"])
        .output()
        .expect("run alef e2e generate")
}

#[test]
fn consecutive_e2e_generations_are_byte_identical_and_keep_eligible_fixture() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    let root = fixture.path();
    let bin_dir = root.join("tools");
    write_fixture(root);
    write_marker_stripping_poly(&bin_dir);

    let first = run_e2e_generate(root, &bin_dir);
    assert!(
        first.status.success(),
        "first generation must succeed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let generated_path = root.join("e2e/rust/tests/contract_test.rs");
    let first_bytes = fs::read(&generated_path).expect("read first generated suite");
    let first_text = String::from_utf8_lossy(&first_bytes);
    assert!(
        first_text.contains("extract_with_external_redaction"),
        "the eligible external-redaction fixture must be present after the first generation"
    );
    assert!(
        first_text.contains("alef:hash:"),
        "the first generation must prove the ownership stamp is present"
    );

    let second = run_e2e_generate(root, &bin_dir);
    assert!(
        second.status.success(),
        "cached second generation must succeed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_bytes = fs::read(&generated_path).expect("read second generated suite");
    assert_eq!(
        second_bytes, first_bytes,
        "a formatter-running cache hit must leave generated e2e output byte-identical"
    );
    assert!(
        String::from_utf8_lossy(&second_bytes).contains("extract_with_external_redaction"),
        "the eligible external-redaction fixture must survive the cache-hit generation"
    );
}
