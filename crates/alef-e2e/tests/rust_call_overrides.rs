//! Verifies the Rust e2e codegen honours `wrap_options_in_some` and `extra_args`
//! overrides from `[e2e.call.overrides.rust]`.
//!
//! These are needed for fallible signatures whose options slot is owned `Option<T>`
//! (rather than borrowed `&T`) and which take additional trailing positional args
//! the fixture cannot supply (e.g. `convert(html, options, visitor) -> Result<…>`).
//!
//! Without them the generator emits `&options` against an `Option<T>` slot, omits
//! the trailing arg, and produces uncompilable output (E0061, E0308, E0609).

use alef_core::config::e2e::E2eConfig;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::rust::RustE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn resolve_one(cfg: &NewAlefConfig) -> (ResolvedCrateConfig, E2eConfig) {
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    (resolved, e2e)
}

// ---------------------------------------------------------------------------
// Bug regression helpers
// ---------------------------------------------------------------------------

fn build_config_with_optional_array_fields(extra_call_override: &str) -> NewAlefConfig {
    let toml_src = format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_optional = ["metadata.sheet_count", "metadata.output_format", "detected_languages"]
fields_array = ["detected_languages"]

[crates.e2e.call]
function = "extract_file"
module = "mylib"
result_var = "result"
async = true
returns_result = true
args = [
  {{ name = "path", field = "input.path", type = "string" }},
]

[crates.e2e.call.overrides.rust]
crate_name = "mylib"
function = "extract_file"
{extra_call_override}
"#
    );
    toml::from_str(&toml_src).expect("config parses")
}

fn build_fixture_with_assertions(id: &str, assertions: Vec<Assertion>) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: id.to_string(),
            category: Some("smoke".to_string()),
            description: "regression test fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({ "path": "test.pdf" }),
            mock_response: Some(alef_e2e::fixture::MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: std::collections::HashMap::new(),
            }),
            visitor: None,
            assertions,
            source: "test.json".to_string(),
            http: None,
        }],
    }
}

fn render_smoke_test(cfg: &NewAlefConfig, assertions: Vec<Assertion>) -> String {
    let (resolved, e2e) = resolve_one(cfg);
    let groups = vec![build_fixture_with_assertions("bug_regression", assertions)];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    test_file.content.clone()
}

fn build_config(extra_call_override: &str) -> NewAlefConfig {
    let toml_src = format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "html-to-markdown-rs"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "convert"
module = "html_to_markdown_rs"
args = [
  {{ name = "html", field = "html", type = "string" }},
  {{ name = "options", field = "options", type = "json_object", optional = true }},
]

[crates.e2e.call.overrides.rust]
crate_name = "html_to_markdown_rs"
function = "convert"
{extra_call_override}
"#
    );
    toml::from_str(&toml_src).expect("config parses")
}

fn build_fixture() -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "smoke_basic".to_string(),
            category: Some("smoke".to_string()),
            description: "basic conversion".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({
                "html": "<p>hi</p>",
                "options": { "headingStyle": "atx" },
            }),
            mock_response: Some(alef_e2e::fixture::MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: std::collections::HashMap::new(),
            }),
            visitor: None,
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("content".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
            }],
            source: "test.json".to_string(),
            http: None,
        }],
    }
}

fn render_rust_test(cfg: &NewAlefConfig) -> String {
    let (resolved, e2e) = resolve_one(cfg);
    let groups = vec![build_fixture()];
    let files = RustE2eCodegen
        .generate(&groups, &e2e, &resolved)
        .expect("generation succeeds");
    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.rs"))
        .expect("smoke_test.rs is emitted");
    test_file.content.clone()
}

#[test]
fn default_options_pass_by_reference() {
    // Without wrap_options_in_some, json_object args render as `&options`.
    let config = build_config("");
    let rendered = render_rust_test(&config);
    assert!(
        rendered.contains("convert(html, &options)"),
        "default rust override should pass json_object args by reference. Rendered:\n{rendered}"
    );
}

#[test]
fn wrap_options_in_some_emits_some_clone() {
    // With wrap_options_in_some = true, the json_object expression is wrapped
    // in `Some(...).clone()` so it matches owned `Option<T>` parameter slots.
    let config = build_config("wrap_options_in_some = true");
    let rendered = render_rust_test(&config);
    assert!(
        rendered.contains("Some(options.clone())"),
        "wrap_options_in_some should emit `Some(options.clone())`. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("convert(html, &options"),
        "wrap_options_in_some must not emit the default `&options` form. Rendered:\n{rendered}"
    );
}

#[test]
fn extra_args_are_appended_after_configured_args() {
    // extra_args = ["None"] must be emitted verbatim after html and options,
    // matching e.g. `convert(html, options, visitor) -> Result<…>`.
    let config = build_config(r#"extra_args = ["None"]"#);
    let rendered = render_rust_test(&config);
    assert!(
        rendered.contains(", None)"),
        "extra_args entry `None` must be appended as a trailing positional arg. Rendered:\n{rendered}"
    );
}

#[test]
fn wrap_options_in_some_combined_with_extra_args_and_returns_result() {
    // The full html-to-markdown shape: owned options slot, trailing visitor slot,
    // and a fallible return that triggers `.expect("should succeed")`.
    let config = build_config(
        r#"
wrap_options_in_some = true
extra_args = ["None"]
returns_result = true
"#,
    );
    let rendered = render_rust_test(&config);
    assert!(
        rendered.contains("convert(html, Some(options.clone()), None)"),
        "combined overrides should emit the full 3-arg call shape. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains(".expect(\"should succeed\")"),
        "returns_result = true must emit the `.expect(...)` unwrap. Rendered:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug A regression: Option<Vec<String>> unwrap uses &[] not ""
// ---------------------------------------------------------------------------

#[test]
fn bug_a_optional_vec_string_unwrap_fallback_is_empty_slice() {
    // `detected_languages` is both fields_optional and fields_array (Option<Vec<String>>).
    // The unwrap binding must emit `.as_deref().unwrap_or(&[])`, not `.unwrap_or("")`,
    // because `as_deref()` on Option<Vec<T>> yields Option<&[T]>, not Option<&str>.
    let config = build_config_with_optional_array_fields("");
    let assertions = vec![Assertion {
        assertion_type: "contains".to_string(),
        field: Some("detected_languages".to_string()),
        value: Some(serde_json::Value::String("eng".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
    }];
    let rendered = render_smoke_test(&config, assertions);
    assert!(
        rendered.contains("unwrap_or(&[])"),
        "Option<Vec<String>> binding must use unwrap_or(&[]), not unwrap_or(\"\")\nRendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("unwrap_or(\"\")"),
        "Option<Vec<String>> binding must not use unwrap_or(\"\")\nRendered:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug B regression: Option<usize> comparison wraps with unwrap_or(0)
// ---------------------------------------------------------------------------

#[test]
fn bug_b_optional_numeric_greater_than_or_equal_wraps_unwrap_or() {
    // `metadata.sheet_count` is fields_optional but not fields_array (Option<usize>).
    // A `greater_than_or_equal` assertion must emit `.unwrap_or(0) >= N` not bare `>= N`,
    // because comparing Option<usize> >= N directly is a type error.
    let config = build_config_with_optional_array_fields("");
    let assertions = vec![Assertion {
        assertion_type: "greater_than_or_equal".to_string(),
        field: Some("metadata.sheet_count".to_string()),
        value: Some(serde_json::Value::Number(2.into())),
        values: None,
        method: None,
        check: None,
        args: None,
    }];
    let rendered = render_smoke_test(&config, assertions);
    assert!(
        rendered.contains("unwrap_or(0) >= 2"),
        "Option<usize> >= N must emit .unwrap_or(0) >= N\nRendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("sheet_count >= 2"),
        "bare Option<usize> >= N comparison must not be emitted\nRendered:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug C regression: Option<String> in equals uses .as_deref().unwrap_or("").trim()
// ---------------------------------------------------------------------------

#[test]
fn bug_c_optional_string_equals_in_vec_result_uses_as_deref_unwrap_or() {
    // When the outer result is Vec<T> (result_is_vec), the call-site unwrap pass is
    // skipped. An `equals` assertion on an optional string field inside the per-element
    // `for r in &result` loop must emit `.as_deref().unwrap_or("").trim()` rather than
    // `.trim()` directly on Option<String> (which would be E0599).
    let config = build_config_with_optional_array_fields("result_is_vec = true");
    let assertions = vec![Assertion {
        assertion_type: "equals".to_string(),
        field: Some("metadata.output_format".to_string()),
        value: Some(serde_json::Value::String("markdown".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
    }];
    let rendered = render_smoke_test(&config, assertions);
    assert!(
        rendered.contains("as_deref().unwrap_or(\"\").trim()"),
        "Optional<String> equals in vec loop must use .as_deref().unwrap_or(\"\").trim()\nRendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("output_format.trim()"),
        "bare .trim() on Option<String> must not be emitted\nRendered:\n{rendered}"
    );
}

// ---------------------------------------------------------------------------
// Bug D regression: field: "result" (sentinel) refers to the whole result var
// ---------------------------------------------------------------------------

#[test]
fn bug_d_field_named_result_refers_to_whole_result_not_struct_field() {
    // When a fixture uses `field: "result"` and the function is NOT result_is_simple,
    // the codegen must NOT emit `result.result` (treating "result" as a struct field).
    // Instead it should emit assertions directly against the `result` variable.
    let config = build_config_with_optional_array_fields("");
    let assertions = vec![Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("result".to_string()),
        value: None,
        values: None,
        method: None,
        check: None,
        args: None,
    }];
    let rendered = render_smoke_test(&config, assertions);
    assert!(
        !rendered.contains("result.result"),
        "field: \"result\" must not emit result.result — should refer to the whole result var\nRendered:\n{rendered}"
    );
}
