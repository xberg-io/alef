//! Verifies that Kotlin e2e codegen handles `json_object` array arguments
//! with `element_type = "String"` correctly.
//!
//! Background: the kotlin_android codegen had special handling for typed arrays
//! that tried to read each element as a file path and wrap it in a type constructor.
//! This was incorrect for plain string arrays: if the fixture contains
//! `["First", "Second"]`, they should be emitted as `listOf("First", "Second")`
//! not as file-reading constructors like `String(Files.readAllBytes(...), Charset)`.
//!
//! Regression: Kotlin e2e codegen emitted incorrect code like:
//!   ```kotlin
//!   String(java.nio.file.Files.readAllBytes(...), "application/octet-stream")
//!   ```
//! which fails with type mismatch (String literals don't take Charset as second arg).

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::kotlin::KotlinE2eCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

fn make_fixture_with_string_array() -> FixtureGroup {
    FixtureGroup {
        category: "embed_texts".to_string(),
        fixtures: vec![Fixture {
            id: "string_array_basic".to_string(),
            category: Some("embed_texts".to_string()),
            description: "embed texts: basic string array".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({
                "texts": ["First", "Second"]
            }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: BTreeMap::new(),
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![],
            source: "embed_texts.json".to_string(),
            http: None,
        }],
    }
}

fn base_toml() -> &'static str {
    r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "embed_texts"
module = "dev.sample.SampleLib"
result_var = "result"

[crates.e2e.call.overrides.kotlin]
class = "SampleLib"
function = "embedTexts"

[[crates.e2e.call.args]]
name = "texts"
field = "input.texts"
type = "json_object"
element_type = "String"
"#
}

#[test]
fn string_array_arg_with_element_type_does_not_wrap_in_constructor() {
    let cfg: NewAlefConfig = toml::from_str(base_toml()).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_fixture_with_string_array()];
    let files = KotlinE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".kt"))
        .expect("test file should be emitted");
    let content = &test_file.content;

    // Bug repro: broken codegen emitted `String(Files.readAllBytes(...), "...")`.
    // This fails because String(bytes, charset) constructor expects Charset type, not String.
    assert!(
        !content.contains("Files.readAllBytes"),
        "must NOT try to read string array elements as file paths. Rendered:\n{content}"
    );

    assert!(
        !content.contains("String(java.nio.file"),
        "must NOT wrap strings in file-reading constructors. Rendered:\n{content}"
    );

    // Sanity: the argument should be a simple listOf(...) with the string literals.
    assert!(
        content.contains("listOf(\"First\", \"Second\")"),
        "must emit string literals as a simple listOf. Rendered:\n{content}"
    );
}
