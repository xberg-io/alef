use super::*;

#[test]
fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
    let fixture = Fixture {
        id: "rate_limit_429".into(),
        description: "Rate limited".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(
        !rendered.contains("MOCK_SERVER"),
        "mock-server env var leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains("/fixtures/rate_limit_429"),
        "mock-server fixture route leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains("\"test-key\""),
        "literal credential leaked:\n{rendered}"
    );
    assert!(
        rendered.contains("std.c.getenv(\"API_KEY\")"),
        "credential is not read from the environment:\n{rendered}"
    );
    assert!(
        rendered.contains("sample.create_client(std.mem.span(_api_key), null, null, null, null)"),
        "client is not constructed the way a reader would:\n{rendered}"
    );
}

/// A fixture whose docs declare a custom `client.base_url` — the mechanism a
/// `configuration/custom-base-url` topic uses — must show that base URL in its Zig
/// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
/// (`python/mod.rs::client_factory_snippet_renders_the_base_url_the_fixture_documents`).
/// Paired with `client_factory_snippet_without_docs_client_keeps_the_base_url_slot_null`
/// below as the negative control: an indiscriminate "always add base_url" change would
/// fail that test. ~keep
#[test]
fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "custom_base_url",
        "description": "Custom base URL",
        "input": null,
        "docs": {
            "topic": "configuration",
            "client": {"base_url": "https://llm.internal.example.com/v1"}
        }
    }))
    .expect("fixture must parse");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.result_var = "result".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(
        rendered.contains(
            "sample.create_client(std.mem.span(_api_key), \"https://llm.internal.example.com/v1\", null, null, null)"
        ),
        "the snippet for a custom-base-url topic must show the custom base URL:\n{rendered}"
    );
}

/// Negative control for the test above: a fixture that declares no `docs.client` must
/// keep the base-url slot as a bare `null`, exactly what the generator rendered before
/// this fixture's docs got wired up. ~keep
#[test]
fn client_factory_snippet_without_docs_client_keeps_the_base_url_slot_null() {
    let fixture = Fixture {
        id: "no_docs_client".into(),
        description: "No docs client".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(
        rendered.contains("sample.create_client(std.mem.span(_api_key), null, null, null, null)"),
        "a fixture with no docs.client must keep the bare base-url slot:\n{rendered}"
    );
}

#[test]
fn e2e_test_file_still_points_the_client_at_the_mock_server() {
    let fixture = Fixture {
        id: "rate_limit_429".into(),
        description: "Rate limited".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    let rendered = render_test_file(
        "errors",
        &[&fixture],
        &e2e,
        "chat",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(
        rendered.contains("MOCK_SERVER_URL"),
        "e2e test lost its mock-server wiring:\n{rendered}"
    );
    assert!(
        rendered.contains("/fixtures/rate_limit_429"),
        "e2e test lost its per-fixture route:\n{rendered}"
    );
}

#[test]
fn snippet_keeps_import_and_call_without_test_harness() {
    let fixture = Fixture {
        id: "count".into(),
        description: "Count".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "count".into();
    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");
    assert!(rendered.contains("const sample = @import(\"sample\")"));
    assert!(rendered.contains("const result = try sample.count()"));
    assert!(rendered.contains("std.debug.print(\"{any}\\n\", .{result})"));
    assert!(!rendered.contains("test \""));
    assert!(!rendered.contains("defer "));
    assert!(rendered.contains("pub fn main() !void"));
    assert!(!rendered.contains("testing."));
}

#[test]
fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "present_items", "description": "Present returned items", "input": null,
        "docs": {"topic": "guides", "presentation": {"operations": [
            {"op": "show", "path": "summary", "display": true},
            {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
        ]}}
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(rendered.contains("const result = try sample.process()"), "{rendered}");
    assert!(
        rendered.contains("std.debug.print(\"{s}\\n\", .{ result.summary });"),
        "{rendered}"
    );
    assert!(rendered.contains("for (result.items) |item| {"), "{rendered}");
    assert!(
        rendered.contains("std.debug.print(\"{any}\\n\", .{ item.label });"),
        "{rendered}"
    );
    assert!(
        !rendered.contains(".{result})"),
        "the whole-result fallback must give way to the documented presentation:\n{rendered}"
    );
}

/// A `result_is_json_struct` call binds `_result_json` (a `[]u8` payload) and never a
/// typed `result`, so `docs.shows` field paths have no struct to read from — the
/// snippet must keep printing the whole payload rather than emit `result.summary`.
#[test]
fn json_struct_result_keeps_the_payload_print_instead_of_field_accessors() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "present_items", "description": "Present returned items", "input": null,
        "docs": {"topic": "guides", "presentation": {"operations": [
            {"op": "show", "path": "summary", "display": true}
        ]}}
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.result_fields = ["summary".to_string()].into_iter().collect();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            result_is_json_struct: true,
            ..Default::default()
        },
    );

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(!rendered.contains("result.summary"), "{rendered}");
    assert!(
        rendered.contains("std.debug.print(\"{s}\\n\", .{_result_json})"),
        "{rendered}"
    );
}

#[test]
fn json_result_snippet_consumes_and_frees_the_result() {
    let fixture = Fixture {
        id: "process".into(),
        description: "Process".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.call.overrides.insert(
        "zig".into(),
        crate::e2e::config::CallOverride {
            result_is_json_struct: true,
            ..Default::default()
        },
    );

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(
        rendered.contains("const _result_json = try sample.process()"),
        "{rendered}"
    );
    assert!(rendered.contains("free(_result_json)"), "{rendered}");
    assert!(rendered.contains("std.debug.print(\"{s}\\n\", .{_result_json})"));
}

#[test]
fn generated_tests_preserve_abort_failures() {
    let fixture = Fixture {
        id: "count".into(),
        description: "Count".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "count".into();
    let rendered = render_test_file(
        "smoke",
        &[&fixture],
        &e2e,
        "count",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(!rendered.contains("suppress_abort"), "{rendered}");
    assert!(!rendered.contains("signal(6, 1)"), "{rendered}");
}

#[test]
fn expected_error_snippet_prints_caught_error_name() {
    let mut fixture = Fixture {
        id: "invalid".into(),
        description: "Invalid".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");
    assert!(rendered.contains("else |err|"));
    assert!(rendered.contains("call failed as expected"));
    assert!(rendered.contains("return error.TestUnexpectedResult"));
    assert!(!rendered.contains("testing.expect"));
}

#[test]
fn visitor_snippet_reuses_native_callback_setup() {
    let mut fixture = Fixture {
        id: "custom_text".into(),
        description: "Custom text".into(),
        input: serde_json::json!({ "html": "<p>Hello</p>" }),
        ..Fixture::default()
    };
    fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
        callbacks: [("visit_text".into(), crate::e2e::fixture::CallbackAction::Continue)].into(),
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "render_document".into();
    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("visitor snippet renders");
    assert!(rendered.contains("pub fn main() !void"));
    assert!(rendered.contains("_visitor"));
    assert!(!rendered.contains("test \""));
    assert!(!rendered.contains("testing."));
    assert!(!rendered.contains("\n    }\n}"), "{rendered}");
}

#[test]
fn streaming_snippet_reuses_error_union_call_preparation() {
    let fixture = Fixture {
        id: "stream_items".into(),
        description: "Stream items".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "stream_items".into();
    e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("streaming snippet renders");

    assert!(rendered.contains("stream_items"), "{rendered}");
    assert!(rendered.contains("pub fn main() !void"));
}

#[test]
fn streaming_e2e_uses_scalar_handle_tokens() {
    let mut fixture = Fixture {
        id: "stream_records".into(),
        description: "Stream records".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "not_empty".into(),
        field: Some("chunks".into()),
        ..Default::default()
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "stream_records".into();
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
            result_is_json_struct: true,
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        adapters: vec![crate::core::config::AdapterConfig {
            name: "stream_records".into(),
            pattern: AdapterPattern::Streaming,
            core_path: "sample::Client::stream_records".into(),
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

    let rendered = render_test_file(
        "streaming",
        &[&fixture],
        &e2e,
        "stream_records",
        "result",
        &[],
        "sample",
        "sample",
        &config,
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(rendered.contains("sample.c.sample_client_stream_records_start(_client._handle, _req_handle)"));
    assert!(rendered.contains("if (_stream_handle == 0)"));
    assert!(!rendered.contains("@ptrCast(_client._handle)"));

    let snippet = render_snippet_body(&fixture, &e2e, "sample", "sample", &config, &[], &[], &[])
        .expect("streaming snippet renders");
    assert!(
        snippet.contains("sample.c.sample_client_stream_records_start"),
        "{snippet}"
    );
    assert!(
        snippet.contains("defer sample.c.sample_client_stream_records_free"),
        "{snippet}"
    );
    assert!(
        !snippet.contains("free(_result_json)"),
        "a stream is not a byte slice: {snippet}"
    );
    assert!(
        snippet.contains("sample.c.sample_client_stream_records_next"),
        "stream items must be read: {snippet}"
    );
    assert!(
        snippet.contains("std.debug.print"),
        "stream items must be displayed: {snippet}"
    );
    assert!(
        snippet.contains("sample.c.sample_last_error_code()"),
        "stream failures must propagate: {snippet}"
    );
}

/// Fixture shared by the snippet/test-target pair below: a `json_object` arg whose docs
/// presentation replaces a nested field with a file read (mirrors the batch `bytes_happy`
/// fixture that drives `test_documents/html/html.html` into `/inputs/1/bytes`). The file
/// read is emitted by `render_docs_json`, the SAME emitter `generate()` (test target) and
/// `render_snippet_body()` (snippet target) both call through `build_args_and_setup`.
fn docs_bytes_fixture_and_config() -> (Fixture, E2eConfig) {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "docs_nested_bytes_snippet",
        "description": "Process a document loaded from disk",
        "input": {"content": "ignored"},
        "assertions": [],
        "docs": {
            "topic": "guides",
            "presentation": {
                "files": [{"field": "/content", "path": "document.pdf"}]
            }
        }
    }))
    .expect("fixture");

    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.call.args = vec![crate::e2e::config::ArgMapping {
        name: "input".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    (fixture, e2e)
}

/// Doc snippets compile as standalone `pub fn main()` executables, not `zig test`
/// binaries — `std.testing.io` is only valid inside `builtin.is_test` code (Zig rejects
/// it with "not testing" otherwise). The docs-file read must therefore never reference
/// `std.testing`, regardless of target.
#[test]
fn snippet_target_never_references_std_testing_for_a_docs_bytes_input() {
    let (fixture, e2e) = docs_bytes_fixture_and_config();

    let rendered = render_snippet_body(
        &fixture,
        &e2e,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        &[],
    )
    .expect("snippet renders");

    assert!(rendered.contains("readFileAlloc"), "{rendered}");
    assert!(rendered.contains("std.Io.Threaded"), "{rendered}");
    assert!(!rendered.contains("std.testing"), "{rendered}");
}

/// Sibling control for the snippet-target assertion above: the SAME fixture, rendered
/// through the e2e test-file generator, still imports `std.testing` (legitimate inside a
/// `test { ... }` block) — proving the fix removed the *dependency* on `std.testing.io`
/// from the shared file-read emitter without stripping `std.testing` from the target
/// that is actually allowed to reference it.
#[test]
fn test_target_still_references_std_testing_for_the_same_docs_bytes_input() {
    let (fixture, e2e) = docs_bytes_fixture_and_config();

    let rendered = render_test_file(
        "guides",
        &[&fixture],
        &e2e,
        "process",
        "result",
        &e2e.call.args,
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(rendered.contains("const testing = std.testing;"), "{rendered}");
    assert!(rendered.contains("readFileAlloc"), "{rendered}");
    assert!(rendered.contains("std.Io.Threaded"), "{rendered}");
    assert!(!rendered.contains("std.testing.io"), "{rendered}");
}
