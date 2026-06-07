use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::render_main_test_go;
use super::setup::build_args_and_setup;
use super::test_file::{GoTestFileContext, render_test_file};
use super::test_function::{GoTestFunctionContext, render_test_function};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        source: String::new(),
        http: None,
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// snake_case function names in `[e2e.call]` must be routed through `to_go_name`
/// so the emitted Go call uses the idiomatic CamelCase (e.g. `CleanExtractedText`
/// instead of `clean_extracted_text`).
#[test]
fn test_go_method_name_uses_go_casing() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "clean_extracted_text".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let fixture = make_fixture("basic_text");
    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        },
    );

    assert!(
        out.contains("sample_crate.CleanExtractedText("),
        "expected Go-cased method name 'CleanExtractedText', got:\n{out}"
    );
    assert!(
        !out.contains("sample_crate.clean_extracted_text("),
        "must not emit raw snake_case method name, got:\n{out}"
    );
}

#[test]
fn handle_config_deserialization_uses_resolved_options_type() {
    let args = vec![ArgMapping {
        name: "session".to_string(),
        field: "input.config".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "session_fixture".to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "config": { "limit": 3 } }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
    };
    let data_enum_names = std::collections::HashSet::new();
    let (package_decls, setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        "pkg",
        Some("SessionConfig"),
        &fixture,
        false,
        false,
        &data_enum_names,
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
        &[],
    );

    let rendered = setup.join("\n");
    assert!(package_decls.is_empty());
    assert_eq!(args_str, "session");
    assert!(rendered.contains("var sessionConfig pkg.SessionConfig"));
    assert!(rendered.contains("pkg.CreateSession(&sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

#[test]
fn test_streaming_fixture_emits_collect_snippet() {
    // A streaming fixture should emit `stream, err :=` and the collect loop.
    let streaming_fixture_json = r#"{
            "id": "basic_stream",
            "description": "basic streaming test",
            "call": "chat_stream",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}]
            },
            "assertions": [
                {"type": "count_min", "field": "chunks", "value": 1}
            ]
        }"#;
    let fixture: Fixture = serde_json::from_str(streaming_fixture_json).unwrap();
    assert!(fixture.is_streaming_mock(), "fixture should be detected as streaming");

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat_stream".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            r#async: true,
            streaming: Some(crate::core::config::e2e::StreamingConfig::Recipe(
                crate::core::config::e2e::StreamingRecipe {
                    item_type: Some("StreamChunk".to_string()),
                    ..Default::default()
                },
            )),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        },
    );

    assert!(out.contains("stream, err :="), "should use stream binding, got:\n{out}");
    assert!(
        out.contains("for chunk := range stream"),
        "should emit collect loop, got:\n{out}"
    );
}

#[test]
fn test_streaming_with_client_factory_and_json_arg() {
    // Covers no returns_result on the call, json_object args
    // (binding_returns_error=true), and client_factory from the Go call override.
    use crate::core::config::e2e::{ArgMapping, CallOverride};
    let streaming_fixture_json = r#"{
            "id": "basic_stream_client",
            "description": "basic streaming test with client",
            "call": "chat_stream",
            "input": {"model": "gpt-4", "messages": [{"role": "user", "content": "hello"}]},
            "mock_response": {
                "status": 200,
                "stream_chunks": [{"delta": "hello"}]
            },
            "assertions": [
                {"type": "count_min", "field": "chunks", "value": 1}
            ]
        }"#;
    let fixture: Fixture = serde_json::from_str(streaming_fixture_json).unwrap();
    assert!(fixture.is_streaming_mock(), "fixture should be detected as streaming");

    let go_override = CallOverride {
        client_factory: Some("CreateClient".to_string()),
        ..Default::default()
    };

    let mut call_overrides = std::collections::HashMap::new();
    call_overrides.insert("go".to_string(), go_override);

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat_stream".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: false, // NOT true — like real demo-client
            r#async: true,
            streaming: Some(crate::core::config::e2e::StreamingConfig::Recipe(
                crate::core::config::e2e::StreamingRecipe {
                    item_type: Some("StreamChunk".to_string()),
                    ..Default::default()
                },
            )),
            args: vec![ArgMapping {
                name: "request".to_string(),
                field: "input".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            overrides: call_overrides,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let mut out = String::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &crate::core::config::ResolvedCrateConfig::default(),
            type_defs: &[],
            enums: &[],
        },
    );

    eprintln!("generated:\n{out}");
    assert!(out.contains("stream, err :="), "should use stream binding, got:\n{out}");
    assert!(
        out.contains("for chunk := range stream"),
        "should emit collect loop, got:\n{out}"
    );
}

/// When `segments` is an optional field (Option<Vec<T>>) and a fixture asserts on
/// `segments[0].id`, the prefix guard must be `result.Segments != nil` — NOT
/// `result.Segments[0] != nil`, which is a compile error for a value-typed element.
#[test]
fn test_indexed_element_prefix_guard_uses_array_not_element() {
    let mut optional_fields = std::collections::HashSet::new();
    optional_fields.insert("segments".to_string());
    let mut array_fields = std::collections::HashSet::new();
    array_fields.insert("segments".to_string());

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "transcribe".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_optional: optional_fields,
        fields_array: array_fields,
        ..E2eConfig::default()
    };

    let fixture = Fixture {
        id: "edge_transcribe_with_timestamps".to_string(),
        category: None,
        description: "Transcription with timestamp segments".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        source: String::new(),
        http: None,
        assertions: vec![
            Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            },
            Assertion {
                assertion_type: "equals".to_string(),
                field: Some("segments[0].id".to_string()),
                value: Some(serde_json::Value::Number(serde_json::Number::from(0u64))),
                ..Default::default()
            },
        ],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        },
    );

    eprintln!("generated:\n{out}");

    // Must guard on the slice itself — not on the element.
    // Accepts either `Segments != nil` or `len(Segments) > 0`; both are
    // valid Go guards for the slice and avoid the invalid element nil
    // check.
    assert!(
        out.contains("result.Segments != nil") || out.contains("len(result.Segments) > 0"),
        "guard must be on Segments (the slice), not an element; got:\n{out}"
    );
    // Must NOT emit the invalid element nil check.
    assert!(
        !out.contains("result.Segments[0] != nil"),
        "must not emit Segments[0] != nil for a value-type element; got:\n{out}"
    );
}

/// Regression test: a `result_is_simple` call with a `contains` assertion whose
/// `field` ("result") is not a struct field must still bind the call to the result
/// variable AND emit the `fmt`/`strings` imports.  The assertion renderer ignores
/// the field for `result_is_simple` calls and emits `strings.Contains(fmt.Sprint(result), …)`,
/// so binding to `_` (or omitting the imports) produces uncompilable Go.
#[test]
fn test_result_is_simple_contains_binds_result_and_emits_imports() {
    use crate::core::config::e2e::ArgMapping;

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "detect_mime_type_from_bytes".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_is_simple: true,
            args: vec![ArgMapping {
                name: "content".to_string(),
                field: "input.data".to_string(),
                arg_type: "bytes".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let fixture = Fixture {
        id: "mime_detect_bytes".to_string(),
        category: None,
        description: "Detect MIME type from file bytes".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({"data": "pdf/fake_memo.pdf"}),
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("result".to_string()),
            value: Some(serde_json::Value::String("pdf".to_string())),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let out = render_test_file(
        "mime_utilities",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/mylib",
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        },
    );

    assert!(
        out.contains("result, err := sample_crate.DetectMimeTypeFromBytes("),
        "expected the call to bind to `result`, not `_`; got:\n{out}"
    );
    assert!(
        out.contains("strings.Contains(") && out.contains("string("),
        "expected `strings.Contains(string(...))` cast rendering; got:\n{out}"
    );
    assert!(
        !out.contains("\t\"fmt\""),
        "expected fmt import to NOT be emitted (uses string cast not fmt.Sprint); got:\n{out}"
    );
    assert!(
        out.contains("\t\"strings\""),
        "expected the `strings` import to be emitted; got:\n{out}"
    );
}

#[test]
fn main_test_go_http_fixtures_omits_net_http_and_strings_imports() {
    // When needs_mock_server_bootstrap=false (HTTP-fixtures harness path), the bootstrap uses
    // net.DialTimeout + io.Copy for readiness polling.
    // "net/http" and "strings" are NOT referenced, so they must not be imported.
    let out = render_main_test_go("testing_data", false, true);
    assert!(
        !out.contains("\t\"net/http\""),
        "main_test.go (http-fixtures harness path) must NOT import net/http; got:\n{out}"
    );
    assert!(
        !out.contains("\t\"strings\""),
        "main_test.go (http-fixtures harness path) must NOT import strings; got:\n{out}"
    );
    // But it must still import "net" and "io" for the harness path
    assert!(out.contains("\t\"net\""), "must import net; got:\n{out}");
    assert!(out.contains("\t\"io\""), "must import io; got:\n{out}");
}

#[test]
fn main_test_go_non_http_fixtures_includes_net_http_and_strings_imports() {
    // When needs_mock_server_bootstrap=true (mock-server path for function-call fixtures),
    // http.Get (net/http) and strings.HasPrefix/TrimPrefix are used — both must be imported.
    let out = render_main_test_go("testing_data", true, false);
    assert!(
        out.contains("\t\"net/http\""),
        "main_test.go (mock-server bootstrap path) must import net/http; got:\n{out}"
    );
    assert!(
        out.contains("\t\"strings\""),
        "main_test.go (mock-server bootstrap path) must import strings; got:\n{out}"
    );
    // io is now needed for the runTests helper's io.ReadCloser parameter
    assert!(
        out.contains("\t\"io\""),
        "main_test.go (mock-server bootstrap path) must import io for helper; got:\n{out}"
    );
    // And must NOT import "net" (that's http-fixtures harness path only)
    assert!(
        !out.contains("\t\"net\""),
        "main_test.go (mock-server bootstrap path) must NOT import net; got:\n{out}"
    );
}

/// The generated TestMain must set `MOCK_SERVER_NO_STDIN_WATCH=1` on the
/// mock-server subprocess so the server does not treat stdin EOF (from
/// Go's exec.Command defaulting Stdin to /dev/null) as a shutdown signal.
#[test]
fn main_test_go_sets_mock_server_no_stdin_watch_env() {
    let out = render_main_test_go("testing_data", true, false);
    assert!(
        out.contains("MOCK_SERVER_NO_STDIN_WATCH=1"),
        "main_test.go must set MOCK_SERVER_NO_STDIN_WATCH=1 on the mock-server subprocess; got:\n{out}"
    );
    // Must appear as cmd.Env assignment, not as a stray string in a comment.
    assert!(
        out.contains("cmd.Env = append(os.Environ(),"),
        "main_test.go must use cmd.Env = append(os.Environ(), ...) form; got:\n{out}"
    );
}

/// Regression test: TestMain must not trigger the 'exitAfterDefer' linter error.
/// This is avoided by extracting deferred cleanup into helper functions that
/// return int before os.Exit is called.
#[test]
fn main_test_go_avoids_exitafterdefer_linter_error() {
    // Mock-server bootstrap path: must have a runTests helper function
    let mock_server_out = render_main_test_go("testing_data", true, false);
    assert!(
        mock_server_out.contains("func runTests(m *testing.M, cmd *exec.Cmd, stdout io.ReadCloser) int"),
        "mock-server bootstrap path must emit runTests helper; got:\n{mock_server_out}"
    );
    assert!(
        mock_server_out.contains("code := runTests(m, cmd, stdout)"),
        "TestMain must call runTests to get int, not inline defer; got:\n{mock_server_out}"
    );
    assert!(
        mock_server_out.contains("os.Exit(code)"),
        "os.Exit must be called AFTER runTests returns; got:\n{mock_server_out}"
    );
    // Must NOT have os.Exit inside a function with defers still in scope
    assert!(
        !mock_server_out.contains("defer func() { _ = cmd.Process.Kill() }()")
            || mock_server_out.contains("func runTests"),
        "defers must be moved out of TestMain scope; got:\n{mock_server_out}"
    );

    // Harness-spawn path: must have runHarnessTests helper
    let harness_out = render_main_test_go("testing_data", false, true);
    assert!(
        harness_out.contains(
            "func runHarnessTests(m *testing.M, cmd *exec.Cmd, stdin io.WriteCloser, stdout io.ReadCloser) int"
        ),
        "harness-spawn path must emit runHarnessTests helper; got:\n{harness_out}"
    );
    assert!(
        harness_out.contains("code := runHarnessTests(m, cmd, stdin, stdout)"),
        "TestMain must call runHarnessTests to get int; got:\n{harness_out}"
    );
    assert!(
        harness_out.contains("os.Exit(code)"),
        "os.Exit must be called AFTER runHarnessTests returns; got:\n{harness_out}"
    );
}
