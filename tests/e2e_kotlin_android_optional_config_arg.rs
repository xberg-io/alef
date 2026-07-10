//! Regression test for kotlin_android e2e codegen bug:
//!
//! - Null cannot be a value of a non-null config type:
//!   kotlin_android binding signatures declare config as non-nullable,
//!   but the e2e codegen was passing null when the fixture omitted the config arg.
//!   The fix: emit a default constructor call instead of `null`.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin_android::KotlinAndroidE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use std::path::{Path, PathBuf};

fn make_extract_bytes_fixture(id: &str, has_config: bool) -> Fixture {
    let mut input = serde_json::json!({
        "data": "pdf/fake_memo.pdf",
        "mime_type": "application/pdf"
    });
    if has_config {
        input["config"] = serde_json::json!({
            "use_cache": true
        });
    }
    Fixture {
        id: id.to_string(),
        category: Some("async".to_string()),
        description: "extract_bytes test".to_string(),
        tags: vec!["async".to_string()],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: Some("extract_bytes".to_string()),
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
        source: "async.json".to_string(),
        http: None,
    }
}

const TOML_EXTRACT_BYTES: &str = r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_bytes"
result_var = "result"

[crates.e2e.calls.extract_bytes]
function = "extract_bytes"
result_var = "result"
async = true

[[crates.e2e.calls.extract_bytes.args]]
name = "content"
field = "input.data"
type = "bytes"

[[crates.e2e.calls.extract_bytes.args]]
name = "mimeType"
field = "input.mime_type"
type = "string"

[[crates.e2e.calls.extract_bytes.args]]
name = "config"
field = "input.config"
type = "json_object"
optional = true

[crates.e2e.calls.extract_bytes.overrides.kotlin_android]
options_type = "ExtractionConfig"

[crates.e2e.packages.kotlin_android]
name = "sample_crate"
"#;

fn render_kotlin_android_test(toml: &str, fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "async".to_string(),
        fixtures: vec![fixture],
    }];
    let files = KotlinAndroidE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    let async_test_path = kotlin_android_async_test_path();
    files
        .iter()
        .find(|f| f.path.ends_with(&async_test_path))
        .expect("AsyncTest.kt is emitted")
        .content
        .clone()
}

fn kotlin_android_async_test_path() -> PathBuf {
    Path::new("src")
        .join("test")
        .join("kotlin")
        .join("dev")
        .join("sample_crate")
        .join("e2e")
        .join("AsyncTest.kt")
}

/// Regression: when a fixture omits the optional config argument, kotlin_android
/// codegen must emit the default constructor instead of `null`, since the binding
/// signature declares config as non-nullable.
#[test]
fn kotlin_android_optional_config_arg_emits_default_constructor_not_null() {
    let fixture = make_extract_bytes_fixture("async_extract_bytes", false);
    let rendered = render_kotlin_android_test(TOML_EXTRACT_BYTES, fixture);

    assert!(
        rendered.contains("ExtractionConfig()"),
        "must emit ExtractionConfig() for optional config arg with no value; got:\n{rendered}"
    );

    let lines: Vec<&str> = rendered
        .lines()
        .filter(|line| line.contains("SampleCrate.extractBytes(") || line.contains(".extractBytes("))
        .collect();
    for line in lines {
        assert!(
            !line.contains(", null)") && !line.contains(", null,"),
            "must NOT pass null as config parameter; got line:\n{line}"
        );
    }
}

/// Regression: when a fixture provides a config value, it should be deserialized
/// and used (normal path, not the null-replacement path).
#[test]
fn kotlin_android_optional_config_arg_with_value_uses_deserialized_config() {
    let fixture = make_extract_bytes_fixture("async_extract_bytes_with_config", true);
    let rendered = render_kotlin_android_test(TOML_EXTRACT_BYTES, fixture);

    assert!(
        rendered.contains("MAPPER.readValue(") && rendered.contains("ExtractionConfig"),
        "must deserialize ExtractionConfig from fixture value; got:\n{rendered}"
    );
}
