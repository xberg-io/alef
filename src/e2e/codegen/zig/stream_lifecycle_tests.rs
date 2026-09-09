use super::*;

fn render_stream(assertion: Option<&str>) -> String {
    render_named_stream(assertion, "stream_records")
}

fn render_named_stream(assertion: Option<&str>, function: &str) -> String {
    render_stream_shape(assertion, function, true)
}

fn render_stream_shape(assertion: Option<&str>, function: &str, json_result: bool) -> String {
    let mut fixture = Fixture {
        id: "stream_lifecycle".into(),
        input: serde_json::json!({}),
        ..Default::default()
    };
    if let Some(assertion) = assertion {
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: assertion.into(),
            field: match assertion {
                "not_empty" => Some("chunks".into()),
                "equals" => Some("model".into()),
                _ => None,
            },
            value: match assertion {
                "equals" => Some(serde_json::json!("retained-model-marker")),
                "error" => Some(serde_json::json!("provider failure")),
                _ => None,
            },
            ..Default::default()
        });
    }
    if assertion == Some("error") {
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "equals".into(),
            field: Some("error.status_code".into()),
            value: Some(serde_json::json!(401)),
            ..Default::default()
        });
    }
    let mut e2e = E2eConfig::default();
    e2e.call.function = function.into();
    e2e.call.args = vec![crate::e2e::config::ArgMapping {
        name: "request".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            result_is_json_struct: json_result,
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        adapters: vec![crate::core::config::AdapterConfig {
            name: function.into(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("sample::Client::{function}"),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some("Client".into()),
            item_type: Some("Record".into()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample::RecordRequest".into()),
            skip_languages: Vec::new(),
        }],
        ..ResolvedCrateConfig::default()
    };

    render_test_file(
        "streaming",
        &[&fixture],
        &e2e,
        function,
        "result",
        &[],
        "sample",
        "sample",
        &config,
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    )
}

#[test]
fn stream_without_field_assertions_is_drained_and_freed() {
    for assertion in [None, Some("not_error")] {
        let rendered = render_stream(assertion);
        assert!(rendered.contains("_next("), "stream was not consumed: {rendered}");
        assert!(
            rendered.contains("_last_error_code()"),
            "iteration errors ignored: {rendered}"
        );
        assert!(
            !rendered.contains("free(_result_json)"),
            "iterator freed as bytes: {rendered}"
        );
    }
}

#[test]
fn expected_stream_error_is_observed_during_iteration() {
    let rendered = render_stream(Some("error"));
    assert!(rendered.contains("_next("), "only initiation checked: {rendered}");
    assert!(
        rendered.contains("_last_error_code()"),
        "deferred error was not checked: {rendered}"
    );
    assert!(
        rendered.contains("error.TestUnexpectedResult"),
        "successful completion must fail: {rendered}"
    );
    assert!(
        rendered.contains("defer") && rendered.contains("_free("),
        "stream lifecycle not released: {rendered}"
    );
}

#[test]
fn configured_stream_adapter_does_not_require_stream_in_its_name() {
    for assertion in [None, Some("error"), Some("not_empty")] {
        let rendered = render_named_stream(assertion, "watch");
        assert!(
            rendered.contains("_client_watch_next("),
            "configured stream not drained: {rendered}"
        );
    }
}

#[test]
fn ordinary_stream_assertion_is_not_silently_discarded() {
    let rendered = render_named_stream(Some("equals"), "watch");
    assert!(
        rendered.contains("retained-model-marker"),
        "ordinary assertion disappeared: {rendered}"
    );
}

#[test]
fn stream_error_message_reads_current_raw_context() {
    let rendered = render_stream(Some("error"));
    assert!(
        rendered.contains("sample.c.sample_last_error_context()"),
        "raw error context not read: {rendered}"
    );
    assert!(
        !rendered.contains("sample._last_error()"),
        "stale wrapper capture used: {rendered}"
    );
    assert!(
        rendered.contains("provider failure"),
        "literal expectation lost: {rendered}"
    );
}

#[test]
fn adapter_metadata_drives_asserted_streams_without_json_result_flag() {
    for assertion in ["not_empty", "equals"] {
        let rendered = render_stream_shape(Some(assertion), "watch", false);
        assert!(
            rendered.contains("_client_watch_next("),
            "metadata stream was not drained: {rendered}"
        );
        if assertion == "equals" {
            assert!(
                rendered.contains("retained-model-marker"),
                "assertion disappeared: {rendered}"
            );
        }
    }
}

#[test]
fn expected_stream_error_does_not_allocate_json_result() {
    let rendered = render_stream(Some("error"));
    assert!(
        !rendered.contains("const allocator ="),
        "error path allocates unused JSON allocator: {rendered}"
    );
}

#[test]
fn asserted_stream_checks_error_after_collecting_chunks() {
    let rendered = render_stream(Some("not_empty"));
    assert!(
        rendered.contains("sample.c.sample_last_error_code()"),
        "partial stream failure swallowed: {rendered}"
    );
}
