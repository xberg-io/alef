//! #442: a non-default `[crates.e2e] alt_host` must be baked into the standalone
//! mock-server spawn's environment, not silently dropped.
//!
//! Split out rather than added directly to `config.rs` (already close to the
//! file-modularization cap; see CLAUDE.md's `file-modularization` rule) so this
//! regression test does not push it over.

use super::config::render_conftest;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

/// A distinctive alt_host value (not the crate-default `"localhost"`) proves the generator
/// threads the configured value through rather than hard-coding it.
#[test]
fn conftest_bakes_configured_alt_host_into_the_mock_server_spawn() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "needs_mock_server",
        "description": "d",
        "input": {"mock_responses": [{"path": "/", "status_code": 200, "body_inline": "ok"}]},
        "assertions": []
    }))
    .expect("fixture parses");
    let groups = vec![FixtureGroup {
        category: "smoke".to_owned(),
        fixtures: vec![fixture],
    }];
    let e2e_config = E2eConfig {
        alt_host: "alt-host-from-config.test".to_string(),
        ..E2eConfig::default()
    };

    let out = render_conftest(&e2e_config, &groups, &[], &[]);

    assert!(
        out.contains("os.environ.setdefault(\"ALEF_MOCK_ALT_HOST\", \"alt-host-from-config.test\")"),
        "conftest.py must set ALEF_MOCK_ALT_HOST to the configured alt_host, got:\n{out}"
    );
}
