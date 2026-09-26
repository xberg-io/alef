//! #442: a non-default `[crates.e2e] alt_host` must be baked into the standalone
//! mock-server spawn's environment, not silently dropped.
//!
//! Split out rather than added to `project.rs` (already over the file-modularization cap;
//! see CLAUDE.md's `file-modularization` rule) so this regression test does not grow it
//! further.

use super::project::{BootstrapOptions, render_bootstrap};
use crate::e2e::config::E2eConfig;

/// A distinctive alt_host value (not the crate-default `"localhost"`) proves the generator
/// threads the configured value through rather than hard-coding it.
#[test]
fn bootstrap_bakes_configured_alt_host_into_the_mock_server_spawn() {
    let e2e_config = E2eConfig {
        alt_host: "alt-host-from-config.test".to_string(),
        ..E2eConfig::default()
    };
    let out = render_bootstrap(BootstrapOptions {
        e2e_config: &e2e_config,
        pkg_path: "pkg",
        has_mock_server_fixtures: true,
        has_file_fixtures: false,
        test_documents_path: "test_documents",
        uses_server_harness: false,
        harness_host: "127.0.0.1",
        harness_port: 8000,
    });

    assert!(
        out.contains("putenv('ALEF_MOCK_ALT_HOST=alt-host-from-config.test')"),
        "bootstrap.php must set ALEF_MOCK_ALT_HOST to the configured alt_host, got:\n{out}"
    );
}
