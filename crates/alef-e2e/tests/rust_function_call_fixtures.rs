//! Regression: rust e2e codegen used to short-circuit any fixture without `http`
//! or `mock_response` to a TODO stub, on the assumption that such fixtures were
//! schema/spec validation (asyncapi, grpc, graphql_schema, …) with no callable
//! Rust API. That assumption is wrong for libraries whose fixtures invoke a
//! plain function (e.g. `kreuzberg::extract_file(path, mime, config)`): every
//! such fixture would emit `// TODO: implement when a callable API is available`
//! instead of a real call, producing 0 effective rust e2e tests.
//!
//! The codegen now stubs only when the resolved call config has no function
//! name. Fixtures that point at a configured `[e2e.call]` (or `[e2e.calls.<n>]`)
//! render real function invocations.
//!
//! See: alef-e2e codegen rust render_test_function (the gate that checks
//! `function_name.is_empty()`).

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::rust::RustE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config_with_default_call() -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
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

fn build_config_without_function() -> (alef_e2e::config::E2eConfig, alef_core::config::ResolvedCrateConfig) {
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
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "regression: function-call fixture (no http, no mock)".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "path": "test.pdf" }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }
}

#[test]
fn rust_codegen_emits_real_call_for_function_fixture_without_http_or_mock() {
    let (e2e, resolved) = build_config_with_default_call();
    let groups = vec![build_function_call_fixture("function_call_fixture")];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
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
        "real `extract_file(...)` call missing — codegen still stubs to TODO:\n{content}"
    );
    assert!(
        !content.contains("TODO: implement when a callable API is available"),
        "TODO stub still emitted for a fixture with a configured call:\n{content}"
    );
}

#[test]
fn rust_codegen_loads_bytes_arg_from_file_path_value() {
    // Fixture has `input.data = "pdf/fake_memo.pdf"` and the call's `data` arg is
    // declared as `type = "bytes"`. The codegen must read the file from
    // test_documents instead of treating the path as inline bytes.
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
            id: "load_pdf".to_string(),
            category: Some("smoke".to_string()),
            description: "load a pdf by path".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "data": "pdf/fake_memo.pdf" }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
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
    // When [e2e.calls.<n>.args] sets owned = true on a `bytes` arg whose value
    // is a relative file path, the codegen must emit `let <name> = std::fs::read(...);`
    // followed by passing `<name>` by value (not `&<name>`). Mirror for string args.
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
            id: "owned_bytes_filepath".to_string(),
            category: Some("smoke".to_string()),
            description: "owned bytes arg loaded from test_documents".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "data": "images/hello.png" }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
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
    // detect_mime_type takes a `String` by value; with owned = true the codegen
    // must call `.to_string()` on the literal and pass by value.
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
            id: "owned_string_arg".to_string(),
            category: Some("smoke".to_string()),
            description: "owned string arg passed by value".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "path": "doc.pdf", "check_exists": false }),
            mock_response: None,
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                field: None,
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
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
    // Regression: imports were previously gated on mock_response; non-mock
    // function-call fixtures rendered without their `use mylib::extract_file;`
    // statement, producing E0425 "cannot find function".
    let (e2e, resolved) = build_config_with_default_call();
    let groups = vec![build_function_call_fixture("call_fixture")];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
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
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    let content = &test_file.content;

    assert!(
        content.contains("TODO: implement when a callable API is available"),
        "expected TODO stub when no function is configured, got:\n{content}"
    );
}
