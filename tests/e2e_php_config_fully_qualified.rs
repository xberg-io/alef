//! Regression test: verify that PHP e2e codegen fully qualifies config type names
//! to the binding namespace (e.g. `\Mylib\ExtractionConfig`), not bare names.
//!
//! A bare `ExtractionConfig` is resolved against the *test* namespace (`Mylib\E2e`), where no
//! such class exists, so an unqualified reference is a "Class not found" fatal the moment PHPUnit
//! runs the emitted file — never a codegen-time error.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

const TOML_EXTRACT_FILE: &str = r#"
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
options_type = "ExtractionConfig"
args = [
  { name = "path", field = "input.path", type = "file_path" },
  { name = "config", field = "input.config", type = "json_object", optional = true },
]
"#;

fn render(toml_src: &str, fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];
    let files = PhpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
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
        docs: None,
        requirements: Vec::new(),
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
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
    }
}

/// Every `::from_json` construction of a config type in an emitted PHP test must carry a
/// leading `\` and the binding namespace. PHPUnit resolves a bare `ExtractionConfig` against
/// the *test* namespace (`SampleCrate\E2e`), where no such class exists, so an unqualified
/// reference is a runtime "Class not found", not a compile-time error any lint would catch.
fn assert_every_config_construction_is_namespace_qualified(content: &str) {
    let unqualified: Vec<&str> = content
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("::from_json("))
        .filter(|line| !line.contains("\\ExtractionConfig::from_json("))
        .collect();
    assert!(
        unqualified.is_empty(),
        "every config construction must be namespace-qualified as \\<Namespace>\\ExtractionConfig::from_json(...); \
         unqualified line(s): {unqualified:?}\nfull output:\n{content}"
    );
}

/// The fixture omits `config` entirely, so codegen supplies the default
/// `\<Namespace>\ExtractionConfig::from_json('{}')`. This is the emission site that produced
/// the original "Class not found" failure.
#[test]
fn php_config_types_are_namespace_qualified() {
    let content = render(
        TOML_EXTRACT_FILE,
        smoke_fixture(serde_json::json!({ "path": "doc.pdf" })),
    );

    assert!(
        content.contains("::from_json('{}')"),
        "the omitted optional config must still emit a default construction; got:\n{content}"
    );
    assert_every_config_construction_is_namespace_qualified(&content);
}

/// The same qualification must hold on the other emission path: a config the fixture *does*
/// supply, which is constructed from the fixture's JSON rather than from `'{}'`.
#[test]
fn php_config_from_fixture_json_is_namespace_qualified() {
    let content = render(
        TOML_EXTRACT_FILE,
        smoke_fixture(serde_json::json!({ "path": "doc.pdf", "config": { "use_cache": true } })),
    );

    assert!(
        content.contains("::from_json("),
        "a supplied config must be constructed via from_json; got:\n{content}"
    );
    assert_every_config_construction_is_namespace_qualified(&content);
}
