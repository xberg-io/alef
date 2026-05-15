//! Verifies the Zig e2e codegen omits `try` for functions that return a plain
//! value (no error union) such as `language_count() u64`.
//!
//! The Zig backend emits `pub fn language_count() u64` (no error union) when the
//! Rust function is infallible and has no string parameters that require heap
//! allocation. The e2e codegen must not emit `try` in that case, because `try`
//! on a non-error-union type is a compile error in Zig.

use alef_core::config::NewAlefConfig;
use alef_e2e::codegen::E2eCodegen;
use alef_e2e::codegen::zig::ZigE2eCodegen;
use alef_e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("registry".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("language_count".to_string()),
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
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
        source: "registry.json".to_string(),
        http: None,
    }
}

fn make_group() -> FixtureGroup {
    FixtureGroup {
        category: "registry".to_string(),
        fixtures: vec![make_fixture("registry_language_count")],
    }
}

fn render_zig_registry(toml: &str) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group()];
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("registry_test.zig"))
        .expect("registry_test.zig is emitted")
        .content
        .clone()
}

/// `language_count()` returns `u64` (infallible, no error union). The e2e
/// codegen must NOT emit `try` for this call — `try` on a non-error-union
/// expression is a Zig compile error.
#[test]
fn infallible_function_omits_try() {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "tree_sitter_language_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "tree_sitter_language_pack"
result_var = "result"
args = [{ name = "source", field = "source_code", type = "string" }]

[crates.e2e.calls.language_count]
function = "language_count"
module = "tree_sitter_language_pack"
result_var = "result"
result_is_simple = true
args = []

[crates.e2e.calls.language_count.overrides.zig]
returns_result = false
"#;

    let rendered = render_zig_registry(toml);

    assert!(
        !rendered.contains("= try tree_sitter_language_pack.language_count()"),
        "infallible language_count must NOT use `try`. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("language_count()"),
        "language_count call must still be emitted. Rendered:\n{rendered}"
    );
}

/// `has_language(name)` returns `error{{OutOfMemory}}!bool` in Zig (string
/// param requires heap allocation). The e2e codegen MUST emit `try` for it.
#[test]
fn string_param_function_emits_try() {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "tree_sitter_language_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "process"
module = "tree_sitter_language_pack"
result_var = "result"
args = [{ name = "source", field = "source_code", type = "string" }]

[crates.e2e.calls.has_language]
function = "has_language"
module = "tree_sitter_language_pack"
result_var = "result"
result_is_simple = true
args = [{ name = "name", field = "language", type = "string" }]
"#;

    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");

    let fixture = Fixture {
        id: "registry_has_language_true".to_string(),
        category: Some("registry".to_string()),
        description: "has_language returns true for python".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: Some("has_language".to_string()),
        input: serde_json::json!({ "language": "python" }),
        mock_response: None,
        visitor: None,
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
        source: "registry.json".to_string(),
        http: None,
    };

    let groups = vec![FixtureGroup {
        category: "registry".to_string(),
        fixtures: vec![fixture],
    }];
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[])
        .expect("generation succeeds");
    let rendered = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("registry_test.zig"))
        .expect("registry_test.zig is emitted")
        .content
        .clone();

    assert!(
        rendered.contains("try tree_sitter_language_pack.has_language("),
        "has_language with string param must use `try`. Rendered:\n{rendered}"
    );
}
