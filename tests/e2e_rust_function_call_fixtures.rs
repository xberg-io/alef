//! Regression: rust e2e codegen used to short-circuit any fixture without `http`
//! or `mock_response` to an unsupported stub, on the assumption that such fixtures were
//! schema/spec validation (asyncapi, grpc, graphql_schema, …) with no callable
//! Rust API. That assumption is wrong for libraries whose fixtures invoke a
//! plain function (e.g. `demo_crate::extract_file(path, mime, config)`): every
//! such fixture would emit `// Unsupported: no callable API is configured`
//! instead of a real call, producing 0 effective rust e2e tests.
//!
//! The codegen now stubs only when the resolved call config has no function
//! name. Fixtures that point at a configured `[e2e.call]` (or `[e2e.calls.<n>]`)
//! render real function invocations.
//!
//! See: alef-e2e codegen rust render_test_function (the gate that checks
//! `function_name.is_empty()`).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::rust::RustE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config_with_default_call() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_file"
module = "mylib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "path", field = "input.path", type = "string" },
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "extract_file"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn build_config_without_function() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "schemalib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = ""
module = "schemalib"
async = true
args = []
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn build_function_call_fixture(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "regression: function-call fixture (no http, no mock)".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "path": "test.pdf" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }
}

#[test]
fn rust_codegen_emits_real_call_for_function_fixture_without_http_or_mock() {
    let (e2e, resolved) = build_config_with_default_call();
    let groups = vec![build_function_call_fixture("function_call_fixture")];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("test_function_call_fixture"),
        "test function not emitted: {content}"
    );
    assert!(
        content.contains("extract_file("),
        "real `extract_file(...)` call missing — codegen still emits an unsupported stub:\n{content}"
    );
    assert!(
        !content.contains("Unsupported: no callable API is configured"),
        "unsupported stub still emitted for a fixture with a configured call:\n{content}"
    );
}

#[test]
fn rust_codegen_loads_bytes_arg_from_file_path_value() {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_text_from_pdf"
module = "mylib"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "pdf_bytes", field = "input.data", type = "bytes" },
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "extract_text_from_pdf"
result_is_simple = true
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "load_pdf".to_string(),
            category: Some("smoke".to_string()),
            description: "load a pdf by path".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "data": "pdf/fake_memo.pdf" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("std::fs::read"),
        "expected std::fs::read for file-path bytes value:\n{content}"
    );
    assert!(
        content.contains("/test_documents/"),
        "expected test_documents directory in path resolution:\n{content}"
    );
    assert!(
        content.contains("pdf/fake_memo.pdf"),
        "expected fixture path retained:\n{content}"
    );
    assert!(
        !content.contains(".as_bytes()"),
        "should not pass path string as inline bytes:\n{content}"
    );
}

#[test]
fn rust_codegen_passes_owned_bytes_arg_by_value_not_reference() {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "detect_image_format"
module = "mylib"
result_var = "result"
async = false
args = [
  { name = "data", field = "input.data", type = "bytes", owned = true },
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "detect_image_format"
result_is_simple = true
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "owned_bytes_filepath".to_string(),
            category: Some("smoke".to_string()),
            description: "owned bytes arg loaded from test_documents".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "data": "images/hello.png" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("std::fs::read"),
        "expected std::fs::read for file-path bytes value:\n{content}"
    );
    assert!(
        content.contains("detect_image_format(data)"),
        "expected owned `data` passed by value (no &), got:\n{content}"
    );
    assert!(
        !content.contains("detect_image_format(&data)"),
        "owned bytes must not be passed by reference:\n{content}"
    );
}

#[test]
fn rust_codegen_passes_owned_string_arg_by_value_not_reference() {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "detect_mime_type"
module = "mylib"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "path", field = "input.path", type = "string", owned = true },
  { name = "check_exists", field = "input.check_exists", type = "bool" },
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "detect_mime_type"
result_is_simple = true
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "owned_string_arg".to_string(),
            category: Some("smoke".to_string()),
            description: "owned string arg passed by value".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "path": "doc.pdf", "check_exists": false }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("path.to_string()"),
        "expected `path.to_string()` to convert &str → String for owned arg:\n{content}"
    );
    assert!(
        content.contains("detect_mime_type(path.to_string()"),
        "expected owned `path.to_string()` passed by value:\n{content}"
    );
}

#[test]
fn rust_codegen_imports_function_for_non_mock_call_fixture() {
    let (e2e, resolved) = build_config_with_default_call();
    let groups = vec![build_function_call_fixture("call_fixture")];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("use mylib::extract_file"),
        "expected import for non-mock function-call fixture:\n{content}"
    );
}

#[test]
fn rust_codegen_still_stubs_when_no_callable_function_configured() {
    let (e2e, resolved) = build_config_without_function();
    let groups = vec![build_function_call_fixture("schema_only_fixture")];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("Unsupported: no callable API is configured"),
        "expected unsupported stub when no function is configured, got:\n{content}"
    );
}

#[test]
fn rust_codegen_honors_call_level_streaming_opt_out_for_chunks_field() {
    // tokio_stream::StreamExt::collect(...).await ...` inside a non-async `#[test]`
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_array = ["chunks"]

[crates.e2e.call]
function = "process"
module = "mylib"
result_var = "result"
returns_result = true
streaming = false
args = [
  { name = "source", field = "source_code", type = "string" },
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "process"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);

    let group = FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "chunking_non_streaming".to_string(),
            category: Some("smoke".to_string()),
            description: "non-streaming process() returning a struct with chunks: Vec<_>".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "source_code": "fn x() {}" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "count_min".to_string(),
                field: Some("chunks".to_string()),
                value: Some(serde_json::json!(1)),
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    };

    let files = RustE2eCodegen
        .generate(&[group], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        !content.contains("tokio_stream::StreamExt::collect"),
        "streaming=false fixture must not emit stream-collection code:\n{content}"
    );
    assert!(
        !content.contains(".await"),
        "streaming=false sync test must not emit .await:\n{content}"
    );
    assert!(
        content.contains("fn test_chunking_non_streaming"),
        "expected sync `fn test_*`:\n{content}"
    );
}

fn generate_handle_teardown_fixture(include_handle: bool, call_is_async: bool, expects_error: bool) -> String {
    let args = if include_handle {
        r#"[
  { name = "engine", field = "config", type = "handle" },
  { name = "url", field = "url", type = "string" },
]"#
    } else {
        r#"[
  { name = "url", field = "url", type = "string" },
]"#
    };
    let async_setting = if call_is_async { "async = true" } else { "async = false" };
    let toml_src = format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "mylib"
result_var = "result"
returns_result = true
{async_setting}
args = {args}

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "scrape"
handle_teardown = "shutdown_engine"
result_is_simple = true
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config exists");
    let resolved = cfg.resolve().expect("resolves").remove(0);
    let assertions = if expects_error {
        vec![Assertion {
            skip: None,
            assertion_type: "error".to_string(),
            field: None,
            value: Some(serde_json::json!("boom")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }]
    } else {
        vec![
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
                assertion_type: "not_empty".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            },
        ]
    };
    let group = FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "handle_teardown".to_string(),
            category: Some("smoke".to_string()),
            description: "tear down the owning handle after assertions".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "config": null, "url": "https://example.com" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions,
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    };

    RustE2eCodegen
        .generate(&[group], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds")
        .into_iter()
        .find(|file| file.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted")
        .content
}

#[test]
fn rust_codegen_tears_down_handle_once_after_assertions_inside_async_block() {
    let content = generate_handle_teardown_fixture(true, false, false);
    let assertion = content.find("assert!").expect("assertion is emitted");
    let teardown = content
        .find("shutdown_engine(engine).await;")
        .expect("handle teardown is emitted");
    let async_close = content[teardown..]
        .find("    });")
        .map(|offset| teardown + offset)
        .expect("async block closes after teardown");

    assert_eq!(content.matches("shutdown_engine(engine).await;").count(), 1);
    assert!(content[..content.find("#[test]").expect("test is emitted")].contains("shutdown_engine"));
    assert!(assertion < teardown, "teardown must follow assertions:\n{content}");
    assert!(
        teardown < async_close,
        "teardown must remain inside the async block:\n{content}"
    );
    assert!(content.contains("common::runtime().block_on(async {"));
    assert!(
        content.contains("let result = scrape(&engine, url).expect(\"call failed\");")
            && !content.contains("scrape(&engine, url).await"),
        "sync call itself must not be awaited merely because teardown is async:\n{content}"
    );
}

#[test]
fn rust_codegen_ignores_handle_teardown_without_handle_argument() {
    let content = generate_handle_teardown_fixture(false, false, false);

    assert!(
        !content.contains("shutdown_engine"),
        "non-handle fixture emitted teardown:\n{content}"
    );
    assert!(!content.contains("common::runtime().block_on(async {"));
}

#[test]
fn rust_codegen_keeps_async_call_awaited_before_handle_teardown() {
    let content = generate_handle_teardown_fixture(true, true, false);

    assert!(
        content.contains("let result = scrape(&engine, url).await.expect(\"call failed\");"),
        "async call lost await:\n{content}"
    );
    assert_eq!(content.matches("shutdown_engine(engine).await;").count(), 1);
}

#[test]
fn rust_codegen_retains_successful_error_path_handle_and_tears_it_down_after_assertion() {
    let content = generate_handle_teardown_fixture(true, false, true);
    let assertion = content
        .find("assert!(result.is_err()")
        .expect("error assertion is emitted");
    let teardown = content
        .find("shutdown_engine(engine).await;")
        .expect("handle teardown is emitted");

    assert!(
        content.contains("Err(e) => (Err(e), None),"),
        "failed construction must retain no handle:\n{content}"
    );
    assert!(
        content.contains("(result, Some(engine))"),
        "successful construction must retain its handle:\n{content}"
    );
    assert!(
        content.contains("if let Some(engine) = engine {"),
        "teardown must skip a failed construction:\n{content}"
    );
    assert!(
        assertion < teardown,
        "teardown must follow the error assertion:\n{content}"
    );
    assert_eq!(content.matches("shutdown_engine(engine).await;").count(), 1);
    assert!(
        content.contains("let result = scrape(&engine, url);") && !content.contains("scrape(&engine, url).await"),
        "sync error-path call itself must not be awaited:\n{content}"
    );
}

#[test]
fn rust_handle_teardown_rejects_blank_malformed_and_keyword_values() {
    for invalid in ["", "   ", "shutdown-engine", "shutdown_engine(); panic!()", "await"] {
        let toml_src = format!(
            r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e.call.overrides.rust]
handle_teardown = {invalid:?}
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config syntax parses");
        let resolved = cfg.resolve().expect("config resolves before semantic validation");
        let error = alef::core::config::validation::validate_resolved(&resolved[0])
            .expect_err("invalid teardown identifier must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("handle_teardown"),
            "error must name the field, got: {message}"
        );
        assert!(
            message.contains(invalid),
            "error must name the invalid value, got: {message}"
        );
    }
}

#[test]
fn rust_handle_teardown_accepts_bare_non_keyword_identifier() {
    let toml_src = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e.call.overrides.rust]
handle_teardown = "shutdown_engine"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config syntax parses");
    let resolved = cfg.resolve().expect("config resolves before semantic validation");

    alef::core::config::validation::validate_resolved(&resolved[0])
        .expect("bare non-keyword teardown identifier must validate");
}

fn generate_named_call_with_default_teardown(named_teardown: Option<&str>) -> String {
    let named_override = named_teardown.map_or_else(String::new, |function| {
        format!(
            r#"
[crates.e2e.calls.crawl.overrides.rust]
handle_teardown = {function:?}
"#
        )
    });
    let toml_src = format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "scrape"
module = "mylib"
args = [{{ name = "engine", field = "config", type = "handle" }}]

[crates.e2e.call.overrides.rust]
handle_teardown = "shutdown_engine"

[crates.e2e.calls.crawl]
function = "crawl"
module = "mylib"
args = [{{ name = "engine", field = "config", type = "handle" }}]
{named_override}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config exists");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let group = FixtureGroup {
        category: "crawl".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "named_crawl".to_string(),
            category: Some("crawl".to_string()),
            description: "named call inherits the default teardown".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("crawl".to_string()),
            input: serde_json::json!({ "config": null }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }],
    };

    RustE2eCodegen
        .generate(&[group], &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds")
        .into_iter()
        .find(|file| file.path.to_string_lossy().contains("crawl_test.rs"))
        .expect("crawl_test.rs is emitted")
        .content
}

#[test]
fn rust_named_call_inherits_default_handle_teardown() {
    let content = generate_named_call_with_default_teardown(None);

    assert!(
        content.contains("use mylib::{crawl, shutdown_engine};"),
        "default teardown import is missing:\n{content}"
    );
    assert_eq!(content.matches("shutdown_engine(engine).await;").count(), 1);
}

#[test]
fn rust_named_call_can_override_default_handle_teardown() {
    let content = generate_named_call_with_default_teardown(Some("close_engine"));

    assert!(
        content.contains("use mylib::{close_engine, crawl};"),
        "named teardown import is missing:\n{content}"
    );
    assert_eq!(content.matches("close_engine(engine).await;").count(), 1);
    assert!(
        !content.contains("shutdown_engine"),
        "default teardown must be replaced:\n{content}"
    );
}
