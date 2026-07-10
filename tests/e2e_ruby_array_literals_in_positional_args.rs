//! Verifies the Ruby e2e codegen properly renders array literals from fixture data
//! in positional arguments when element_type is set.
//!
//! Pre-fix, when element_type="String" and the fixture had `["First", "Second"]`,
//! the codegen would emit `[]` due to filtering for objects instead of strings.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::ruby::RubyCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "embed_texts"
module = "MyLib"
result_var = "result"
result_is_simple = true
result_is_array = true
async = false
returns_result = true
args = [
  { name = "texts", field = "input.texts", type = "json_object", element_type = "String" },
  { name = "config", field = "input.config", type = "json_object", optional = true },
]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn string_array_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "embeddings".to_string(),
        fixtures: vec![Fixture {
            id: "embed_strings_happy".to_string(),
            category: Some("embeddings".to_string()),
            description: "embed array of strings".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({
                "texts": ["First", "Second"],
                "config": null
            }),
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
            source: "embeddings/embed_strings_happy.json".to_string(),
            http: None,
        }],
    }
}

/// A fixture with array-of-strings argument must render the array elements,
/// not an empty array.
#[test]
fn ruby_array_literals_render_elements() {
    let (e2e, resolved) = build_config();
    let groups = vec![string_array_fixture_group()];
    let files = RubyCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let spec_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("embeddings_spec.rb"))
        .expect("Ruby embeddings_spec.rb is emitted");

    let body = &spec_file.content;

    assert!(
        body.contains("\"First\"") || body.contains("'First'"),
        "array should contain 'First' element, got:\n{body}"
    );
    assert!(
        body.contains("\"Second\"") || body.contains("'Second'"),
        "array should contain 'Second' element, got:\n{body}"
    );

    assert!(
        !body.contains("embed_texts([], ") && !body.contains("embed_texts_async([], "),
        "embed_texts should not be called with empty array [], got:\n{body}"
    );
}
