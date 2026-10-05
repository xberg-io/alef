use super::helpers::resolve_module_for_call;
use super::*;

#[test]
fn resolve_module_for_call_prefers_crate_name_override() {
    use crate::e2e::config::CallConfig;
    use std::collections::HashMap;
    let mut overrides = HashMap::new();
    overrides.insert(
        "rust".to_string(),
        crate::e2e::config::CallOverride {
            crate_name: Some("custom_crate".to_string()),
            module: Some("ignored_module".to_string()),
            ..Default::default()
        },
    );
    let call = CallConfig {
        overrides,
        ..Default::default()
    };
    let result = resolve_module_for_call(&call, "dep_name");
    assert_eq!(result, "custom_crate");
}

/// Regression test: a non-streaming fixture whose result struct has a `chunks`
/// field (registered in `fields_array`) must emit `let chunks = &result.chunks;`
/// before any assertion so the streaming-virtual-field arm's hardcoded `chunks`
/// identifier resolves.  Without the fix this generated
/// `assert!(chunks.len() >= 2 as usize, ...)` with `chunks` undeclared.
#[test]
fn fields_array_binding_emitted_before_count_min_assertion_for_non_streaming_fixture() {
    use crate::e2e::config::{CallConfig, StreamingConfig};
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    let mut fields_array = HashSet::new();
    fields_array.insert("chunks".to_string());

    let call = CallConfig {
        function: "process".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        fields_array,
        returns_result: true,
        streaming: Some(StreamingConfig::Enabled(false)),
        ..Default::default()
    };

    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "chunking_test".to_string(),
        description: "Chunking produces multiple pieces".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "count_min".to_string(),
            field: Some("chunks".to_string()),
            value: Some(serde_json::Value::Number(serde_json::Number::from(2u64))),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &[],
        &[],
        &[],
        "my_crate",
        None,
        None,
        false,
    );

    assert!(
        out.contains("let chunks = &result.chunks"),
        "expected `let chunks = &result.chunks` binding before assertion; got:\n{out}"
    );
    assert!(
        out.contains("chunks.len() >= 2"),
        "expected count_min assertion referencing `chunks`; got:\n{out}"
    );
    // The binding must appear before the assertion in the output.
    let binding_pos = out.find("let chunks = &result.chunks").unwrap();
    let assert_pos = out.find("chunks.len() >= 2").unwrap();
    assert!(
        binding_pos < assert_pos,
        "binding must appear before assertion; got:\n{out}"
    );
}

/// Regression test for alef task #81: rust had no fallback at all for a dropped
/// field assertion — `is_valid_for_result` rejects the field, `render_assertion`
/// emits a skip comment, and (until this fix) nothing else ever consulted it. This
/// pins that `finalize_test_body` sees the skip comment with the exact marker text
/// the shared `fail_on_unavailable_field_markers` mechanism (src/e2e/codegen/mod.rs)
/// matches on. `body_buf` is a fresh buffer per `render_test_function` call (see
/// `let mut body_buf = String::new();` at the top of this file), so per-fixture
/// attribution is correct by construction — no offset bookkeeping needed here,
/// unlike Go's shared-buffer caller.
#[test]
fn dropped_field_assertion_carries_the_marker_that_arms_the_strict_mode() {
    use crate::e2e::config::CallConfig;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    let call = CallConfig {
        function: "process".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        result_fields: HashSet::from(["content".to_string()]),
        returns_result: true,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "process_smoke".to_string(),
        description: "Process produces a result".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::Value::String("x".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &[],
        &[],
        &[],
        "my_crate",
        None,
        None,
        false,
    );

    assert!(
        out.contains("field 'nonexistent_field' not available on result type"),
        "got:\n{out}"
    );
}

/// End-to-end regression test for the `fields_enum` defect: with NO `fields_enum` entry
/// configured anywhere in `alef.toml`, an `equals` assertion on an enum-typed field must
/// still stringify via `Debug` (`format!("{:?}", ...)`), not `.to_string()` — the latter is a
/// compile error for any enum that does not implement `Display`. The IR alone (a function
/// returning `DataNode`, whose `kind` field is the real enum `DataNodeKind`) must be enough.
///
/// A sibling type `PlainNode` also declares a field named `kind`, but as a plain `String` —
/// proving the fix is keyed by the field's actual declared type on `DataNode`, not by the
/// bare name `"kind"` (which appears on both types with different meanings).
#[test]
fn enum_typed_field_stringifies_via_debug_with_no_fields_enum_config_at_all() {
    use crate::core::ir::{EnumDef, FieldDef, FunctionDef, TypeDef, TypeRef};
    use crate::e2e::config::CallConfig;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashSet;

    let call = CallConfig {
        function: "get_node".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        result_fields: HashSet::from(["kind".to_string()]),
        returns_result: false,
        ..Default::default()
    };
    // `fields_enum` is deliberately left empty everywhere in this config — the defect this
    // test guards against is exactly "no fields_enum entry anywhere".
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "get_node_kind".to_string(),
        description: "get_node returns a DataNode whose kind is an enum".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            skip: None,
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("Foo".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let functions = vec![FunctionDef {
        name: "get_node".to_string(),
        return_type: TypeRef::Named("DataNode".to_string()),
        ..Default::default()
    }];
    let type_defs = vec![
        TypeDef {
            name: "DataNode".to_string(),
            fields: vec![FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::Named("DataNodeKind".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "PlainNode".to_string(),
            fields: vec![FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let enums = vec![EnumDef {
        name: "DataNodeKind".to_string(),
        ..Default::default()
    }];

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &type_defs,
        &enums,
        &functions,
        "my_crate",
        None,
        None,
        false,
    );

    assert!(
        out.contains("format!(\"{:?}\", "),
        "expected enum field to stringify via Debug; got:\n{out}"
    );
    assert!(
        !out.contains(".kind.to_string()"),
        "must not emit .to_string() for a field the IR proves is an enum; got:\n{out}"
    );
}

/// Regression test for alef task #81: a `test_backend` arg whose declared trait has
/// no matching `[[crates.trait_bridges]]` entry used to fall through — with only a
/// `tracing::warn!` — to rendering the arg's raw (null) fixture value as an ordinary
/// argument, deferring the real failure to a later `cargo build` far from the
/// fixture that caused it. It must now fail at generation time, naming the fixture,
/// the arg, and the missing trait — mirroring `c/assertions.rs`'s
/// `build_args_string_c`, which already fails loudly for the identical scenario.
#[test]
#[should_panic(expected = "fixture `register_sample_backend` requires trait `SampleBackend`")]
fn unregistered_test_backend_trait_fails_loudly_instead_of_falling_back() {
    use crate::e2e::config::{ArgMapping, CallConfig};
    use crate::e2e::fixture::Fixture;

    let call = CallConfig {
        function: "register_backend".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        returns_result: false,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        id: "register_sample_backend".to_string(),
        description: "Register a sample backend".to_string(),
        args: vec![ArgMapping {
            name: "backend".to_string(),
            field: "backend".to_string(),
            arg_type: "test_backend".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("SampleBackend".to_string()),
        }],
        ..Fixture::default()
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    // No `[[crates.trait_bridges]]` entries declared — `SampleBackend` is unregistered.
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &[],
        &[],
        &[],
        "my_crate",
        None,
        None,
        false,
    );
}

/// Regression test: a `result_is_simple` call with a `count_equals` assertion whose
/// `field` is NOT a real field on the (plain Vec) result type must still bind the
/// call to the result variable.  The assertion renderer emits `result.len()` for
/// `result_is_simple` calls regardless of the field, so binding to `_` would leave
/// `result` undefined.
#[test]
fn result_is_simple_count_assertion_binds_to_result_variable() {
    use crate::e2e::config::{CallConfig, StreamingConfig};
    use crate::e2e::fixture::{Assertion, Fixture};

    let call = CallConfig {
        function: "embed_texts".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        result_is_simple: true,
        returns_result: true,
        streaming: Some(StreamingConfig::Enabled(false)),
        ..Default::default()
    };

    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "embed_empty".to_string(),
        description: "embed_texts: empty input".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![
            Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
            Assertion {
                skip: None,
                assertion_type: "count_equals".to_string(),
                field: Some("embeddings".to_string()),
                value: Some(serde_json::Value::Number(serde_json::Number::from(0u64))),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
        ],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: None,
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &[],
        &[],
        &[],
        "my_crate",
        None,
        None,
        false,
    );

    assert!(
        out.contains("let result = embed_texts"),
        "expected the call to bind to `result`, not `_`; got:\n{out}"
    );
    assert!(
        out.contains("assert_eq!(result.len(), 0"),
        "expected `count_equals` assertion to render `result.len()`; got:\n{out}"
    );
    assert!(
        !out.contains("let _ = embed_texts"),
        "call must not bind to `_` when an assertion references the result; got:\n{out}"
    );
}

#[test]
fn handle_config_import_uses_resolved_options_type() {
    use crate::e2e::config::{ArgMapping, CallConfig, CallOverride};
    use crate::e2e::fixture::Fixture;
    use std::collections::HashMap;

    let mut overrides = HashMap::new();
    overrides.insert(
        "rust".to_string(),
        CallOverride {
            options_type: Some("SessionConfig".to_string()),
            ..Default::default()
        },
    );
    let call = CallConfig {
        function: "run_session".to_string(),
        module: "my_crate".to_string(),
        result_var: "result".to_string(),
        returns_result: false,
        args: vec![ArgMapping {
            name: "session".to_string(),
            field: "input.config".to_string(),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        overrides,
        ..Default::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "session_fixture".to_string(),
        description: "session fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "config": { "limit": 3 } }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        category: Some("sessions".to_string()),
    };
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    let out = render_test_file(
        "sessions",
        &[&fixture],
        &e2e_config,
        &test_config,
        &[],
        &[],
        &[],
        "my_crate",
        false,
        None,
        false,
    );

    assert!(
        out.contains("use my_crate::SessionConfig;"),
        "expected SessionConfig import, got:\n{out}"
    );
    assert!(out.contains("let session_config: SessionConfig = serde_json::from_str"));
    assert!(!out.contains("CrawlConfig"));
}

mod client_factory_construction {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
    use crate::e2e::fixture::{Fixture, FixtureDocsClient, MockResponse};

    const DOCUMENTED_BASE_URL: &str = "https://llm.internal.example.com/v1";

    fn crate_config() -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn e2e_config(trailing_args: &[&str]) -> E2eConfig {
        let mut call = CallConfig {
            function: "chat".to_string(),
            module: "my_crate".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..Default::default()
        };
        call.overrides.insert(
            "rust".to_string(),
            CallOverride {
                client_factory: Some("create_client".to_string()),
                client_factory_trailing_args: trailing_args.iter().map(|arg| (*arg).to_string()).collect(),
                ..Default::default()
            },
        );
        E2eConfig {
            call,
            ..Default::default()
        }
    }

    fn fixture(with_mock: bool) -> Fixture {
        Fixture {
            id: "custom_base_url".to_string(),
            description: "Client configured with a custom base URL".to_string(),
            input: serde_json::Value::Null,
            mock_response: with_mock.then(|| MockResponse {
                status: 200,
                body: Some(serde_json::json!({"ok": true})),
                stream_chunks: None,
                headers: Default::default(),
            }),
            ..Fixture::default()
        }
    }

    fn documented_client() -> FixtureDocsClient {
        FixtureDocsClient {
            base_url: Some(DOCUMENTED_BASE_URL.to_string()),
            ..FixtureDocsClient::default()
        }
    }

    fn render(fixture: &Fixture, config: &E2eConfig, docs_client: Option<&FixtureDocsClient>) -> String {
        let mut out = String::new();
        render_test_function(
            &mut out,
            fixture,
            config,
            &crate_config(),
            &[],
            &[],
            &[],
            "my_crate",
            Some("create_client"),
            docs_client,
            false,
        );
        out
    }

    fn client_line(rendered: &str) -> String {
        rendered
            .lines()
            .find(|line| line.contains("create_client("))
            .unwrap_or_else(|| panic!("no client construction in:\n{rendered}"))
            .trim()
            .to_string()
    }

    #[test]
    fn a_project_configuring_nothing_keeps_the_argument_list_it_renders_today() {
        let rendered = render(&fixture(false), &e2e_config(&[]), None);
        assert_eq!(
            client_line(&rendered),
            "let client = my_crate::create_client(\"test-key\".to_string(), None, None, None, None).unwrap();",
            "reviving client_factory_trailing_args must not change unconfigured output"
        );
    }

    #[test]
    fn a_mocked_fixture_keeps_binding_the_mock_server_to_the_base_url_slot() {
        let rendered = render(&fixture(true), &e2e_config(&[]), None);
        assert_eq!(
            client_line(&rendered),
            "let client = my_crate::create_client(\"test-key\".to_string(), Some(mock_server.url.clone()), None, None, None).unwrap();"
        );
    }

    #[test]
    fn configured_trailing_args_replace_the_generators_hardcoded_defaults() {
        let rendered = render(&fixture(false), &e2e_config(&["Some(30)", "Some(2)", "None"]), None);
        assert_eq!(
            client_line(&rendered),
            "let client = my_crate::create_client(\"test-key\".to_string(), None, Some(30), Some(2), None).unwrap();"
        );
    }

    #[test]
    fn a_documentation_snippet_renders_the_base_url_the_fixture_documents() {
        let client = documented_client();
        let rendered = render(&fixture(false), &e2e_config(&[]), Some(&client));
        assert_eq!(
            client_line(&rendered),
            format!(
                "let client = my_crate::create_client(\"test-key\".to_string(), Some(\"{DOCUMENTED_BASE_URL}\".to_string()), None, None, None).unwrap();"
            ),
            "the snippet for a custom-base-url topic must show the custom base URL"
        );
    }

    #[test]
    fn a_documentation_client_can_replace_the_trailing_args_for_its_language_alone() {
        let client = FixtureDocsClient {
            base_url: Some(DOCUMENTED_BASE_URL.to_string()),
            args: [(
                "rust".to_string(),
                vec!["Some(120)".to_string(), "None".to_string(), "None".to_string()],
            )]
            .into_iter()
            .collect(),
            ..FixtureDocsClient::default()
        };
        let rendered = render(&fixture(false), &e2e_config(&["None", "None", "None"]), Some(&client));
        assert!(
            client_line(&rendered).contains("Some(120), None, None"),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn the_e2e_suite_ignores_the_documentation_base_url_of_every_fixture_that_declares_one() {
        // ~keep The docs override and the mock server compete for the same argument
        // slot. A generator that consulted the fixture's docs metadata directly would
        // retarget a real e2e test at an endpoint that exists only in prose, and the
        // suite would still compile — so the isolation is pinned here rather than left
        // to the reader of the resolution order.
        let mut documented = fixture(true);
        documented.docs = Some(
            serde_json::from_value(serde_json::json!({
                "topic": "configuration",
                "client": {"base_url": DOCUMENTED_BASE_URL}
            }))
            .expect("fixture docs"),
        );

        let e2e_mode = render(&documented, &e2e_config(&[]), None);
        assert!(
            !e2e_mode.contains(DOCUMENTED_BASE_URL),
            "the documented endpoint leaked into the executable suite:\n{e2e_mode}"
        );
        assert_eq!(
            client_line(&e2e_mode),
            client_line(&render(&fixture(true), &e2e_config(&[]), None)),
            "declaring docs.client must not change a single byte of the e2e test"
        );
    }
}

/// End-to-end coverage for the `mock.*` request-count gate (alef issue #443): the call site
/// (`assertion_mock_capture::try_render_mock_capture_assertion`) and the helper-definition gate
/// (`file_rendering::render_test_file`'s `body_references_symbol(&body_buf, capture.helper_name)`
/// scan) are tested SEPARATELY everywhere else in this codebase -- this drives the real
/// `render_test_file` entry point so both land in ONE rendered artifact, the only way to catch
/// the two disagreeing about `capture.helper_name`.
mod mock_capture_gate {
    use super::*;
    use crate::e2e::config::{CallConfig, E2eConfig};
    use crate::e2e::fixture::{Assertion, Fixture, MockResponse};

    fn crate_config() -> crate::core::config::ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"my_crate\"\nsources = [\"src/lib.rs\"]\n",
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn e2e_config() -> E2eConfig {
        let call = CallConfig {
            function: "chat".to_string(),
            module: "my_crate".to_string(),
            result_var: "result".to_string(),
            returns_result: false,
            ..Default::default()
        };
        E2eConfig {
            call,
            ..Default::default()
        }
    }

    fn fixture_with_assertions(assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            id: "mock_capture_fixture".to_string(),
            description: "asserts against the mock server request log".to_string(),
            input: serde_json::Value::Null,
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::json!({"ok": true})),
                stream_chunks: None,
                headers: Default::default(),
            }),
            assertions,
            ..Fixture::default()
        }
    }

    fn render(fixture: &Fixture) -> String {
        render_test_file(
            "mock_capture",
            &[fixture],
            &e2e_config(),
            &crate_config(),
            &[],
            &[],
            &[],
            "my_crate",
            true,
            None,
            false,
        )
    }

    /// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key
    /// form on the same fixture -- cheap here since both share one rendered file. ~keep
    #[test]
    fn a_mock_assertion_gets_both_its_call_and_the_helper_definition_in_one_file() {
        let fixture = fixture_with_assertions(vec![
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("mock.requests.total".to_string()),
                value: Some(serde_json::json!(1)),
                ..Default::default()
            },
            Assertion {
                assertion_type: "greater_than_or_equal".to_string(),
                field: Some(r#"mock.requests["POST /v1/chat"]"#.to_string()),
                value: Some(serde_json::json!(1)),
                ..Default::default()
            },
        ]);
        let out = render(&fixture);

        assert!(
            out.contains(
                "    assert_eq!(alef_mock_request_count(mock_server.url.as_str(), \"/__alef/requests/total?prefix=\").await, 1, \"expected mock.requests.total == 1\");"
            ),
            "expected the total-count call, got:\n{out}"
        );
        assert!(
            out.contains(
                "    assert!(alef_mock_request_count(mock_server.url.as_str(), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\").await >= 1, \"expected mock.requests[\\\"POST /v1/chat\\\"] >= 1\");"
            ),
            "expected the bracketed-key call, got:\n{out}"
        );
        assert!(
            out.contains("async fn alef_mock_request_count(base_url: &str, path_and_query: &str) -> u64 {"),
            "expected the helper definition in the same file as the call, got:\n{out}"
        );
    }

    /// Negative direction: an always-emit bug in the helper-definition gate would pass every
    /// positive test above, so a fixture with NO `mock.*` assertion must not define the helper.
    #[test]
    fn a_file_with_no_mock_assertion_defines_no_helper() {
        let fixture = fixture_with_assertions(vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }]);
        let out = render(&fixture);
        assert!(
            !out.contains("alef_mock_request_count"),
            "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
        );
    }
}
