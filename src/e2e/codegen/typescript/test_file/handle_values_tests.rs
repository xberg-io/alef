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
    };
    let mut used_types = std::collections::BTreeSet::new();
    collect_used_handle_config_types(key, value, &context, &mut used_types);
    used_types
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
