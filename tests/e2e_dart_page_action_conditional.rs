//! Verifies the Dart e2e codegen only emits PageAction helper functions when actually used.
//! Regression test for a bug where _parsePageAction was emitted unconditionally even when no
//! fixture used PageAction, causing compilation errors with "Type 'PageAction' not found".

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("code".to_string()),
        description: "test fixture without PageAction".to_string(),
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

fn make_group(category: &str, id: &str, input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: category.to_string(),
        fixtures: vec![make_fixture(id, input)],
    }
}

const TOML_NO_PAGE_ACTION: &str = r#"
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
function = "extract_bytes"
result_var = "result"

[[crates.e2e.call.args]]
name = "content"
field = "input.bytes"
type = "bytes"
"#;

fn render_no_page_action(fixture_id: &str, input: serde_json::Value) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML_NO_PAGE_ACTION).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group("code", fixture_id, input)];
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

/// PageAction helper should not be emitted when no fixture uses PageAction.
#[test]
fn page_action_not_emitted_when_not_used() {
    let rendered = render_no_page_action(
        "code_shebang_detection",
        serde_json::json!({
            "bytes": "code/script.sh",
        }),
    );

    assert!(
        !rendered.contains("PageAction _parsePageAction"),
        "_parsePageAction should not be emitted when not used. Rendered:\n{rendered}"
    );

    assert!(
        !rendered.contains("ScrollDirection"),
        "ScrollDirection should not be emitted when not used. Rendered:\n{rendered}"
    );

    assert!(
        rendered.contains("String _alefE2eText(Object? value)"),
        "_alefE2eText helper should still be emitted. Rendered:\n{rendered}"
    );

    assert!(
        !rendered.contains("import 'dart:convert';"),
        "dart:convert should not be imported when not needed. Rendered:\n{rendered}"
    );
}
