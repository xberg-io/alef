//! Regression test for BLK-7: PHP e2e codegen incorrectly camelCases fixture keys
//! when the receiving Rust core struct does NOT have `#[serde(rename_all = "camelCase")]`.
//!
//! The bug was in `json_to_php_camel_keys()`, which blindly converted all keys to
//! camelCase regardless of the core struct's actual serde config. This caused
//! `PageConfig` (which lacks `rename_all = "camelCase"`) to receive keys like
//! `extract_pages: true` transformed to `extractPages: true`, silently falling back
//! to default values (`extract_pages: false`) at serde deserialization time.
//!
//! The fix checks the IR's `serde_rename_all` field for each type and only applies
//! camelCase transformation when it equals `Some("camelCase")`.

use alef::core::ir::TypeDef;
use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup, Assertion};

fn render_with_type_defs(
    toml_src: &str,
    type_defs: Vec<TypeDef>,
    fixture: Fixture,
) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![fixture],
    }];
    let files = PhpCodegen
        .generate(&groups, &e2e, &resolved, &type_defs, &[])
        .expect("PHP codegen succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Test.php"))
        .expect("a *Test.php file is emitted")
        .content
        .clone()
}

fn test_fixture(input: serde_json::Value) -> Fixture {
    Fixture {
        id: "test_case".to_string(),
        category: Some("test".to_string()),
        description: "test case".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        call: None,
        input,
        assertions: vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("result".to_string()),
            value: Some(serde_json::Value::String("expected_value".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        mock_response: None,
        visitor: None,
        source: String::new(),
        http: None,
    }
}

#[test]
fn php_respects_serde_rename_all_camel_case_when_present() {
    // Create a type WITH `rename_all = "camelCase"`.
    let config_with_camel = TypeDef {
        name: "ConfigWithCamel".to_string(),
        rust_path: "test_crate::ConfigWithCamel".to_string(),
        original_rust_path: String::new(),
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "config with camelCase".to_string(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: Some("camelCase".to_string()), // Present!
        has_serde: true,
        super_traits: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        fields: Vec::new(),
        methods: Vec::new(),
    };

    let toml_src = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test_crate"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
result_var = "result"
args = [{ name = "config", field = "config", type = "json_object" }]

[crates.e2e.call.overrides.php]
options_type = "ConfigWithCamel"
"#;

    let fixture = test_fixture(
        serde_json::json!({
            "config": {
                "extract_pages": true,
                "insert_page_markers": false,
            }
        }),
    );

    let output = render_with_type_defs(toml_src, vec![config_with_camel], fixture);

    // When serde_rename_all = "camelCase", keys SHOULD be transformed.
    assert!(
        output.contains("extractPages"),
        "camelCase keys should be emitted when type has rename_all = \"camelCase\""
    );
    assert!(
        output.contains("insertPageMarkers"),
        "camelCase keys should be emitted when type has rename_all = \"camelCase\""
    );
}

#[test]
fn php_preserves_snake_case_when_serde_rename_all_absent() {
    // Create a type WITHOUT `rename_all` setting (should use snake_case).
    let config_without_camel = TypeDef {
        name: "ConfigWithoutCamel".to_string(),
        rust_path: "test_crate::ConfigWithoutCamel".to_string(),
        original_rust_path: String::new(),
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "config without rename_all".to_string(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None, // Absent!
        has_serde: true,
        super_traits: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        fields: Vec::new(),
        methods: Vec::new(),
    };

    let toml_src = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test_crate"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract"
result_var = "result"
args = [{ name = "config", field = "config", type = "json_object" }]

[crates.e2e.call.overrides.php]
options_type = "ConfigWithoutCamel"
"#;

    let fixture = test_fixture(
        serde_json::json!({
            "config": {
                "extract_pages": true,
                "insert_page_markers": false,
            }
        }),
    );

    let output = render_with_type_defs(toml_src, vec![config_without_camel], fixture);

    // When serde_rename_all is absent, keys should NOT be transformed (stay snake_case).
    assert!(
        output.contains("extract_pages"),
        "snake_case keys should be preserved when type lacks rename_all setting"
    );
    assert!(
        output.contains("insert_page_markers"),
        "snake_case keys should be preserved when type lacks rename_all setting"
    );
    // Ensure the camelCase versions are NOT present.
    assert!(
        !output.contains("extractPages"),
        "camelCase keys should NOT be emitted when type lacks rename_all setting"
    );
    assert!(
        !output.contains("insertPageMarkers"),
        "camelCase keys should NOT be emitted when type lacks rename_all setting"
    );
}
