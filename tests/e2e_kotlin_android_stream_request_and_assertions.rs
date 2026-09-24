//! Regression tests for two kotlin_android streaming e2e emitter defects.
//!
//! A2: the streaming call site deserialized the raw fixture `input` blob (fixture-harness
//! metadata such as `mock_responses` included) into the request DTO, which Jackson rejects with
//! `UnrecognizedPropertyException`. The request must be built from the resolved mock-server URL
//! list, exactly as the java backend does.
//!
//! A3: every `stream.*` event assertion was skipped, so the test ended in
//! `Assumptions.assumeTrue(false, ..)` and reported as SKIPPED rather than running.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "demo-crawler"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.democrawler.android"
namespace = "dev.sample_crate.democrawler.android"
artifact_id = "demo-crawler-android"
group_id = "dev.sample_crate"

[[crates.adapters]]
name = "batch_crawl_stream"
pattern = "streaming"
core_path = "batch_crawl_stream"
owner_type = "CrawlEngineHandle"
item_type = "CrawlEvent"
request_type = "democrawler::BatchCrawlStreamRequest"

[[crates.adapters.params]]
name = "req"
type = "BatchCrawlStreamRequest"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "batch_crawl_stream"
result_var = "result"

[crates.e2e.calls.batch_crawl_stream]
function = "batch_crawl_stream"
result_var = "result"
async = true
streaming = true
options_type = "CrawlConfig"

[[crates.e2e.calls.batch_crawl_stream.args]]
name = "engine"
field = "config"
type = "handle"
optional = false

[[crates.e2e.calls.batch_crawl_stream.args]]
name = "urls"
field = "batch_urls"
type = "mock_url_list"
element_type = "String"
owned = true
optional = false

[crates.e2e.packages.kotlin_android]
name = "demo-crawler"
"#;

fn assertion(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        skip: None,
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn render() -> String {
    let fixture = Fixture {
        id: "batch_crawl_stream_events".to_string(),
        category: Some("stream".to_string()),
        description: "Batch crawl stream produces expected Page and Complete events".to_string(),
        call: Some("batch_crawl_stream".to_string()),
        input: serde_json::json!({
            "mock_responses": [
                {"path": "/page1", "status_code": 200, "body_inline": "<html></html>"},
                {"path": "/page2", "status_code": 200, "body_inline": "<html></html>"}
            ],
            "config": {"max_concurrent": 2},
            "batch_urls": ["/page1", "/page2", "/page3"]
        }),
        assertions: vec![
            assertion(
                "greater_than_or_equal",
                "stream.event_count_min",
                Some(serde_json::json!(4)),
            ),
            assertion("is_true", "stream.has_page_event", None),
            assertion("is_true", "stream.has_complete_event", None),
        ],
        source: "stream/batch_crawl_stream_events.json".to_string(),
        ..Fixture::default()
    };

    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "stream".to_string(),
        fixtures: vec![fixture],
    }];
    let files = KotlinAndroidE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("StreamTest.kt"))
        .expect("StreamTest.kt is emitted")
        .content
        .clone()
}

/// A2. The request DTO must be constructed from the resolved mock-server URL list, never
/// deserialized from the fixture's raw `input` object.
#[test]
fn should_build_stream_request_from_mock_server_urls_not_fixture_input_blob() {
    let out = render();
    assert!(
        out.contains("val urlsReq = BatchCrawlStreamRequest(urls)"),
        "expected the request to be constructed from the resolved `urls` list, got:\n{out}"
    );
    assert!(
        out.contains("urlsBase"),
        "expected the mock-server base URL to be used to resolve the fixture's bare paths, got:\n{out}"
    );
    assert!(
        !out.contains("BatchCrawlStreamRequest::class.java"),
        "the fixture `input` blob must not be deserialized into the request DTO, got:\n{out}"
    );
    assert!(
        !out.contains("mock_responses"),
        "fixture-harness metadata must never reach the request DTO, got:\n{out}"
    );
}

/// A3. All three streaming assertions must render as runnable Kotlin, never as skip markers
/// terminating in `assumeTrue(false, ..)`.
#[test]
fn should_emit_runnable_streaming_assertions_for_kotlin_android() {
    let out = render();
    assert!(
        !out.contains("assumeTrue(false"),
        "streaming assertions must be runnable, not skipped, got:\n{out}"
    );
    assert!(
        out.contains("assertTrue(chunks.size >= 4"),
        "expected a runnable stream.event_count_min assertion, got:\n{out}"
    );
    assert!(
        out.contains("chunks.any { it is CrawlEvent.Page }"),
        "expected a runnable stream.has_page_event assertion, got:\n{out}"
    );
    assert!(
        out.contains("chunks.any { it is CrawlEvent.Complete }"),
        "expected a runnable stream.has_complete_event assertion, got:\n{out}"
    );
    assert!(
        out.contains("import dev.sample_crate.democrawler.android.CrawlEvent"),
        "the streaming event union must be imported — a test package does not see its parent's \
         symbols implicitly, got:\n{out}"
    );
}
