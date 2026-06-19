//! Verifies the Elixir e2e codegen emits Rustler NifTaggedEnum tuples for tagged-enum array args.
//!
//! Internally-tagged enums (with `#[serde(tag = "type")]`) require special tuple formatting
//! for Rustler's `NifTaggedEnum` decoder, which expects either `:atom` (unit variants) or
//! `{:atom, %{field: val}}` (struct variants), where field atoms use Rust field names,
//! not serde wire names.

use alef::core::config::NewAlefConfig;
use alef::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::elixir::ElixirCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn build_config_with_args(args_toml: &str) -> (alef::e2e::config::E2eConfig, alef::core::config::ResolvedCrateConfig) {
    let toml_src = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "interact"
module = "MyLib"
result_var = "result"
returns_result = true
args = [
  {args_toml}
]
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().expect("resolves").remove(0);
    (e2e, resolved)
}

fn fixture_with_input(input: serde_json::Value) -> FixtureGroup {
    FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![Fixture {
            id: "test_fixture".to_string(),
            category: Some("test".to_string()),
            description: "test fixture".to_string(),
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
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some("output".to_string()),
                value: None,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            source: "test/test_fixture.json".to_string(),
            http: None,
        }],
    }
}

/// Build synthetic enums for testing.
fn build_test_enums() -> Vec<EnumDef> {
    // ScrollDirection unit-only enum with snake_case rename_all
    let scroll_direction = EnumDef {
        name: "ScrollDirection".to_string(),
        rust_path: "my_crate::ScrollDirection".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Up".to_string(),
                fields: Vec::new(),
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Down".to_string(),
                fields: Vec::new(),
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: true,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // MyAction tagged enum with camelCase rename_all
    let my_action = EnumDef {
        name: "MyAction".to_string(),
        rust_path: "my_crate::MyAction".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            // Click { selector: String }
            EnumVariant {
                name: "Click".to_string(),
                fields: vec![FieldDef {
                    name: "selector".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: Default::default(),
                    vec_inner_core_wrapper: Default::default(),
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            // Scroll { direction: ScrollDirection }
            EnumVariant {
                name: "Scroll".to_string(),
                fields: vec![FieldDef {
                    name: "direction".to_string(),
                    ty: TypeRef::Named("ScrollDirection".to_string()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: Default::default(),
                    vec_inner_core_wrapper: Default::default(),
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            // Scrape (unit variant)
            EnumVariant {
                name: "Scrape".to_string(),
                fields: Vec::new(),
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: Some("camelCase".to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    vec![scroll_direction, my_action]
}

/// Verify that a tagged-enum array arg emits Rustler tuple format.
/// Input: [{"type": "click", "selector": "..."}, {"type": "scroll", "direction": "down"}, {"type": "scrape"}]
/// Expected: [{:click, %{selector: "..."}}, {:scroll, %{direction: :down}}, :scrape]
#[test]
fn tagged_enum_array_emits_rustler_tuples() {
    let args_toml = r#"
  { name = "engine", field = "input.engine", type = "handle" },
  { name = "url", field = "input.url", type = "string" },
  { name = "actions", field = "input.actions", type = "json_object", element_type = "MyAction" }
"#;
    let (e2e, resolved) = build_config_with_args(args_toml);
    let input = serde_json::json!({
        "engine": {},
        "url": "http://example.com",
        "actions": [
            {"type": "click", "selector": "#button"},
            {"type": "scroll", "direction": "down"},
            {"type": "scrape"}
        ]
    });
    let groups = vec![fixture_with_input(input)];
    let enums = build_test_enums();

    let files = ElixirCodegen
        .generate(&groups, &e2e, &resolved, &[], &enums)
        .expect("generation succeeds");

    let test_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_test.exs"))
        .expect("Elixir test file is emitted");

    let body = &test_file.content;

    // Verify that the action list contains Rustler tuples, not raw maps.
    // Verify the interact call exists and contains the actions arg.
    let _interact_call = body
        .lines()
        .find(|line| line.contains("MyLib.interact"))
        .expect("interact call found");

    // Unit variant: :scrape
    assert!(
        body.contains(":scrape"),
        "unit variant should emit as atom :scrape, got:\n{body}"
    );

    // Click variant with field: {:click, %{selector: "..."}}
    assert!(
        body.contains(":click") && body.contains("selector:"),
        "click variant should emit tuple with selector field, got:\n{body}"
    );

    // Scroll variant with nested unit enum field: {:scroll, %{direction: :down}}
    assert!(
        body.contains(":scroll") && body.contains("direction: :down"),
        "scroll variant should emit tuple with direction as atom, got:\n{body}"
    );

    // Verify the overall shape: the actions are in a list [...].
    // A simple heuristic: count occurrences of leading colons in the actions context.
    let action_count =
        body.matches(":click").count() + body.matches(":scroll").count() + body.matches(":scrape").count();
    assert!(
        action_count >= 3,
        "should find at least 3 variant atoms (:click, :scroll, :scrape), found {action_count}"
    );
}
