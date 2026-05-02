//! Regression test: Go e2e codegen now supports loading bytes arguments from
//! file paths, not just base64-encoded inline values or HTTP fixtures.
//!
//! Previously, any fixture with a bytes argument that contained a file path
//! (e.g. `"pdf/memo.pdf"`) would be skipped with:
//!   t.Skip("non-HTTP fixture: Go binding does not expose a callable...")
//!
//! The codegen now detects file paths in bytes arguments and emits:
//!   pdfBytes, err := os.ReadFile("pdf/memo.pdf")
//!   if err != nil { t.Fatalf("failed to read fixture file...") }
//!
//! This allows fixtures with file path bytes arguments to run as real tests.

use alef_core::config::AlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::go::GoCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_go_config_with_bytes_call() -> AlefConfig {
    let toml_src = r#"
languages = ["go"]

[crate]
name = "mylib"
sources = ["src/lib.rs"]

[e2e]
fixtures = "fixtures"
output = "e2e"

[e2e.call]
function = "extract_text_from_pdf"
module = "github.com/example/mylib"
result_var = "result"
returns_result = true
args = [
  { name = "pdf_bytes", field = "input.data", type = "bytes" },
]

[e2e.call.overrides.go]
function = "ExtractTextFromPDF"
module = "github.com/example/mylib"
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_file_path_bytes_fixture(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "Go fixture with file path bytes argument".to_string(),
            tags: Vec::new(),
            skip: None,
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
    }
}

fn build_inline_text_bytes_fixture(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "Go fixture with inline text bytes argument".to_string(),
            tags: Vec::new(),
            skip: None,
            call: None,
            input: serde_json::json!({ "data": "<!DOCTYPE html><html><body>test</body></html>" }),
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

fn build_base64_bytes_fixture(id: &str) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "Go fixture with base64 bytes argument".to_string(),
            tags: Vec::new(),
            skip: None,
            call: None,
            input: serde_json::json!({ "data": "/9j/4AAQSkZJRg==" }),
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
fn go_codegen_loads_bytes_arg_from_file_path_value() {
    let config = build_go_config_with_bytes_call();
    let groups = vec![build_file_path_bytes_fixture("file_path_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    let content = &test_file.content;

    // Must emit a real test (not skipped).
    assert!(
        content.contains("func Test_FilePath_Bytes(t *testing.T)"),
        "test function not emitted: {content}"
    );
    assert!(
        !content.contains("t.Skip(\"non-HTTP fixture"),
        "test should not be skipped: {content}"
    );

    // Must emit os.ReadFile for the file path.
    assert!(
        content.contains("os.ReadFile("),
        "os.ReadFile(...) for file path not emitted: {content}"
    );
    assert!(
        content.contains("pdf/fake_memo.pdf"),
        "file path not in generated code: {content}"
    );
    assert!(
        content.contains("if err != nil { t.Fatalf("),
        "error handling for file read not emitted: {content}"
    );

    // Must emit the real function call.
    assert!(
        content.contains("ExtractTextFromPDF("),
        "real function call missing: {content}"
    );
}

#[test]
fn go_codegen_emits_bytes_literal_for_inline_text_argument() {
    let config = build_go_config_with_bytes_call();
    let groups = vec![build_inline_text_bytes_fixture("inline_text_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    let content = &test_file.content;

    // Must emit a real test (not skipped).
    assert!(
        content.contains("func Test_InlineText_Bytes(t *testing.T)"),
        "test function not emitted: {content}"
    );
    assert!(
        !content.contains("t.Skip(\"non-HTTP fixture"),
        "test should not be skipped: {content}"
    );

    // Must emit []byte(...) for inline text.
    assert!(
        content.contains("[]byte("),
        "[]byte(...) for inline text not emitted: {content}"
    );
    assert!(
        content.contains("DOCTYPE"),
        "HTML content not in generated code: {content}"
    );

    // Must not try to read from file or decode base64.
    assert!(
        !content.contains("os.ReadFile("),
        "file read should not be emitted for inline text: {content}"
    );
    assert!(
        !content.contains("base64.StdEncoding.DecodeString("),
        "base64 decode should not be emitted for inline text: {content}"
    );
}

#[test]
fn go_codegen_emits_base64_decode_for_opaque_bytes_argument() {
    let config = build_go_config_with_bytes_call();
    let groups = vec![build_base64_bytes_fixture("base64_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    let content = &test_file.content;

    // Must emit a real test (not skipped).
    assert!(
        content.contains("func Test_Base64_Bytes(t *testing.T)"),
        "test function not emitted: {content}"
    );
    assert!(
        !content.contains("t.Skip(\"non-HTTP fixture"),
        "test should not be skipped: {content}"
    );

    // Must emit base64.StdEncoding.DecodeString for opaque bytes.
    assert!(
        content.contains("base64.StdEncoding.DecodeString("),
        "base64 decode not emitted: {content}"
    );
    assert!(
        content.contains("/9j/4AAQSkZJRg=="),
        "base64 content not in generated code: {content}"
    );

    // Must not try to read from file or emit raw bytes literal.
    assert!(
        !content.contains("os.ReadFile("),
        "file read should not be emitted for base64: {content}"
    );
    assert!(
        content.lines().filter(|l| l.contains("[]byte(") && !l.contains("base64") && !l.contains("StdEncoding")).count() == 0,
        "raw bytes literal should not be emitted for base64: {content}"
    );
}

#[test]
fn go_codegen_needs_os_import_only_when_file_path_bytes_present() {
    // File path fixture needs "os"
    let config = build_go_config_with_bytes_call();
    let groups = vec![build_file_path_bytes_fixture("file_path_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        test_file.content.contains("\"os\""),
        "\"os\" import missing for file path fixture"
    );

    // Inline text fixture does not need "os"
    let groups = vec![build_inline_text_bytes_fixture("inline_text_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        !test_file.content.contains("\"os\""),
        "\"os\" import should not be present for inline text fixture"
    );

    // Base64 fixture does not need "os"
    let groups = vec![build_base64_bytes_fixture("base64_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        !test_file.content.contains("\"os\""),
        "\"os\" import should not be present for base64 fixture"
    );
}

#[test]
fn go_codegen_needs_base64_import_only_when_base64_bytes_present() {
    // Base64 fixture needs "encoding/base64"
    let config = build_go_config_with_bytes_call();
    let groups = vec![build_base64_bytes_fixture("base64_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        test_file.content.contains("\"encoding/base64\""),
        "\"encoding/base64\" import missing for base64 fixture"
    );

    // File path fixture does not need "encoding/base64"
    let groups = vec![build_file_path_bytes_fixture("file_path_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        !test_file.content.contains("\"encoding/base64\""),
        "\"encoding/base64\" import should not be present for file path fixture"
    );

    // Inline text fixture does not need "encoding/base64"
    let groups = vec![build_inline_text_bytes_fixture("inline_text_bytes")];
    let files = GoCodegen
        .generate(&groups, &config.e2e.clone().unwrap(), &config)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.go"))
        .expect("smoke_test.go is emitted");
    assert!(
        !test_file.content.contains("\"encoding/base64\""),
        "\"encoding/base64\" import should not be present for inline text fixture"
    );
}
