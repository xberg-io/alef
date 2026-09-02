//! Nesting coverage for handle-config value rendering.
//!
//! The map that says "this key's object is a binding class" used to be consulted only for a
//! directly nested object, so an object inside a list stayed a bare literal — the shape
//! wasm-bindgen rejects, and the same defect the Python generator carried. These tests pin the
//! four nesting shapes the traversal must enter and, just as importantly, the untyped shapes it
//! must leave alone.

use super::*;

fn class_map(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, class_name)| ((*key).to_string(), (*class_name).to_string()))
        .collect()
}

/// Renders through the real traversal with an empty IR registry, so every class in play is an
/// explicit map entry and the assertions read as the map's own semantics rather than as a
/// by-product of `derive_nested_types_for_wasm`.
fn render(key: &str, value: &serde_json::Value, classes: &[(&str, &str)]) -> String {
    let map = class_map(classes);
    let context = HandleConfigContext {
        nested_types: &map,
        effective_nested_types: &map,
        lang: "wasm",
        enum_fields: &std::collections::HashMap::new(),
        bigint_fields: &std::collections::BTreeSet::new(),
        type_defs: &[],
        enums: &[],
        wasm_type_prefix: "Wasm",
        owner_type: None,
    };
    build_handle_config_value(key, value, &context, &mut std::collections::BTreeSet::new())
}

fn collect(key: &str, value: &serde_json::Value, classes: &[(&str, &str)]) -> std::collections::BTreeSet<String> {
    let map = class_map(classes);
    let context = HandleConfigContext {
        nested_types: &map,
        effective_nested_types: &map,
        lang: "wasm",
        enum_fields: &std::collections::HashMap::new(),
        bigint_fields: &std::collections::BTreeSet::new(),
        type_defs: &[],
        enums: &[],
        wasm_type_prefix: "Wasm",
        owner_type: None,
    };
    let mut used_types = std::collections::BTreeSet::new();
    collect_used_handle_config_types(key, value, &context, &mut used_types);
    used_types
}

fn field(name: &str, ty: TypeRef) -> crate::core::ir::FieldDef {
    crate::core::ir::FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

/// An `EngineConfig` IR type carrying the three field shapes the wasm scalar path must
/// distinguish: an enum-typed field, a `Duration` (bigint on the wasm boundary — see
/// `wasm_bigint_field`) field, and a plain `u32` field.
///
/// `request_timeout` is declared `Duration`, not `TypeRef::Primitive(U64)`, on purpose: that is
/// crawlberg's actual IR shape (`crates/crawlberg/src/types/config.rs`'s `CrawlConfig::
/// request_timeout: Duration`), and it is the shape that first exposed the gap — `Duration`
/// lowers to Rust `u64` at the wasm-bindgen boundary just as directly as a primitive `u64` field
/// does, but was not itself recognised as bigint-typed. ~keep
fn engine_config_type_def() -> TypeDef {
    TypeDef {
        name: "EngineConfig".to_string(),
        fields: vec![
            field("crawl_strategy", TypeRef::Named("CrawlStrategyKind".to_string())),
            field("request_timeout", TypeRef::Duration),
            field(
                "max_body_size",
                TypeRef::Optional(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
            ),
            field("max_depth", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)),
        ],
        ..Default::default()
    }
}

fn crawl_strategy_enum() -> EnumDef {
    EnumDef {
        name: "CrawlStrategyKind".to_string(),
        variants: vec![
            crate::core::ir::EnumVariant {
                name: "Bfs".to_string(),
                ..Default::default()
            },
            crate::core::ir::EnumVariant {
                name: "Dfs".to_string(),
                ..Default::default()
            },
        ],
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    }
}

/// Renders a top-level `EngineConfig` scalar field through the real traversal, with the IR
/// registry populated so `owner_type` resolution can find the field's declared type.
fn render_engine_config_field(
    key: &str,
    value: &serde_json::Value,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    let type_defs = [engine_config_type_def()];
    let enums = [crawl_strategy_enum()];
    let empty_map = std::collections::HashMap::new();
    let empty_set = std::collections::BTreeSet::new();
    let context = HandleConfigContext {
        nested_types: &empty_map,
        effective_nested_types: &empty_map,
        lang: "wasm",
        enum_fields: &empty_map,
        bigint_fields: &empty_set,
        type_defs: &type_defs,
        enums: &enums,
        wasm_type_prefix: "Wasm",
        owner_type: Some("EngineConfig"),
    };
    build_handle_config_value(key, value, &context, referenced_enums)
}

fn built(class_name: &str, body: &str) -> String {
    format!("(() => {{ const _u0 = {class_name}.default(); {body} return _u0; }})()")
}

#[test]
fn should_construct_each_object_inside_a_class_typed_list() {
    let rendered = render(
        "rules",
        &serde_json::json!([{ "deny": true }, { "deny": false }]),
        &[("rules", "WasmRule")],
    );
    assert_eq!(
        rendered,
        format!(
            "[{}, {}]",
            built("WasmRule", "_u0.deny = true;"),
            built("WasmRule", "_u0.deny = false;")
        )
    );
}

#[test]
fn should_leave_list_elements_bare_when_the_key_declares_no_class() {
    let rendered = render(
        "rules",
        &serde_json::json!([{ "deny": true }]),
        &[("policy", "WasmPolicy")],
    );
    assert_eq!(rendered, "[{ deny: true }]");
}

#[test]
fn should_still_construct_a_directly_nested_object() {
    let rendered = render(
        "policy",
        &serde_json::json!({ "deny": true }),
        &[("policy", "WasmPolicy")],
    );
    assert_eq!(rendered, built("WasmPolicy", "_u0.deny = true;"));
}

#[test]
fn should_construct_objects_nested_two_lists_deep() {
    let rendered = render(
        "rules",
        &serde_json::json!([[{ "deny": true }]]),
        &[("rules", "WasmRule")],
    );
    assert_eq!(rendered, format!("[[{}]]", built("WasmRule", "_u0.deny = true;")));
}

#[test]
fn should_construct_a_class_typed_object_inside_an_untyped_object() {
    let rendered = render(
        "extra",
        &serde_json::json!({ "matcher": { "deny": true } }),
        &[("matcher", "WasmMatcher")],
    );
    assert_eq!(
        rendered,
        format!("{{ matcher: {} }}", built("WasmMatcher", "_u0.deny = true;"))
    );
}

#[test]
fn should_camel_case_the_keys_of_an_untyped_object() {
    let rendered = render("extra", &serde_json::json!({ "max_depth": 3 }), &[]);
    assert_eq!(rendered, "{ maxDepth: 3 }");
}

#[test]
fn should_leave_a_scalar_untouched() {
    let rendered = render("max_depth", &serde_json::json!(3), &[("max_depth", "WasmDepth")]);
    assert_eq!(rendered, "3");
}

#[test]
fn collector_should_report_a_class_reached_only_through_a_list() {
    let expected: std::collections::BTreeSet<String> = ["WasmRule".to_string()].into_iter().collect();
    let used = collect(
        "rules",
        &serde_json::json!([{ "deny": true }]),
        &[("rules", "WasmRule")],
    );
    assert_eq!(used, expected);
}

#[test]
fn collector_should_report_nothing_for_an_untyped_value() {
    let used = collect(
        "rules",
        &serde_json::json!([{ "deny": true }]),
        &[("policy", "WasmPolicy")],
    );
    assert!(used.is_empty(), "got: {used:?}");
}

/// End-to-end through the emitter the consumer actually runs: a `handle` arg whose config type
/// carries a list of class-typed entries. Before the traversal landed, this line fell to
/// `json_to_js_camel` and emitted `engineConfig.rules = [{ deny: true }];` — a plain object
/// literal where wasm-bindgen requires a `WasmRule` instance.
#[test]
fn handle_config_list_entries_are_constructed_through_build_args_and_setup() {
    let fixture = crate::e2e::fixture::Fixture {
        id: "crawl".to_string(),
        description: "Crawl".to_string(),
        ..Default::default()
    };
    let input = serde_json::json!({ "config": { "rules": [{ "deny": true }] } });
    let args = [ArgMapping {
        name: "engine".into(),
        field: "input.config".into(),
        arg_type: "handle".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let nested_types = class_map(&[("rules", "WasmRule")]);
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &nested_types,
        "wasm",
        &Default::default(),
        &Default::default(),
        Some("WasmCrawlConfig"),
        &[],
        &[],
        "Wasm",
        &config,
        true,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    let setup = setup_lines.join("\n");
    assert!(
        setup.contains(&format!(
            "engineConfig.rules = [{}];",
            built("WasmRule", "_u0.deny = true;")
        )),
        "list entry must be constructed as its declared binding class: {setup}"
    );
    assert!(
        !setup.contains("engineConfig.rules = [{ deny: true }];"),
        "must not fall back to the untyped json_to_js_camel dump: {setup}"
    );
}

/// A top-level handle-config scalar whose IR field type is a wasm-bindgen C-style enum must
/// render as an `EnumType.Member` reference, not the fixture's raw wire string. Before the
/// `owner_type` seam existed, `engineConfig.crawlStrategy = "dfs"` compiled and ran, but
/// `ToInt32("dfs")` coerces to `0`, which is `WasmCrawlStrategyKind.Bfs` — the test silently
/// exercised the wrong algorithm.
#[test]
fn should_render_a_top_level_enum_scalar_as_a_member_reference() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field("crawl_strategy", &serde_json::json!("dfs"), &mut referenced_enums);
    assert_eq!(rendered, "WasmCrawlStrategyKind.Dfs");
    assert!(
        referenced_enums.contains("WasmCrawlStrategyKind"),
        "enum member reference must register its import: {referenced_enums:?}"
    );
}

/// A top-level handle-config scalar whose IR field type is a wasm `u64`/`i64` must render as a
/// BigInt literal — wasm-bindgen's setter rejects a plain `Number`, throwing `TypeError: Cannot
/// convert 500 to a BigInt` at run time for `engineConfig.requestTimeout = 500;`.
#[test]
fn should_render_a_top_level_bigint_scalar_as_a_bigint_literal() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field("request_timeout", &serde_json::json!(500), &mut referenced_enums);
    assert_eq!(rendered, "500n");
}

/// Zero is exact-integer-shaped too, and must not be special-cased away from the `n` suffix.
#[test]
fn should_render_a_top_level_bigint_scalar_of_zero_as_a_bigint_literal() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field("request_timeout", &serde_json::json!(0), &mut referenced_enums);
    assert_eq!(rendered, "0n");
}

/// A magnitude past `2^53` must survive intact: routing the JSON number through an f64/i64 round
/// trip before appending `n` would already have lost precision by the time the suffix is added.
#[test]
fn should_preserve_bigint_precision_past_the_f64_safe_integer_range() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field(
        "request_timeout",
        &serde_json::json!(9_007_199_254_740_993u64),
        &mut referenced_enums,
    );
    assert_eq!(rendered, "9007199254740993n");
}

/// A `TypeRef::Primitive(U64)` field — the original, narrower bigint case — must still render as
/// a BigInt literal alongside the `Duration` case above.
#[test]
fn should_render_a_top_level_primitive_u64_scalar_as_a_bigint_literal() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field("max_body_size", &serde_json::json!(1024), &mut referenced_enums);
    assert_eq!(rendered, "1024n");
}

/// A plain scalar field (neither enum nor bigint) must keep rendering as an ordinary JS literal —
/// proving the new `owner_type` resolution does not misclassify unrelated fields.
#[test]
fn should_leave_a_plain_scalar_field_unaffected_by_owner_type_resolution() {
    let mut referenced_enums = std::collections::BTreeSet::new();
    let rendered = render_engine_config_field("max_depth", &serde_json::json!(3), &mut referenced_enums);
    assert_eq!(rendered, "3");
    assert!(referenced_enums.is_empty());
}
