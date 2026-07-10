//! Verifies the Dart e2e codegen emits scalar integer arguments correctly
//! alongside bytes arguments. Regression test for a bug where integer
//! arguments following bytes arguments were dropped entirely, producing missing
//! required argument errors at Dart compile time.
//!
//! Example: `SampleCrateBridge.renderPdfPageToPng(bytes)` should be
//! `SampleCrateBridge.renderPdfPageToPng(bytes, pageIndex)`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("pdf".to_string()),
        description: "render PDF page test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input,
        mock_response: None,
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "pdf.json".to_string(),
        http: None,
    }
}

fn make_group(id: &str, input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "pdf".to_string(),
        fixtures: vec![make_fixture(id, input)],
    }
}

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_crate"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "render_pdf_page_to_png"
result_var = "result"

[[crates.e2e.call.args]]
name = "pdf_bytes"
field = "input.pdf_bytes"
type = "bytes"

[[crates.e2e.call.args]]
name = "page_index"
field = "input.page_index"
type = "int"
"#;

fn render(fixture_id: &str, input: serde_json::Value) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id, input)];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("pdf_test.dart"))
        .expect("pdf_test.dart is emitted")
        .content
        .clone()
}

/// Both bytes and int arguments must be emitted positionally, in order.
/// The bug was that the int argument was dropped entirely.
#[test]
fn bytes_and_int_args_both_emitted_positionally() {
    let rendered = render(
        "render_pdf_page_first",
        serde_json::json!({
            "pdf_bytes": "pdf/fake_memo.pdf",
            "page_index": 0,
        }),
    );

    assert!(
        rendered.contains("File('pdf/fake_memo.pdf').readAsBytesSync()"),
        "must emit the bytes argument. Rendered:\n{rendered}"
    );

    assert!(
        rendered.contains("renderPdfPageToPng(File('pdf/fake_memo.pdf').readAsBytesSync(), 0)"),
        "must emit both bytes and int arguments positionally in order. Rendered:\n{rendered}"
    );
}

/// Integer argument with non-zero value must be emitted correctly.
#[test]
fn int_arg_with_nonzero_value_emitted_correctly() {
    let rendered = render(
        "render_pdf_page_second",
        serde_json::json!({
            "pdf_bytes": "pdf/fake_memo.pdf",
            "page_index": 5,
        }),
    );

    assert!(
        rendered.contains("renderPdfPageToPng(File('pdf/fake_memo.pdf').readAsBytesSync(), 5)"),
        "must emit the correct page_index value. Rendered:\n{rendered}"
    );
}
