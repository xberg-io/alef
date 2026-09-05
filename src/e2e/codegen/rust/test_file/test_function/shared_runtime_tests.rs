//! Regression coverage for the shared-runtime fix: every generated test now blocks a single
//! process-wide multi-thread Tokio runtime (`common::runtime().block_on(...)`) instead of
//! spinning up — and dropping — its own via `#[tokio::test]`.
//!
//! ~keep A `#[tokio::test]`-per-test runtime is dropped when the test returns, but the crawler
//! under test caches its HTTP client process-globally with no runtime identity. A pooled
//! connection created on one dropped runtime can later be handed to a different test's runtime
//! already dead, producing intermittent "error sending request" / "error decoding response body"
//! failures. Routing every async test through one shared runtime for the life of the process
//! eliminates the cross-runtime connection-pool hazard.

use super::*;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Fixture, MockResponse};

fn crate_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"sample_lib\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn base_call(is_async: bool) -> CallConfig {
    CallConfig {
        function: "process".to_string(),
        module: "sample_lib".to_string(),
        result_var: "result".to_string(),
        r#async: is_async,
        ..CallConfig::default()
    }
}

fn render(fixture: &Fixture, call: CallConfig) -> String {
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let mut out = String::new();
    render_test_function(
        &mut out,
        fixture,
        &e2e_config,
        &crate_config(),
        &[],
        &[],
        &[],
        "sample_lib",
        None,
        None,
        false,
    );
    out
}

#[test]
fn async_call_config_blocks_the_shared_runtime_instead_of_tokio_test() {
    let fixture = Fixture {
        id: "process_async".to_string(),
        description: "an async call".to_string(),
        ..Fixture::default()
    };
    let out = render(&fixture, base_call(true));

    assert!(!out.contains("#[tokio::test]"), "{out}");
    assert!(!out.contains("async fn test_"), "{out}");
    assert!(out.contains("#[test]\nfn test_process_async() {\n"), "{out}");
    assert!(out.contains("common::runtime().block_on(async {"), "{out}");
    assert!(out.trim_end().ends_with("});\n}"), "{out}");

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}

#[test]
fn sync_call_config_renders_a_plain_test_with_no_runtime_wrapper() {
    let fixture = Fixture {
        id: "process_sync".to_string(),
        description: "a sync call".to_string(),
        ..Fixture::default()
    };
    let out = render(&fixture, base_call(false));

    assert!(!out.contains("#[tokio::test]"), "{out}");
    assert!(!out.contains("common::runtime()"), "{out}");
    assert!(out.contains("#[test]\nfn test_process_sync() {\n"), "{out}");

    let unit = syn::parse_file(&out);
    assert!(unit.is_ok(), "generated Rust must parse: {:?}\n{out}", unit.err());
}

/// A mock-backed fixture is always async (Axum requires a Tokio runtime) even when the call
/// itself is not configured `async = true` — it must still block the shared runtime, not spin
/// up its own via `#[tokio::test]`.
#[test]
fn mock_response_fixture_blocks_the_shared_runtime_even_when_the_call_is_not_marked_async() {
    let fixture = Fixture {
        id: "mocked_call".to_string(),
        description: "a call backed by a mock server".to_string(),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::json!({"ok": true})),
            stream_chunks: None,
            headers: Default::default(),
        }),
        ..Fixture::default()
    };
    let out = render(&fixture, base_call(false));

    assert!(!out.contains("#[tokio::test]"), "{out}");
    assert!(out.contains("#[test]\nfn test_mocked_call() {\n"), "{out}");
    assert!(out.contains("common::runtime().block_on(async {"), "{out}");
    assert!(out.trim_end().ends_with("});\n}"), "{out}");
}

/// The "unsupported: no callable API configured" stub has no real work to block on, so it stays
/// a plain synchronous `#[test]` with no shared-runtime wrapper.
#[test]
fn unsupported_stub_fixture_renders_a_plain_test_with_no_runtime_wrapper() {
    let fixture = Fixture {
        id: "spec_only".to_string(),
        description: "a schema-only fixture with no callable function".to_string(),
        ..Fixture::default()
    };
    let call = CallConfig {
        function: String::new(),
        ..CallConfig::default()
    };
    let out = render(&fixture, call);

    assert!(!out.contains("#[tokio::test]"), "{out}");
    assert!(!out.contains("common::runtime()"), "{out}");
    assert!(out.contains("#[test]\nfn test_spec_only() {\n"), "{out}");
}
