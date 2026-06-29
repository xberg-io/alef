//! Regression test for the PHP e2e key-renaming pipeline.
//!
//! BLK-7 originally keyed off the core struct's `serde_rename_all` to decide whether
//! to camelCase fixture keys, but that was incorrect: PHP `from_json` deserializes
//! into the BINDING struct (`serde_json::from_str::<Self>`), and the PHP backend always
//! emits binding structs with `#[serde(rename_all = "{php_lang_rename_all}")]` —
//! camelCase by default, regardless of what the core struct carries. The fix in
//! commit 9e8070a8 sources the rename strategy from the PHP language config rather
//! than the core IR. These tests pin that contract: keys are camelCased per the
//! binding's effective `lang_rename_all`, not the core type's `serde_rename_all`.

use alef::core::config::NewAlefConfig;
use alef::core::ir::TypeDef;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php::PhpCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn render_with_type_defs(toml_src: &str, type_defs: Vec<TypeDef>, fixture: Fixture) -> String {
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
        setup: Vec::new(),
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
        args: Vec::new(),
        assertion_recipes: Vec::new(),
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
        is_variant_wrapper: false,
        fields: Vec::new(),
        methods: Vec::new(),
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
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

    let fixture = test_fixture(serde_json::json!({
        "config": {
            "extract_pages": true,
            "insert_page_markers": false,
        }
    }));

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
fn php_camel_cases_keys_when_core_type_lacks_rename_all() {
    // The binding emits `#[serde(rename_all = "camelCase")]` on every struct by
    // default (driven by the PHP backend's `lang_rename_all`), so fixture keys are
    // camelCased regardless of whether the CORE type carries `rename_all`. This
    // pins commit 9e8070a8's contract — the rename source of truth is the BINDING,
    // not the core IR.
    let config_without_camel = TypeDef {
        name: "ConfigWithoutCamel".to_string(),
        rust_path: "test_crate::ConfigWithoutCamel".to_string(),
        original_rust_path: String::new(),
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: "config without rename_all on the core type".to_string(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None, // Absent on the CORE type — but the PHP binding still uses camelCase.
        has_serde: true,
        super_traits: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        fields: Vec::new(),
        methods: Vec::new(),
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
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

    let fixture = test_fixture(serde_json::json!({
        "config": {
            "extract_pages": true,
            "insert_page_markers": false,
        }
    }));

    let output = render_with_type_defs(toml_src, vec![config_without_camel], fixture);

    // camelCase is the PHP binding default — keys are renamed regardless of the
    // core IR's `serde_rename_all`.
    assert!(
        output.contains("extractPages"),
        "camelCase keys should be emitted because the PHP binding default is camelCase\n{output}"
    );
    assert!(
        output.contains("insertPageMarkers"),
        "camelCase keys should be emitted because the PHP binding default is camelCase\n{output}"
    );
}
