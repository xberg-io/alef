//! Verifies the Dart e2e codegen emits the `mime_type` argument *positionally*
//! for facade extract methods. Regression test for a bug where an optional
//! `mime_type` string arg was emitted as a Dart named argument (`mimeType:`),
//! but the generated `SampleCrateBridge` facade declares it as a required
//! positional parameter — `extractBytesSync(Uint8List content, String mimeType,
//! [Config? config])` — producing either a "too few positional
//! arguments" error or invalid `named-then-positional` Dart syntax.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("code".to_string()),
        description: "extract file sync test fixture".to_string(),
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
        source: "code.json".to_string(),
        http: None,
    }
}

fn make_group(id: &str, input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "code".to_string(),
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
function = "extract_file_sync"
result_var = "result"

[[crates.e2e.call.args]]
name = "path"
field = "input.path"
type = "file_path"

[[crates.e2e.call.args]]
name = "mime_type"
field = "input.mime_type"
type = "string"
optional = true
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
        .find(|f| f.path.to_string_lossy().contains("code_test.dart"))
        .expect("code_test.dart is emitted")
        .content
        .clone()
}

/// An explicit `mime_type` value in the fixture must be emitted positionally,
/// not as the named argument `mimeType:`. Source-code file fixtures stay on the
/// file facade so extension-based language detection can run in the core.
#[test]
fn mime_type_string_arg_emitted_positionally_for_facade_extract() {
    let rendered = render(
        "code_shebang_detection",
        serde_json::json!({ "path": "code/script.sh", "mime_type": "text/x-source-code" }),
    );

    assert!(
        rendered.contains("'text/x-source-code'"),
        "must emit the mime type literal. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("mimeType: 'text/x-source-code'"),
        "mime_type must NOT be emitted as a named argument for the facade extract method. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("extractFileSync("),
        "source-code file_path arg must keep extractFileSync. Rendered:\n{rendered}"
    );
}

/// When the fixture omits `mime_type`, the inferred MIME type must also be
/// emitted positionally for the facade extract method.
#[test]
fn inferred_mime_type_emitted_positionally_for_facade_extract() {
    let rendered = render("code_no_mime", serde_json::json!({ "path": "code/script.sh" }));

    assert!(
        !rendered.contains("mimeType:"),
        "inferred mime_type must NOT be emitted as a named argument. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("extractFileSync("),
        "source-code file_path arg must keep extractFileSync. Rendered:\n{rendered}"
    );
}
