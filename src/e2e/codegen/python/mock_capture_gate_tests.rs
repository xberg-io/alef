//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443), split out of
//! `test_file.rs` (over the baselined file-size ceiling) matching the
//! `dead_helper_tests.rs`/`test_file_misc_tests.rs` split.
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`references_identifier(&fixtures_body, capture.helper_name)` in
//! `render_test_file`) are tested SEPARATELY everywhere else in this codebase -- these tests
//! drive the real `render_test_file` entry point so a fixture's call and the file's helper
//! definition land in ONE rendered artifact, the only way to catch the two disagreeing about
//! `capture.helper_name`. The async form additionally pins the `asyncio.to_thread` call
//! alongside the `import asyncio` line `import_lines::finalize_stdlib_and_bare_imports` gates
//! through its own, independent scan. ~keep

use super::*;

fn minimal_fixture(id: &str, assertions: Vec<crate::e2e::fixture::Assertion>) -> crate::e2e::fixture::Fixture {
    crate::e2e::fixture::Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        description: "Mock capture smoke test".to_string(),
        input: serde_json::Value::Null,
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions,
        call: None,
        skip: None,
        env: None,
        setup: Vec::new(),
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        mock_response: None,
        source: String::new(),
        category: None,
        tags: Vec::new(),
    }
}

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render(fixture: &crate::e2e::fixture::Fixture, e2e_config: &crate::e2e::config::E2eConfig) -> String {
    let fixtures: Vec<&crate::e2e::fixture::Fixture> = vec![fixture];
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_test_file(
        "mock_capture",
        &fixtures,
        e2e_config,
        &config,
        &[],
        &[],
        &[],
        &[],
        false,
    )
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same (synchronous) fixture -- cheap here since both share one rendered file. ~keep
#[test]
fn a_sync_mock_assertion_gets_both_its_call_and_the_helper_definition() {
    let fixture = minimal_fixture(
        "mock_capture_sync",
        vec![
            mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
            mock_assertion(
                r#"mock.requests["POST /v1/chat"]"#,
                "greater_than_or_equal",
                serde_json::json!(1),
            ),
        ],
    );
    let out = render(&fixture, &crate::e2e::config::E2eConfig::default());

    assert!(
        out.contains(
            "    assert _alef_mock_request_count(os.environ[\"MOCK_SERVER_URL\"], \"/__alef/requests/total?prefix=\") == 1, \"expected mock.requests.total == 1\""
        ),
        "expected the sync total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "    assert _alef_mock_request_count(os.environ[\"MOCK_SERVER_URL\"], \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 1, \"expected mock.requests[\\\"POST /v1/chat\\\"] >= 1\""
        ),
        "expected the sync bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("def _alef_mock_request_count(base_url: str, path_and_query: str) -> int:"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
    // The call reads `os.environ["MOCK_SERVER_URL"]`, so the file must import `os`. This fixture
    // has no client factory, no http test and no `mock_url` argument, so none of the config flags
    // `needs_os_import` is built from are set -- only the rendered-body scan can catch it, and
    // without that scan the generated suite raises NameError on a line every other test here
    // asserts is correct (alef issue #443). ~keep
    assert!(
        out.contains("\nimport os\n"),
        "expected `import os` alongside the os.environ call, got:\n{out}"
    );
}

/// The async form is a third independent scan (`import_lines::finalize_stdlib_and_bare_imports`'s
/// `needs_asyncio_import`) beyond the two the sync test above already covers -- pin the
/// `asyncio.to_thread` call, the `import asyncio` line, AND the helper definition together. ~keep
#[test]
fn an_async_mock_assertion_gets_the_to_thread_call_and_the_asyncio_import_together() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.r#async = true;
    let fixture = minimal_fixture(
        "mock_capture_async",
        vec![mock_assertion("mock.requests.total", "count_min", serde_json::json!(1))],
    );
    let out = render(&fixture, &e2e_config);

    assert!(
        out.contains(
            "    assert await asyncio.to_thread(_alef_mock_request_count, os.environ[\"MOCK_SERVER_URL\"], \"/__alef/requests/total?prefix=\") >= 1, \"expected mock.requests.total >= 1\""
        ),
        "expected the asyncio.to_thread call, got:\n{out}"
    );
    assert!(
        out.contains("import asyncio"),
        "expected the `import asyncio` line alongside the to_thread call, got:\n{out}"
    );
    assert!(
        out.contains("def _alef_mock_request_count(base_url: str, path_and_query: str) -> int:"),
        "expected the helper definition too, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass both positive
/// tests above, so a fixture with NO `mock.*` assertion must not reference or define the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper() {
    let fixture = minimal_fixture(
        "mock_capture_absent",
        vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
    );
    let out = render(&fixture, &crate::e2e::config::E2eConfig::default());
    assert!(
        !out.contains("_alef_mock_request_count"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
}
