//! Shared `npx` gate for the tests that run a real `napi build`.
//!
//! Every such test invokes `npx --yes -p @napi-rs/cli@<version> ...`, and `npx` installs the
//! package into a shared cache directory keyed by the package spec on first use. Two tests that
//! reach that first use concurrently -- which `cargo test`'s default thread pool makes routine --
//! both install into the same directory, and the loser can observe the winner's half-written
//! `node_modules`: `Test (ubuntu-latest)` on 2026-09-18 failed both napi fixtures with
//! `ERR_MODULE_NOT_FOUND ... fast-string-truncated-width` under `~/.npm/_npx/`. The
//! [`OnceLock`] here performs that first install exactly once, before any test's own `npx` runs,
//! so the build commands only ever find an already-populated cache. ~keep
//!
//! [`OnceLock`]: std::sync::OnceLock

use crate::core::template_versions as tv;
use std::path::{Path, PathBuf};

/// Apply the shared, serialized Cargo lane used by both real `napi build` fixtures.
///
/// `napi build` delegates compilation to Cargo. A temp-local target makes each fixture rebuild
/// the complete napi dependency tree, while one checkout-local target lets Cargo reuse it across
/// the two cases. Callers hold [`crate::test_support::RealCargoGuard`] for the command's lifetime,
/// so the shared directory has only one writer within this test process. ~keep
pub(super) fn apply_napi_cargo_test_environment(command: &mut std::process::Command) {
    command
        .env("CARGO_TARGET_DIR", napi_cargo_target_directory())
        .env("CARGO_BUILD_JOBS", "1");
}

fn napi_cargo_target_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("napi-real-build-probes")
}

/// Whether `npx` runs, not merely resolves, and `@napi-rs/cli` is installed in its cache.
///
/// A version-manager shim (e.g. nvm) spawns fine then exits non-zero, so a PATH-only check would
/// leave the callers' skip unreachable and fire their asserts everywhere Node is absent. ~keep
pub(super) fn napi_cli_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| npx_is_runnable() && warm_napi_cli_cache())
}

fn npx_is_runnable() -> bool {
    std::process::Command::new("npx")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Populate the `npx` cache for the exact package spec the build command emits, serialized
/// through the caller's [`std::sync::OnceLock`]. A failure here (no network on a cold cache, a
/// broken registry) is reported as "not runnable" so the callers skip, matching the convention
/// for tests that depend on an external toolchain.
fn warm_napi_cli_cache() -> bool {
    std::process::Command::new("npx")
        .args([
            "--yes",
            "-p",
            &format!("@napi-rs/cli@{}", tv::npm::NAPI_RS_CLI_CRATE),
            "napi",
            "--version",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn real_napi_builds_share_one_bounded_checkout_local_cargo_lane() {
    let mut command = std::process::Command::new("sh");

    apply_napi_cargo_test_environment(&mut command);

    let environment = command
        .get_envs()
        .filter_map(|(name, value)| Some((name.to_str()?, value?.to_str()?)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected_target = napi_cargo_target_directory();
    assert_eq!(
        environment["CARGO_TARGET_DIR"],
        expected_target.to_str().expect("UTF-8 target directory")
    );
    assert_eq!(environment["CARGO_BUILD_JOBS"], "1");
}

#[test]
fn every_real_napi_build_acquires_the_guard_and_shared_cargo_lane() {
    for source in [
        include_str!("napi_js_ownership_tests.rs"),
        include_str!("napi_package_json_path_tests.rs"),
    ] {
        assert!(
            source.contains("RealCargoGuard::acquire()"),
            "every real napi build must serialize with other real-Cargo tests"
        );
        assert!(
            source.contains("apply_napi_cargo_test_environment(&mut process)"),
            "every real napi build must reuse the checkout-local Cargo target"
        );
    }
}
