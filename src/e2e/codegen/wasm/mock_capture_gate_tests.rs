//! End-to-end coverage for the `mock.*` request-count gate on the WASM e2e generator (alef issue
//! #443).
//!
//! WASM has no `render_assertion` of its own -- `wasm::generate` renders every test file through
//! `typescript::render_test_file` with `lang = "wasm"` (see `src/e2e/codegen/wasm.rs`'s own module
//! doc), the exact same code path `"node"` uses, executing under the identical vitest-on-Node.js
//! harness. `typescript::test_file::mock_capture_gate_tests` already drives that shared entry
//! point for both languages; this file adds the one WASM-shaped regression that generic table
//! covers less directly: a call marked `result_is_simple` (WASM's own doc calls out `speech`-style
//! byte-returning calls as the paradigm case) combined with a `mock.*` fixture. Before the ordering
//! fix in `typescript::assertions::render_assertion_with_streaming_item_type`, the
//! `result_is_simple` length-only dispatch ran BEFORE the `mock.*` interception, so a
//! `mock.requests.total equals 1` assertion on such a call silently became a `result.length`
//! check instead of a call to `alefMockRequestCount`. ~keep

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

fn mock_total_fixture() -> Fixture {
    Fixture {
        id: "wasm_mock_capture_fixture".to_string(),
        category: Some("mock_capture".to_string()),
        description: "asserts against the mock server request log for a result_is_simple call".to_string(),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("mock.requests.total".to_string()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn render(e2e_config: &E2eConfig, fixture: &Fixture) -> String {
    crate::e2e::codegen::typescript::render_test_file(
        "wasm",
        "mock_capture",
        &[fixture],
        "",
        "sample-wasm-bindings",
        "speech",
        &[],
        None,
        None,
        e2e_config,
        &[],
        &[],
        &[],
        "Wasm",
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
    )
}

/// A `result_is_simple` call (WASM's own paradigm case: bytes/Buffer results, see
/// `render_assertion_with_streaming_item_type`'s doc) still routes a `mock.*` assertion to the
/// mock-request-count helper, never to the `result_is_simple` length-only branch. This is the
/// exact ordering bug `render_assertion_with_streaming_item_type`'s `mock.*` interception comment
/// documents; regressing the interception back below that gate makes this fail. ~keep
#[test]
fn mock_assertion_wins_over_the_result_is_simple_length_dispatch() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.result_is_simple = true;
    let fixture = mock_total_fixture();

    let out = render(&e2e_config, &fixture);

    assert!(
        out.contains(
            "    expect(await alefMockRequestCount(`${process.env.MOCK_SERVER_URL}`, \"/__alef/requests/total?prefix=\")).toBe(1);"
        ),
        "expected the mock-capture call even for a result_is_simple call, got:\n{out}"
    );
    assert!(
        !out.contains("result.length"),
        "the mock.* assertion must never fall into the result_is_simple length dispatch, got:\n{out}"
    );
    assert!(
        out.contains("async function alefMockRequestCount(baseUrl: string, pathAndQuery: string): Promise<number> {"),
        "expected the helper definition alongside the call, got:\n{out}"
    );
}

/// Negative control, mirrored from `typescript::test_file::mock_capture_gate_tests`: a
/// `result_is_simple` fixture with no `mock.*` assertion must still take its ordinary
/// `result_is_simple` path and must not reference the helper at all.
#[test]
fn a_result_is_simple_fixture_with_no_mock_assertion_defines_no_helper() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.result_is_simple = true;
    let fixture = Fixture {
        id: "wasm_no_mock_fixture".to_string(),
        category: Some("mock_capture".to_string()),
        assertions: vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("bytes".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = render(&e2e_config, &fixture);

    assert!(
        !out.contains("alefMockRequestCount"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
