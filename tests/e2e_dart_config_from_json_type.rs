use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo_document"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "demo_document"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
module = "DemoDocument"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "input", field = "payload", type = "json_object", element_type = "ExtractInput" },
  { name = "config", field = "config", type = "json_object", optional = true },
]

[crates.e2e.call.overrides.csharp]
options_type = "ExtractionConfig"

[crates.e2e.calls.extract_batch]
function = "extract_batch"
module = "DemoDocument"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "inputs", field = "inputs", type = "json_object", element_type = "ExtractInput" },
  { name = "config", field = "config", type = "json_object", optional = true },
]

[crates.e2e.calls.extract_batch.overrides.csharp]
options_type = "ExtractionConfig"
"#;

fn fixture(id: &str, call: Option<&str>, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("url".to_string()),
        description: format!("{id} fixture"),
        call: call.map(str::to_string),
        input,
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
        source: "url.json".to_string(),
        ..Fixture::default()
    }
}

fn render() -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "url".to_string(),
        fixtures: vec![
            fixture(
                "url_remote_document",
                None,
                serde_json::json!({
                    "payload": {"kind": "uri", "uri": "$mock_url"},
                    "config": {"mode": "document"}
                }),
            ),
            fixture(
                "url_batch_documents",
                Some("extract_batch"),
                serde_json::json!({
                    "inputs": [
                        {"kind": "uri", "uri": "$mock_url"},
                        {"kind": "bytes", "bytes": [111, 107], "mime_type": "text/plain"}
                    ],
                    "config": {"mode": "document"}
                }),
            ),
        ],
    }];
    let files = DartE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("dart generation succeeds");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("url_test.dart"))
        .expect("url_test.dart is emitted")
        .content
        .clone()
}

#[test]
fn dart_url_and_batch_config_json_use_compatible_options_type() {
    let content = render();

    assert_eq!(
        content.matches("createExtractionConfigFromJson").count(),
        2,
        "URL and batch config JSON args must use the configured type. Generated:\n{content}"
    );
    assert!(
        !content.contains("createConfigFromJson"),
        "config JSON args must not fall back to the arg name. Generated:\n{content}"
    );
}
