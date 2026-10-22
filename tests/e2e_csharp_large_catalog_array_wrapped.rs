//! Regression test for task #365: alef's C# e2e generator used to emit a large fixture
//! array (e.g. a "catalog" of test data) as a single unwrapped line — `new List<T>() { a, b,
//! c, ... }` joined with no newlines. For a large fixture array that produced one line tens
//! of thousands of characters long, forcing `poly fmt`'s clang-format engine to reflow the
//! whole thing from scratch instead of receiving output that was already close to its final
//! shape.
//!
//! The fix wraps collection literals above `CSHARP_COLLECTION_INLINE_LIMIT` elements one
//! element per line via `csharp/wrapped_collection_literal.jinja`
//! (`src/e2e/codegen/csharp/values.rs::render_collection_literal`). This test asserts a
//! concrete measurable property of the generated output — the maximum line length stays
//! below a named constant — rather than merely checking the file is non-empty.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::csharp::CSharpCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, MockResponse};
use std::collections::BTreeMap;

/// Number of elements in the fixture's catalog array. Comfortably above
/// `CSHARP_COLLECTION_INLINE_LIMIT` (8) so the wrapped path is actually exercised, and large
/// enough that an unwrapped line would be unambiguously pathological (tens of thousands of
/// characters) if the wrapping regressed.
const CATALOG_ELEMENT_COUNT: usize = 500;

/// Upper bound on any single generated line's length. A wrapped, one-element-per-line
/// collection literal keeps every catalog-array line under roughly 90 characters (indent +
/// one `JsonSerializer.Deserialize<String>("...", ConfigOptions)!,` element). The longest
/// line in an otherwise-unrelated part of the file is the fixed, catalog-size-independent
/// `ConfigOptions` field declaration (measured at 227 characters); this constant sits just
/// above that while remaining orders of magnitude below the tens-of-thousands of characters
/// an unwrapped catalog line would reach.
const MAX_ALLOWED_LINE_LENGTH: usize = 260;

fn make_fixture_with_catalog_array(n: usize) -> FixtureGroup {
    let items: Vec<String> = (0..n).map(|i| format!("item-{i:05}")).collect();
    FixtureGroup {
        category: "contract".to_string(),
        fixtures: vec![Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "large_catalog".to_string(),
            category: Some("contract".to_string()),
            description: "contract: large catalog array".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "texts": items }),
            mock_response: Some(MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: BTreeMap::new(),
                delay_ms: None,
                stream_chunk_delay_ms: None,
            }),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![],
            source: "contract.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            client_options: None,
            preserve_input_urls: false,
        }],
    }
}

fn base_toml() -> &'static str {
    r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.csharp]
namespace = "Sample.Lib"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "embed_texts"
module = "Sample.Lib.SampleLib"
result_var = "result"

[crates.e2e.call.overrides.csharp]
class = "SampleLib"
function = "EmbedTexts"

[[crates.e2e.call.args]]
name = "texts"
field = "input.texts"
type = "json_object"
element_type = "String"
"#
}

fn generate_contract_tests_cs() -> String {
    let cfg: NewAlefConfig = toml::from_str(base_toml()).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_fixture_with_catalog_array(CATALOG_ELEMENT_COUNT)];
    let files = CSharpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation succeeds");

    files
        .into_iter()
        .find(|f| f.path.to_string_lossy().ends_with("Tests.cs"))
        .expect("test file should be emitted")
        .content
}

#[test]
fn large_catalog_array_is_wrapped_not_one_giant_line() {
    let content = generate_contract_tests_cs();

    let max_line_len = content.lines().map(str::len).max().unwrap_or(0);
    assert!(
        max_line_len <= MAX_ALLOWED_LINE_LENGTH,
        "expected every generated line to be at most {MAX_ALLOWED_LINE_LENGTH} characters, \
         but the longest line was {max_line_len} characters. A {CATALOG_ELEMENT_COUNT}-element \
         catalog array must be wrapped one element per line, not emitted as a single unwrapped \
         line. Rendered:\n{content}"
    );

    // The wrapped literal still renders every element via the same
    // `JsonSerializer.Deserialize<String>` path as before — wrapping must not drop elements.
    let deserialize_call_count = content.matches("JsonSerializer.Deserialize<String>(").count();
    assert_eq!(
        deserialize_call_count, CATALOG_ELEMENT_COUNT,
        "expected all {CATALOG_ELEMENT_COUNT} catalog elements to still be rendered, found {deserialize_call_count}"
    );

    // Sanity: the array literal actually spans multiple lines (not collapsed some other way).
    assert!(
        content.contains("new List<String>()\n"),
        "expected the wrapped collection literal to open on its own line before the elements. Rendered:\n{content}"
    );
}
