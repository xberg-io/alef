//! Regression coverage for the java docs-snippet renderer's streaming-adapter blindness.
//!
//! Split out of `snippet.rs`'s inline `tests` module to keep that file under the 1000-line
//! cap (`file-modularization`).
//!
//! Before this fix, `render_snippet_body_with_ir` always called `build_args_and_setup` with
//! `adapter_request_type: None` and `owner_handle_is_receiver: false` -- literal, hardcoded
//! values -- so a fixture calling a streaming adapter whose Java facade takes a typed request
//! DTO (`engine.streamEvents(SampleStreamRequest req)`) instead rendered the flat, scalar-arg
//! shape (`Facade.streamEvents(engine, url)`) every plain call takes. `javac` rejects that:
//! `String` is not assignable to a request record. `java/test_method.rs` (the generated e2e
//! test path) already resolved this correctly via `config.adapters`; these tests pin that the
//! snippet path now asks the same seam instead of carrying its own, silently stale answer. ~keep

use super::render_snippet_body;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::{AdapterConfig, AdapterPattern};
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::fixture::Fixture;

fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
    body.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
}

fn engine_handle_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "engine".into(),
        field: "input.engine".into(),
        arg_type: "handle".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn url_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "url".into(),
        field: "input.url".into(),
        arg_type: "mock_url".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn urls_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "urls".into(),
        field: "input.urls".into(),
        arg_type: "mock_url_list".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn streaming_owner_adapter(name: &str, request_type: &str) -> AdapterConfig {
    AdapterConfig {
        name: name.to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: format!("sample::{name}"),
        params: Vec::new(),
        returns: None,
        error_type: None,
        owner_type: Some("SampleEngine".to_string()),
        item_type: Some("sample::SampleEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some(request_type.to_string()),
        skip_languages: Vec::new(),
    }
}

/// The defect this fix closes: a single-item streaming call whose facade takes the owner
/// handle as the instance receiver and a typed request DTO. Without the fix the call rendered
/// `Sample.streamEvents(engine, url)` -- a nonexistent static overload -- instead of the real
/// `engine.streamEvents(urlReq)` instance call the Java backend actually generates.
#[test]
fn a_single_streaming_call_passes_the_typed_request_not_the_flat_url() {
    let fixture = Fixture {
        id: "stream_a_document".into(),
        description: "Stream a document".into(),
        input: serde_json::json!({"engine": {}, "url": "https://example.com/doc"}),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "stream_events".into(),
        result_var: "events".into(),
        args: vec![engine_handle_arg(), url_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "java".into(),
        CallOverride {
            class: Some("Sample".into()),
            ..CallOverride::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_owner_adapter("stream_events", "sample::SampleStreamRequest")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
    );

    // Positive: the request DTO is really built from the mock URL and the call dispatches on
    // the owner handle instance, not a static overload.
    assert_eq!(
        line_containing(&body, "String url ="),
        "        String url = \"https://example.com/doc\";"
    );
    assert_eq!(
        line_containing(&body, "urlReq ="),
        "        var urlReq = new SampleStreamRequest(url);"
    );
    assert_eq!(
        line_containing(&body, "streamEvents("),
        "        var events = engine.streamEvents(urlReq);"
    );
    // Negative: the pre-fix flat shape must not appear anywhere in the snippet.
    assert!(
        !body.contains("Sample.streamEvents("),
        "must not call a static overload the Java facade never declares:\n{body}"
    );
    assert!(
        !body.contains("streamEvents(engine, url)"),
        "must not pass the raw handle and URL positionally:\n{body}"
    );
}

/// The batch counterpart: a `mock_url_list` arg wrapped in the adapter's declared batch
/// request type, exactly as `java/test_method.rs` already renders it for the generated e2e
/// test.
#[test]
fn a_batch_streaming_call_wraps_the_url_list_in_the_typed_batch_request() {
    let fixture = Fixture {
        id: "stream_several_documents".into(),
        description: "Stream several documents".into(),
        input: serde_json::json!({
            "engine": {},
            "urls": ["https://a.example.com/doc", "https://b.example.com/doc"]
        }),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "stream_events_batch".into(),
        result_var: "events".into(),
        args: vec![engine_handle_arg(), urls_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "java".into(),
        CallOverride {
            class: Some("Sample".into()),
            ..CallOverride::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_owner_adapter(
            "stream_events_batch",
            "sample::SampleBatchStreamRequest",
        )],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
    );

    assert_eq!(
        line_containing(&body, "java.util.List<String> urls ="),
        "        java.util.List<String> urls = java.util.List.of(\"https://a.example.com/doc\", \
         \"https://b.example.com/doc\");"
    );
    assert_eq!(
        line_containing(&body, "urlsReq ="),
        "        var urlsReq = new SampleBatchStreamRequest(urls);"
    );
    assert_eq!(
        line_containing(&body, "streamEventsBatch("),
        "        var events = engine.streamEventsBatch(urlsReq);"
    );
    assert!(
        !body.contains("java.util.List.of(\"https://a.example.com/doc\", \"https://b.example.com/doc\"))"),
        "the raw URL list must not be passed positionally to the call itself:\n{body}"
    );
}

/// Negative control, and the pin that keeps this change scoped: an ordinary `mock_url` call
/// with NO matching adapter (the common case for every non-streaming fixture) must render the
/// flat shape exactly as before, even though `config.adapters` is non-empty -- proving the
/// lookup is scoped to the call's own function name, not "any adapter configured anywhere
/// disables the flat shape". Without this control, a change that wrapped every mock_url arg
/// unconditionally would pass the two tests above just as well.
#[test]
fn a_non_streaming_call_with_no_matching_adapter_keeps_the_flat_mock_url_shape() {
    let fixture = Fixture {
        id: "scrape_a_page".into(),
        description: "Scrape a page".into(),
        input: serde_json::json!({"url": "https://example.com/doc"}),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "scrape".into(),
        result_var: "result".into(),
        args: vec![url_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "java".into(),
        CallOverride {
            class: Some("Sample".into()),
            ..CallOverride::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        // A streaming adapter IS configured, just for a different function -- proves the
        // per-call lookup, not mere presence of `config.adapters`, gates the wrapping.
        adapters: vec![streaming_owner_adapter("stream_events", "sample::SampleStreamRequest")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
    );

    assert_eq!(
        line_containing(&body, "scrape("),
        "        var result = Sample.scrape(url);"
    );
    assert!(
        !body.contains("Req = new"),
        "a call with no matching adapter must not wrap its mock_url arg in a request DTO:\n{body}"
    );
}

#[cfg(test)]
mod stream_presentation_regression {
    use super::*;

    #[test]
    fn streaming_usage_is_printed_from_consumed_chunks() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id":"stream_usage", "description":"Read stream usage", "input":{},
            "docs":{"topic":"streaming"},
            "assertions":[{"type":"not_empty", "field":"usage.total_tokens"}]
        }))
        .expect("fixture");
        let config: E2eConfig = serde_json::from_value(serde_json::json!({
            "call":{"function":"stream_usage", "module":"example", "streaming":true},
            "fields_optional":["usage"]
        }))
        .expect("config");
        let body = render_snippet_body(&fixture, &config, &ResolvedCrateConfig::default(), &[]);
        assert!(body.contains("result.forEach(resultChunk ->"), "{body}");
        assert!(body.contains("resultChunk.usage() != null"), "{body}");
    }
}
