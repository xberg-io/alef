//! Regression test: PHP e2e codegen must emit accessor extraction lines (the
//! `$<field> = $result-><getter>();` block that materializes array-typed result
//! fields into local variables for assertions) in a deterministic order across
//! regens.
//!
//! Pre-fix the codegen collected these bindings into a `HashMap<String, _>` and
//! iterated `.values()` directly, which leaked `RandomState`-randomized
//! iteration order into the generated PHP source. Concretely, sample_language_pack's
//! `e2e/php/tests/ProcessTest.php` flipped the relative order of `$imports`
//! and `$structure` between back-to-back `alef e2e generate` runs, producing
//! noisy diffs and breaking the `CI: regen leaves zero diff` invariant.
//!
//! The fix swaps the `HashMap` for a `BTreeMap` so iteration is sorted by
//! field name. This test:
//!   1. Asserts byte-equal output across two independent renders (catches
//!      non-determinism even when sort order is coincidentally stable).
//!   2. Asserts the bindings appear in lexicographic field-name order
//!      (`$imports` before `$structure`) to pin the chosen ordering and
//!      prevent a regression to insertion-order or hashed order.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config() -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_array = ["imports", "structure"]
result_fields = ["imports", "structure"]

[crates.e2e.call]
function = "process"
module = "MyLib"
result_var = "result"
async = false
returns_result = true
args = [
  { name = "source", field = "input.source", type = "string" },
]

[crates.e2e.call.overrides.php]
module = "MyLib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn build_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_two_arrays".to_string(),
            category: Some("smoke".to_string()),
            description: "result exposes two array fields".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({ "source": "int main() {}" }),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: vec![
                Assertion {
                    assertion_type: "length".to_string(),
                    field: Some("structure".to_string()),
                    value: Some(serde_json::json!(1)),
                    values: None,
                    method: None,
                    check: None,
                    args: None,
                    return_type: None,
                },
                Assertion {
                    assertion_type: "length".to_string(),
                    field: Some("imports".to_string()),
                    value: Some(serde_json::json!(0)),
                    values: None,
                    method: None,
                    check: None,
                    args: None,
                    return_type: None,
                },
            ],
            source: "smoke/smoke_two_arrays.json".to_string(),
            http: None,
        }],
    }
}

fn render_once() -> String {
    let (e2e, resolved) = build_config();
    let groups = vec![build_fixture_group()];
    let files = PhpCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("PHP codegen succeeds");
    let test_file = files
        .iter()
        .find(|f| {
            let p = f.path.to_string_lossy();
            p.ends_with("Test.php") || p.contains("tests/") && p.ends_with(".php")
        })
        .or_else(|| files.iter().find(|f| f.path.to_string_lossy().ends_with(".php")))
        .expect("at least one .php file is emitted");
    test_file.content.clone()
}

/// Rendering the same fixture twice must produce byte-equal PHP source.
#[test]
fn php_accessor_extraction_order_is_deterministic_across_renders() {
    let first = render_once();
    let second = render_once();
    assert_eq!(
        first, second,
        "PHP e2e codegen must be deterministic across renders; got divergent output.\n\
         First render:\n{first}\n\nSecond render:\n{second}"
    );
}

/// The two accessor extraction lines must appear in lexicographic order:
/// `$imports = ...;` before `$structure = ...;`, regardless of the order the
/// matching assertions appear in the fixture.
#[test]
fn php_accessor_extraction_lines_are_sorted_by_field_name() {
    let content = render_once();
    let imports_pos = content
        .find("$imports =")
        .unwrap_or_else(|| panic!("expected `$imports =` accessor extraction line in generated PHP, got:\n{content}"));
    let structure_pos = content.find("$structure =").unwrap_or_else(|| {
        panic!("expected `$structure =` accessor extraction line in generated PHP, got:\n{content}")
    });
    assert!(
        imports_pos < structure_pos,
        "expected `$imports` extraction before `$structure` (lexicographic order) but `$imports` \
         appeared at {imports_pos} and `$structure` at {structure_pos}; full content:\n{content}"
    );
}
