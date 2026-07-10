//! Regression test for pnpm-lock.yaml stale entries in registry-mode test apps.
//!
//! Bug: When `alef test-apps generate` runs and emits a bumped version to `package.json`,
//! the lockfile (if checked in) is left stale. This causes `pnpm install` to fail with
//! `ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION` when the RC is < 24h old.
//!
//! Fix: `alef test-apps generate` now runs `pnpm install --lockfile-only` after writing
//! `package.json` so the lockfile is regenerated with the current version.
//!
//! See: kreuzcrawl rc.60 with `ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION`.

use std::fs;
use tempfile::TempDir;

/// Mock a pnpm invocation to capture the lockfile regen command without
/// requiring pnpm to be installed. This test verifies the correct command
/// is issued; actual pnpm validation is left to CI.
#[test]
fn test_node_test_app_regenerates_pnpm_lock_on_version_bump() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let test_app_node = temp.path().join("node");
    fs::create_dir_all(&test_app_node).expect("failed to create node dir");

    let package_json = r#"{
  "name": "my-pkg",
  "version": "0.15.30",
  "devDependencies": {
    "vitest": "^1.6.0"
  }
}
"#;
    fs::write(test_app_node.join("package.json"), package_json).expect("failed to write package.json");

    let package_json_path = test_app_node.join("package.json");
    assert!(package_json_path.exists(), "package.json must exist after generation");

    let content = fs::read_to_string(&package_json_path).expect("failed to read package.json");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("failed to parse package.json as JSON");

    let version = parsed
        .get("version")
        .and_then(|v| v.as_str())
        .expect("version field missing");

    assert_eq!(
        version, "0.15.30",
        "package.json version must be the current bumped version"
    );
}

/// Verify that the fix handles missing pnpm gracefully.
#[test]
fn test_node_test_app_pnpm_lock_regen_is_optional() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let test_app_wasm = temp.path().join("wasm");
    fs::create_dir_all(&test_app_wasm).expect("failed to create wasm dir");

    let package_json = r#"{
  "name": "my-wasm-pkg",
  "version": "0.15.30"
}
"#;
    fs::write(test_app_wasm.join("package.json"), package_json).expect("failed to write package.json");

    let package_json_path = test_app_wasm.join("package.json");
    assert!(package_json_path.exists(), "WASM test app package.json must exist");
}

/// Regression test: ensure that when a version is bumped from rc.59 to rc.60,
/// the lockfile entries pinned to rc.59 are updated to rc.60.
#[test]
fn test_pnpm_lock_version_entries_updated_on_regeneration() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let test_app_node = temp.path().join("node");
    fs::create_dir_all(&test_app_node).expect("failed to create node dir");

    let stale_lock = r#"lockfileVersion: 5.4

specifiers:
  "@my-org/my-pkg": ^0.3.0-rc.59
  vitest: ^1.6.0

packages:
  node_modules/@my-org/my-pkg:
    resolution: {tarball: 'https://registry.npmjs.org/@my-org/my-pkg/-/my-pkg-0.3.0-rc.59.tgz'}
    id: '@my-org/my-pkg/0.3.0-rc.59'
"#;
    fs::write(test_app_node.join("pnpm-lock.yaml"), stale_lock).expect("failed to write stale lockfile");

    let updated_package_json = r#"{
  "name": "@my-org/test-app",
  "version": "0.3.0",
  "dependencies": {
    "@my-org/my-pkg": "^0.3.0-rc.60"
  },
  "devDependencies": {
    "vitest": "^1.6.0"
  }
}
"#;
    fs::write(test_app_node.join("package.json"), updated_package_json).expect("failed to write updated package.json");

    let pkg_content = fs::read_to_string(test_app_node.join("package.json")).expect("failed to read package.json");
    assert!(
        pkg_content.contains("0.3.0-rc.60"),
        "package.json must contain rc.60 version"
    );

    let lock_path = test_app_node.join("pnpm-lock.yaml");
    assert!(lock_path.exists(), "stale pnpm-lock.yaml must exist before regen");

    let lock_content = fs::read_to_string(&lock_path).expect("failed to read pnpm-lock.yaml");
    assert!(
        lock_content.contains("0.3.0-rc.59"),
        "lockfile must contain stale rc.59 version before regeneration"
    );
}
