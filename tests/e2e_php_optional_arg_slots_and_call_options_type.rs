//! Regression tests for two PHP e2e codegen bugs.
//!
//! Bug 1 — optional middle argument slot collapse.
//!   A call like `extractFile(string $path, ?string $mime_type, ?Config $config)`
//!   whose fixture omits `mime_type` but whose trailing `config` arg always emits
//!   a default (`Config::from_json('{}')`) must keep the `mime_type` slot as an
//!   explicit `null`. Pre-fix, the optional-arg emission probe ignored the
//!   "json_object config always emits" special case, dropped the `mime_type`
//!   slot, and shifted `config` into argument #2 — producing a PHP `TypeError`.
//!
//! Bug 2 — call-level `options_type` ignored.
//!   `[e2e.calls.<name>].options_type` declares the config parameter type
//!   once for every binding. Pre-fix, `CallConfig` had
//!   no such field, so the value was silently dropped and PHP fell back to the
//!   arg-name heuristic, constructing the wrong config type for the parameter.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

fn render(toml_src: &str, fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];
    let files = PhpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("PHP codegen succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Test.php"))
        .expect("a *Test.php file is emitted")
        .content
        .clone()
}

fn smoke_fixture(input: serde_json::Value) -> Fixture {
    Fixture {
        id: "smoke_case".to_string(),
        category: Some("smoke".to_string()),
        description: "smoke".to_string(),
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
        assertions: Vec::new(),
        source: "smoke/smoke_case.json".to_string(),
        http: None,
    }
}

/// Bug 1: a fixture omitting the optional `mime_type` arg must still emit an
/// explicit `null` for that slot, so the always-emitted `config` default lands
/// in argument #3 — never argument #2.
#[test]
fn omitted_optional_mime_type_keeps_explicit_null_slot() {
    let toml_src = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_file"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
args = [
  { name = "path", field = "input.path", type = "file_path" },
  { name = "mime_type", field = "input.mime_type", type = "string", optional = true },
  { name = "config", field = "input.config", type = "json_object", optional = true },
]
"#;
    let content = render(toml_src, smoke_fixture(serde_json::json!({ "path": "doc.pdf" })));
    assert!(
        content.contains("extractFile(\"doc.pdf\", null, "),
        "expected `mime_type` slot emitted as explicit `null` before the config default; got:\n{content}"
    );
    assert!(
        !content.contains("extractFile(\"doc.pdf\", ExtractionConfig"),
        "config object must not be shifted into the `mime_type` slot; got:\n{content}"
    );
}

/// Bug 2: a call-level `options_type` must drive the PHP config constructor type
/// even when no per-language override sets it.
#[test]
fn call_level_options_type_drives_php_config_constructor() {
    let toml_src = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "embed_texts_async"
module = "MyLib"
result_var = "result"
async = true
returns_result = true
options_type = "EmbeddingConfig"
args = [
  { name = "texts", field = "input.texts", type = "json_object", owned = true, element_type = "String" },
  { name = "config", field = "input.config", type = "json_object", optional = true },
]
"#;
    let content = render(toml_src, smoke_fixture(serde_json::json!({ "texts": [] })));
    assert!(
        content.contains("EmbeddingConfig::from_json('{}')"),
        "call-level options_type must yield EmbeddingConfig::from_json('{{}}'); got:\n{content}"
    );
    assert!(
        !content.contains("ExtractionConfig::from_json"),
        "PHP must not fall back to ExtractionConfig when options_type is EmbeddingConfig; got:\n{content}"
    );
    assert!(
        content.contains("use SampleCrate\\EmbeddingConfig;") || content.contains("EmbeddingConfig;"),
        "the resolved options_type must be imported; got:\n{content}"
    );
}
