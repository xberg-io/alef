use super::DartE2eCodegen;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::{Fixture, FixtureGroup};

const MAX: &str = "18446744073709551615";

fn api() -> ApiSurface {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("lib.rs");
    std::fs::write(&source, include_str!("fixtures/u64_callers.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[&source], "u64-caller", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 5);
    assert_eq!(api.types.len(), 3);
    let echo = api.functions.iter().find(|f| f.name == "echo_u64").unwrap();
    assert_eq!(echo.params[0].ty, TypeRef::Primitive(PrimitiveType::U64));
    api
}

fn emitted_call(api: &ApiSurface, function: &str, value: serde_json::Value, config_arg: bool) -> String {
    let name = if config_arg { "config" } else { "value" };
    let fixture = Fixture {
        id: "u64_caller_literal".into(),
        description: "Exact typed caller literal".into(),
        input: serde_json::json!({(name): value}),
        ..Default::default()
    };
    let mut config = E2eConfig::default();
    config.call.function = function.into();
    config.call.args = vec![
        serde_json::from_value::<ArgMapping>(serde_json::json!({
            "name": name, "field": format!("input.{name}"),
            "type": if config_arg { "json_object" } else { "int" },
            "element_type": if config_arg { Some("ProbeConfig") } else { None }
        }))
        .unwrap(),
    ];
    let files = DartE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "u64_callers".into(),
                fixtures: vec![fixture],
            }],
            &config,
            &ResolvedCrateConfig {
                name: "u64-caller".into(),
                ..Default::default()
            },
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
        .unwrap();
    let selected: Vec<_> = files
        .iter()
        .filter(|f| f.path.ends_with("test/u64_callers_test.dart"))
        .collect();
    assert_eq!(selected.len(), 1, "actual full generator emits one test artifact");
    let source = &selected[0].content;
    let camel = heck::AsLowerCamelCase(function).to_string();
    assert!(
        source.contains(&format!("{camel}(")),
        "real target call missing:\n{source}"
    );
    source.clone()
}

fn assert_plain_call(source: &str, function: &str, value: i32) {
    let camel = heck::AsLowerCamelCase(function);
    let pattern = format!(r"{camel}\(\s*(?:value:\s*)?{value}\s*,?\s*\)");
    assert!(regex::Regex::new(&pattern).unwrap().is_match(source), "{source}");
}

fn leaf(optional: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"wide": u64::MAX, "optional_wide": optional, "signed": -1, "narrow": 17, "word": 23})
}

#[test]
fn should_lower_ir_typed_u64_scalar_values_to_bigint_decimal_constructors() {
    let api = api();
    for value in [0_u64, 9_007_199_254_740_993, 9_223_372_036_854_775_808] {
        let source = emitted_call(&api, "echo_u64", value.into(), false);
        assert!(source.contains(&format!("BigInt.parse('{value}')")), "{source}");
    }
}

#[test]
fn should_lower_u64_max_without_emitting_an_overflowing_dart_int_literal() {
    let source = emitted_call(&api(), "echo_u64", u64::MAX.into(), false);
    assert!(source.contains(&format!("BigInt.parse('{MAX}')")), "{source}");
    assert!(!source.contains(&format!("echoU64({MAX}")));
    assert!(!source.contains(&format!("value: {MAX}")));
}

#[test]
fn should_lower_recursive_native_dto_u64_fields_using_extracted_types() {
    let api = api();
    let item = leaf(u64::MAX.into());
    let value = serde_json::json!({"leaf": item.clone(), "optional": item.clone(), "leaves": [item]});
    let source = super::values::render_native_dart_dto("Envelope", &value, &api.types, &[]).unwrap();
    assert_eq!(source.matches(&format!("BigInt.parse('{MAX}')")).count(), 6, "{source}");
    assert_eq!(source.matches("Leaf(").count(), 3);
    for untouched in ["signed: -1", "narrow: 17", "word: 23"] {
        assert_eq!(source.matches(untouched).count(), 3, "{source}");
    }
}

#[test]
fn should_preserve_null_optional_u64_while_lowering_present_native_fields() {
    let source =
        super::values::render_native_dart_dto("Leaf", &leaf(serde_json::Value::Null), &api().types, &[]).unwrap();
    assert!(source.contains("optionalWide: null"), "{source}");
    assert!(source.contains(&format!("wide: BigInt.parse('{MAX}')")), "{source}");
}

#[test]
fn should_keep_signed_u32_and_usize_scalar_callers_as_dart_ints() {
    let api = api();
    for (function, value) in [("echo_i64", -1), ("echo_u32", 17), ("echo_usize", 23)] {
        let source = emitted_call(&api, function, value.into(), false);
        assert!(!source.contains("BigInt."), "{source}");
        assert_plain_call(&source, function, value);
    }
}

#[test]
fn should_keep_rust_json_constructor_payload_as_exact_numeric_json() {
    let source = emitted_call(&api(), "configure", serde_json::json!({"limit": u64::MAX}), true);
    assert!(
        source.contains(&format!("createProbeConfigFromJson(json: '{{\"limit\":{MAX}}}')")),
        "{source}"
    );
    assert!(!source.contains("BigInt."), "{source}");
}

#[test]
fn should_keep_unresolved_integer_callers_unchanged_instead_of_guessing_u64() {
    let mut api = api();
    api.functions.clear();
    let source = emitted_call(&api, "echo_u64", 17.into(), false);
    assert!(!source.contains("BigInt."), "{source}");
    assert_plain_call(&source, "echo_u64", 17);
}
