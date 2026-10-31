use crate::core::config::{ResolvedCrateConfig, e2e::CallOverride};
use crate::core::ir::ApiSurface;
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

const VALUES: [u64; 5] = [
    0,
    9_007_199_254_740_993,
    i64::MAX as u64,
    (i64::MAX as u64) + 1,
    u64::MAX,
];

fn statements(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    function: &str,
    value: serde_json::Value,
    kind: &str,
) -> String {
    let name = if kind == "nested" {
        "req"
    } else if kind == "json" {
        "config"
    } else {
        "value"
    };
    let fixture = Fixture {
        id: "emitted_native_call".into(),
        description: "Actual emitted native caller".into(),
        input: serde_json::json!({(name): value}),
        ..Default::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = function.into();
    e2e.call.args = vec![serde_json::from_value::<ArgMapping>(serde_json::json!({"name":name,"field":format!("input.{name}"),"type":if matches!(kind,"nested"|"json") {"json_object"} else {"int"},"element_type":if kind=="json" {Some("ProbeConfig")} else {None}})).unwrap()];
    if kind == "nested" {
        e2e.call.overrides.insert(
            "dart".into(),
            CallOverride {
                class: Some("raw".into()),
                options_type: Some("Envelope".into()),
                options_via: Some("from_json".into()),
                ..Default::default()
            },
        );
    }
    let first_class = crate::e2e::codegen::dart::values::build_dart_first_class_map(&api.types, &api.enums, &e2e);
    let mut rendered = String::new();
    let bridge_class = config.dart_bridge_class_name();
    crate::e2e::codegen::dart::test_case::render_test_case(
        &mut rendered,
        &fixture,
        crate::e2e::codegen::dart::test_case::DartTestCaseContext {
            e2e_config: &e2e,
            lang: "dart",
            bridge_class: &bridge_class,
            dart_first_class_map: &first_class,
            adapters: &[],
            config,
            type_defs: &api.types,
            enums: &api.enums,
            functions: &api.functions,
            errors: &api.errors,
            native_typed_dtos: kind == "nested",
            is_snippet: kind == "nested",
        },
    );
    let lines = super::super::extract_test_statements(&rendered).expect("actual emitted E2E/snippet statements");
    let source = lines.join("\n");
    let function = heck::AsLowerCamelCase(function).to_string();
    assert_eq!(source.matches(&format!(".{function}(")).count(), 1, "{source}");
    if kind == "nested" {
        assert!(
            source.contains("Envelope(") && source.contains("Leaf("),
            "must use actual native DTO snippet path: {source}"
        );
    }
    source
}

fn envelope(value: u64, absent: bool) -> serde_json::Value {
    let leaf = serde_json::json!({"wide":value,"optional_wide":if absent {None} else {Some(value)},"signed":-1,"narrow":17,"word":23});
    serde_json::json!({"leaf":leaf.clone(),"optional":if absent {serde_json::Value::Null} else {leaf.clone()},"leaves":[leaf]})
}

fn case(id: &str, kind: &str, body: &str, check: &str) -> String {
    format!("{{\n{body}\n{check}\nreceipt('{id}', '{kind}');\n}}\n")
}

pub(super) fn programs(api: &ApiSurface, config: &ResolvedCrateConfig, public_library: &str) -> (String, String) {
    let mut cases = Vec::new();
    for (index, value) in VALUES.iter().enumerate() {
        let call = statements(api, config, "echo_u64", (*value).into(), "scalar");
        assert!(call.contains(&format!("BigInt.parse('{value}')")), "{call}");
        cases.push(case(
            &format!("scalar_{index}"),
            "scalar",
            &call,
            &format!("require(result is BigInt && result.toString() == '{value}', 'scalar {index} native value');"),
        ));
        let call = statements(api, config, "echo_envelope", envelope(*value, false), "nested");
        assert_eq!(call.matches(&format!("BigInt.parse('{value}')")).count(), 6, "{call}");
        cases.push(case(
            &format!("nested_{index}"),
            "nested",
            &call,
            &format!("checkEnvelope(result, '{value}', false);"),
        ));
    }
    let call = statements(api, config, "echo_envelope", envelope(u64::MAX, true), "nested");
    cases.push(case(
        "optional_null",
        "optional_null",
        &call,
        "checkEnvelope(result, '18446744073709551615', true);",
    ));
    for (function, kind, value) in [
        ("echo_i64", "signed", -1),
        ("echo_u32", "u32", 17),
        ("echo_usize", "usize", 23),
    ] {
        let call = statements(api, config, function, value.into(), kind);
        assert!(!call.contains("BigInt."), "{call}");
        cases.push(case(
            kind,
            kind,
            &call,
            &format!("require(result is int && result == {value}, '{kind} native value');"),
        ));
    }
    let call = statements(api, config, "configure", serde_json::json!({"limit":u64::MAX}), "json");
    assert!(
        call.contains("json: '{\"limit\":18446744073709551615}'") && !call.contains("BigInt."),
        "{call}"
    );
    cases.push(case(
        "json",
        "json",
        &call,
        "require(result is BigInt && result.toString() == '18446744073709551615', 'Rust JSON numeric maximum');",
    ));
    assert_eq!(cases.len(), 15);
    let template = include_str!("program.dart.jinja");
    assert_eq!(template.matches("@@PUBLIC_LIBRARY@@").count(), 1);
    let template = template.replace("@@PUBLIC_LIBRARY@@", public_library);
    assert_eq!(template.matches("@@CASES@@").count(), 1);
    let positive = template.replace("@@CASES@@", &cases.join("\n"));
    let poison = statements(api, config, "echo_u64", 17.into(), "scalar");
    assert_eq!(poison.matches("BigInt.parse('17')").count(), 1);
    let poison = poison.replacen("BigInt.parse('17')", "17", 1);
    let poisoned = template.replace(
        "@@CASES@@",
        &format!("{poison}\nrequire(result == 17, 'unreachable poisoned caller');"),
    );
    eprintln!("caller_emissions_positive=15 caller_emissions_negative=1 caller_nested_native=6 caller_full_e2e=9");
    (positive, poisoned)
}
